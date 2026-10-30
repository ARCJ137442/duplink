//! duplink —— 重复文件检测与硬链接合并。
//!
//! 📌【2026-10-30】本文件是薄编排层：按"扫描 → 摘要 → 分组 → 报告 → 确认 → 转换 → 复核"
//! 的固定顺序调用各阶段模块，不包含算法细节。所有破坏性副作用集中在 `link` 模块，
//! 所有人类可读输出集中在 `report` 模块。

mod cli;
mod confirm;
mod digest;
mod group;
mod link;
mod model;
mod platform;
mod report;
mod scan;

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use cli::Args;
use model::human_size;

/// 部分处理失败时的退出码：区分"整体失败"与"完成但有例外"。
const EXIT_PARTIAL: u8 = 2;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{} {error:#}", style("错误：").red().bold());
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let args = Args::parse();
    let root = resolve_root(args.path.as_deref())?;
    if !root.is_dir() {
        anyhow::bail!("扫描根目录不存在或不是目录: {}", root.display());
    }

    // JSON 模式下，人类可读信息走 stderr，stdout 只留 JSON，方便管道消费。
    let mut human: Box<dyn Write> = if args.json {
        Box::new(io::stderr())
    } else {
        Box::new(io::stdout())
    };

    let min_size = args.min_size.unwrap_or(0);
    let excludes = scan::ExcludeMatcher::new(&args.exclude)?;
    let jobs = args.jobs.unwrap_or_else(default_jobs);

    report::print_header(
        &mut human,
        &root,
        args.min_size,
        &args.exclude,
        args.dry_run,
    )?;

    // ── 阶段一：扫描 ────────────────────────────────────────────
    let spinner = ProgressBar::new_spinner();
    spinner.enable_steady_tick(Duration::from_millis(120));
    spinner.set_message("正在遍历目录…");
    let mut outcome = scan::collect_files(
        &root,
        &scan::ScanConfig {
            min_size,
            excludes,
            max_depth: args.max_depth,
        },
    );
    spinner.finish_and_clear();

    let total_bytes: u64 = outcome.entries.iter().map(|entry| entry.size).sum();
    writeln!(
        human,
        "{} {} 个文件 · {}",
        style("[1/4] 扫描").bold(),
        outcome.entries.len(),
        human_size(total_bytes)
    )?;
    if !outcome.skipped.is_empty() {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, reason) in &outcome.skipped {
            *counts.entry(reason.label()).or_insert(0) += 1;
        }
        let detail = counts
            .iter()
            .map(|(label, count)| format!("{label} {count}"))
            .collect::<Vec<_>>()
            .join(" · ");
        writeln!(human, "      {}", style(format!("跳过 {detail}")).dim())?;
    }
    if outcome.unreadable_dirs > 0 {
        writeln!(
            human,
            "      {}",
            style(format!("{} 个目录不可读", outcome.unreadable_dirs)).yellow()
        )?;
    }

    // ── 阶段二：筛选候选 ────────────────────────────────────────
    let candidates = group::candidate_indices(&outcome.entries);
    writeln!(
        human,
        "{} {} 个候选（尺寸与其它文件相同）",
        style("[2/4] 筛选").bold(),
        candidates.len()
    )?;

    // ── 阶段三：计算内容摘要 ────────────────────────────────────
    let bar = ProgressBar::new(candidates.len() as u64);
    bar.set_style(
        ProgressStyle::with_template("{spinner} {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    bar.set_message("正在计算 SHA-256…");
    let digest_outcome = digest::fill_digests(&mut outcome.entries, &candidates, jobs, &bar);
    bar.finish_and_clear();
    let failed_note = if digest_outcome.failed.is_empty() {
        String::new()
    } else {
        format!("，{} 个读取失败", digest_outcome.failed.len())
    };
    writeln!(
        human,
        "{} {} 个文件已计算摘要{failed_note}",
        style("[3/4] 摘要").bold(),
        digest_outcome.hashed
    )?;
    if !digest_outcome.failed.is_empty() {
        writeln!(
            human,
            "      {}",
            style(format!(
                "{} 个文件读取失败，未参与比对：",
                digest_outcome.failed.len()
            ))
            .yellow()
        )?;
        for (path, reason) in digest_outcome.failed.iter().take(5) {
            writeln!(
                human,
                "        {} {}",
                style("·").dim(),
                style(report::short_path(&root, path)).dim()
            )?;
            writeln!(human, "          {}", style(reason).dim())?;
        }
        if digest_outcome.failed.len() > 5 {
            writeln!(
                human,
                "        {}",
                style(format!("… 另有 {} 个", digest_outcome.failed.len() - 5)).dim()
            )?;
        }
    }

    // ── 阶段四：聚合重复组 ──────────────────────────────────────
    let mut groups = group::build_groups(&outcome.entries);
    // 身份比较放在分组之后：只为重复组成员打开句柄，而不是目录中的每个文件。
    group::partition_by_identity(&mut groups);
    writeln!(
        human,
        "{} {} 组重复",
        style("[4/4] 聚合").bold(),
        groups.len()
    )?;
    writeln!(human)?;

    report::print_groups(&mut human, &groups, &root, args.max_listed)?;
    report::print_summary(&mut human, &groups)?;
    report::print_editable_warning(&mut human, &groups)?;

    if groups.is_empty() {
        writeln!(human, "{}", style("没有发现重复文件。").green())?;
        if args.json {
            emit_json(
                &args,
                &root,
                &groups,
                &outcome,
                candidates.len(),
                &digest_outcome.failed,
                None,
            )?;
        }
        return Ok(ExitCode::SUCCESS);
    }

    // ── 阶段五：用户确认 ────────────────────────────────────────
    let selection = if args.yes {
        confirm::Selection::All
    } else {
        confirm::ask(&mut human, &mut io::stdin().lock(), &groups).map_err(anyhow::Error::msg)?
    };
    let indices = confirm::resolve(&selection, &groups);
    if indices.is_empty() {
        writeln!(human, "{}", style("未选择任何重复组，未做改动。").yellow())?;
        if args.json {
            emit_json(
                &args,
                &root,
                &groups,
                &outcome,
                candidates.len(),
                &digest_outcome.failed,
                None,
            )?;
        }
        return Ok(ExitCode::SUCCESS);
    }

    // ── 阶段六：转换为硬链接 ────────────────────────────────────
    let bar = ProgressBar::new(indices.len() as u64);
    bar.set_style(
        ProgressStyle::with_template("{spinner} {pos}/{len} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    bar.set_message("正在建立硬链接…");

    let mut linked = 0usize;
    let mut already = 0usize;
    let mut reclaimed = 0u64;
    let mut failures: Vec<report::FailureReport> = Vec::new();

    for &index in &indices {
        let group = &groups[index];
        let outcome = link::link_group(group, index, args.dry_run);
        linked += outcome.linked_count();
        already += outcome.already_linked_count();
        reclaimed += outcome.reclaimed();
        for (path, result) in outcome.failures() {
            if let link::LinkOutcome::Failed { reason } = result {
                failures.push(report::FailureReport {
                    group: outcome.group_index + 1,
                    path: path.display().to_string(),
                    reason: reason.clone(),
                });
            }
        }
        bar.inc(1);
    }
    bar.finish_and_clear();

    // ── 阶段七：复核 ────────────────────────────────────────────
    // 转换成功的路径必须与主副本共享同一文件身份，否则不算真正完成。
    let verified = verify_shared_identity(&groups, &indices);

    writeln!(human, "{}", style("═".repeat(64)).dim())?;
    if args.dry_run {
        writeln!(human, "{}", style("[预览模式] 未做任何改动").yellow())?;
        writeln!(
            human,
            "  将转换 {} 份副本 · 可回收 {}",
            linked,
            human_size(reclaimed)
        )?;
    } else {
        writeln!(human, "{}", style("完成").bold())?;
        writeln!(human, "  {} 转换 {} 份副本", style("✓").green(), linked)?;
        writeln!(
            human,
            "  {} 复核通过 {verified}/{linked}（与主副本共享同一文件身份）",
            style("✓").green()
        )?;
        writeln!(
            human,
            "  {} 回收 {}",
            style("✓").green(),
            style(human_size(reclaimed)).bold().green()
        )?;
        if already > 0 {
            writeln!(
                human,
                "  {} 已是硬链接而跳过 {already} 份",
                style("○").dim()
            )?;
        }
        if !failures.is_empty() {
            writeln!(
                human,
                "  {} 失败 {} 份（原文件已恢复原名，内容未变）",
                style("✗").red(),
                failures.len()
            )?;
            for failure in failures.iter().take(10) {
                writeln!(
                    human,
                    "      {}",
                    style(format!("第 {} 组", failure.group)).dim()
                )?;
                writeln!(human, "      {}", style(&failure.reason).dim())?;
                writeln!(human, "      {}", style(&failure.path).dim())?;
            }
        }
    }
    writeln!(human, "{}", style("═".repeat(64)).dim())?;

    let has_failures = !failures.is_empty();
    let execution = report::ExecutionReport {
        linked,
        already_linked: already,
        reclaimed_bytes: reclaimed,
        verified,
        failures,
    };
    if args.json {
        emit_json(
            &args,
            &root,
            &groups,
            &outcome,
            candidates.len(),
            &digest_outcome.failed,
            Some(execution),
        )?;
    }

    Ok(if has_failures {
        ExitCode::from(EXIT_PARTIAL)
    } else {
        ExitCode::SUCCESS
    })
}

/// 复核：处理过的路径是否确实与主副本共享同一文件身份。
fn verify_shared_identity(groups: &[model::DuplicateGroup], indices: &[usize]) -> usize {
    let mut verified = 0usize;
    for &index in indices {
        let group = &groups[index];
        let keeper = group.keeper().path.clone();
        for member in group.pending() {
            if platform::is_same_file(&member.path, &keeper) == Some(true) {
                verified += 1;
            }
        }
    }
    verified
}

/// 解析扫描根目录：显式指定优先，否则用可执行文件所在目录。
///
/// 📌【2026-10-30】`cargo run` 时可执行文件位于 `target/` 下，那不是用户想扫描的目录，
/// 此时退回当前工作目录，避免开发期行为与发布版行为不一致。
fn resolve_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let executable = std::env::current_exe().context("获取当前程序路径失败")?;
    let directory = executable
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    if directory
        .components()
        .any(|component| component.as_os_str() == "target")
    {
        return std::env::current_dir().context("获取当前工作目录失败");
    }
    Ok(directory)
}

fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
}

/// 输出 JSON 报告到 stdout。
fn emit_json(
    args: &Args,
    root: &Path,
    groups: &[model::DuplicateGroup],
    outcome: &scan::ScanOutcome,
    candidate_files: usize,
    digest_failed: &[(PathBuf, String)],
    execution: Option<report::ExecutionReport>,
) -> Result<()> {
    let group_reports: Vec<report::GroupReport> = groups
        .iter()
        .enumerate()
        .map(|(index, group)| report::GroupReport {
            index: index + 1,
            size_bytes: group.size,
            digest: group.digest.hex(),
            reclaimable_bytes: group.reclaimable(),
            keeper: group.keeper().path.display().to_string(),
            members: group
                .members
                .iter()
                .map(|member| member.path.display().to_string())
                .collect(),
            editable: group
                .pending()
                .iter()
                .any(|member| model::is_editable(&member.path)),
        })
        .collect();

    let report = report::JsonReport {
        root: root.display().to_string(),
        scanned_files: outcome.entries.len(),
        candidate_files,
        duplicate_groups: groups.len(),
        redundant_copies: groups.iter().map(|group| group.pending().len()).sum(),
        reclaimable_bytes: groups.iter().map(|group| group.reclaimable()).sum(),
        already_linked_copies: group::already_linked_count(groups),
        editable_warning_groups: group_reports.iter().filter(|item| item.editable).count(),
        unreadable_files: digest_failed
            .iter()
            .map(|(path, reason)| format!("{}: {}", path.display(), reason))
            .collect(),
        groups: group_reports,
        execution,
    };

    let json = serde_json::to_string_pretty(&report).context("序列化 JSON 报告失败")?;
    println!("{json}");
    let _ = args;
    Ok(())
}
