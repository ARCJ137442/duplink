//! 报告渲染。
//!
//! 📌【2026-10-30】所有人类可读输出都通过显式的 `Write` 参数发出，而不是直接写
//! `println!`。这样 `--json` 模式可以把人类信息导向 stderr、JSON 导向 stdout，
//! 两者互不污染。

use std::io::Write;
use std::path::Path;

use console::style;
use serde::Serialize;

use crate::model::{human_size, is_editable, DuplicateGroup};

/// 尽量显示相对根目录的短路径。
pub fn short_path(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(relative) => relative.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// 单个重复组的 JSON 表示。
#[derive(Serialize)]
pub struct GroupReport {
    pub index: usize,
    pub size_bytes: u64,
    pub digest: String,
    pub reclaimable_bytes: u64,
    pub keeper: String,
    pub members: Vec<String>,
    /// 组内是否有"会被原地编辑"的文本/代码类文件。
    pub editable: bool,
}

/// 一次失败的处理。
#[derive(Serialize)]
pub struct FailureReport {
    /// 失败发生在第几组重复文件。
    pub group: usize,
    pub path: String,
    pub reason: String,
}

/// 执行阶段的结果。
#[derive(Serialize)]
pub struct ExecutionReport {
    pub linked: usize,
    pub already_linked: usize,
    pub reclaimed_bytes: u64,
    /// 复核确认"与主副本共享同一文件身份"的处理数。
    pub verified: usize,
    pub failures: Vec<FailureReport>,
}

/// 完整报告（JSON 输出）。
#[derive(Serialize)]
pub struct JsonReport {
    pub root: String,
    pub scanned_files: usize,
    pub candidate_files: usize,
    pub duplicate_groups: usize,
    pub redundant_copies: usize,
    pub reclaimable_bytes: u64,
    pub already_linked_copies: usize,
    pub editable_warning_groups: usize,
    /// 读取失败、未参与比对的文件。
    pub unreadable_files: Vec<String>,
    pub groups: Vec<GroupReport>,
    pub execution: Option<ExecutionReport>,
}

/// 打印运行参数摘要。
pub fn print_header<W: Write>(
    out: &mut W,
    root: &Path,
    min_size: Option<u64>,
    excludes: &[String],
    dry_run: bool,
) -> std::io::Result<()> {
    writeln!(out, "{}", style("═".repeat(64)).dim())?;
    writeln!(
        out,
        "{}",
        style("  duplink · 重复文件检测与硬链接合并").bold()
    )?;
    writeln!(out, "{}", style("═".repeat(64)).dim())?;
    writeln!(out, "扫描根目录: {}", root.display())?;
    match min_size {
        Some(bytes) => writeln!(out, "最小文件: {}", human_size(bytes))?,
        None => writeln!(out, "最小文件: {}", style("不限").dim())?,
    }
    if !excludes.is_empty() {
        writeln!(out, "排除规则: {}", excludes.join(", "))?;
    }
    if dry_run {
        writeln!(out, "{}", style("[预览模式] 不会修改任何文件").yellow())?;
    }
    writeln!(out)?;
    Ok(())
}

/// 打印重复组列表。
pub fn print_groups<W: Write>(
    out: &mut W,
    groups: &[DuplicateGroup],
    root: &Path,
    max_listed: usize,
) -> std::io::Result<()> {
    let shown = if max_listed == 0 {
        groups.len()
    } else {
        groups.len().min(max_listed)
    };

    for (position, group) in groups.iter().take(shown).enumerate() {
        let index = position + 1;
        let pending = group.pending().len();
        writeln!(
            out,
            "  {:>4}. {} {}",
            style(format!("{index}")).bold(),
            style(format!("[{:>9} ×{}]", human_size(group.size), pending + 1)).cyan(),
            style(group.keeper().name()).bold()
        )?;
        writeln!(
            out,
            "        {} {}",
            style("⭐ 保留").green(),
            short_path(root, &group.keeper().path)
        )?;
        for member in group.pending() {
            writeln!(
                out,
                "           {} {}",
                style("→ 转换").yellow(),
                short_path(root, &member.path)
            )?;
        }
        for member in group.already_linked() {
            writeln!(
                out,
                "           {} {}",
                style("= 已链接").dim(),
                short_path(root, &member.path)
            )?;
        }
    }

    if shown < groups.len() {
        writeln!(
            out,
            "  {}",
            style(format!(
                "… 另有 {} 组未列出（用 --max-listed 0 显示全部）",
                groups.len() - shown
            ))
            .dim()
        )?;
    }
    Ok(())
}

/// 打印汇总统计。
pub fn print_summary<W: Write>(out: &mut W, groups: &[DuplicateGroup]) -> std::io::Result<()> {
    let redundant: usize = groups.iter().map(|group| group.pending().len()).sum();
    let reclaimable: u64 = groups.iter().map(|group| group.reclaimable()).sum();
    let already = groups
        .iter()
        .map(|group| group.already_linked().len())
        .sum::<usize>();

    writeln!(out, "{}", style("─".repeat(64)).dim())?;
    writeln!(
        out,
        "重复组 {} 组 / 冗余副本 {} 份 / 可回收 {}",
        style(groups.len().to_string()).bold(),
        style(redundant.to_string()).bold(),
        style(human_size(reclaimable)).bold().green()
    )?;
    if already > 0 {
        writeln!(
            out,
            "其中 {} 份已经是硬链接，无需处理",
            style(already.to_string()).dim()
        )?;
    }
    writeln!(out, "{}", style("─".repeat(64)).dim())?;
    Ok(())
}

/// 打印"原地编辑会互相影响"的提示。
pub fn print_editable_warning<W: Write>(
    out: &mut W,
    groups: &[DuplicateGroup],
) -> std::io::Result<()> {
    let affected: Vec<&DuplicateGroup> = groups
        .iter()
        .filter(|group| {
            group
                .pending()
                .iter()
                .any(|member| is_editable(&member.path))
        })
        .collect();
    if affected.is_empty() {
        return Ok(());
    }
    writeln!(
        out,
        "{}",
        style(format!(
            "⚠️  {} 组包含文本/代码类文件（.md/.yaml/.ipynb/.js 等）。",
            affected.len()
        ))
        .yellow()
    )?;
    writeln!(
        out,
        "   转为硬链接后原地编辑其中一个，会同步改变其它路径。可用 safe 选项排除。",
    )?;
    Ok(())
}
