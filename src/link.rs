//! 硬链接执行——本工具唯一会改动磁盘数据的模块。
//!
//! 📌【2026-10-30】这里集中了全部破坏性副作用，其它模块只负责计算与报告。
//! 单个副本的处理是一个"改名 → 建链 → 校验 → 删除"的多步序列，
//! 任何一步失败都必须把文件恢复到处理前的状态。

use std::fs;
use std::path::{Path, PathBuf};

use crate::digest::digest_of;
use crate::model::{Digest, DuplicateGroup, FileEntry};
use crate::platform;

/// 单个副本的处理结果。
#[derive(Debug)]
pub enum LinkOutcome {
    /// 已经与主副本共享数据，无需处理。
    AlreadyLinked,
    /// 成功建立硬链接，释放的字节数。
    Linked { reclaimed: u64 },
    /// 处理失败，原文件已恢复原名、内容未变。
    Failed { reason: String },
}

/// 一个重复组的处理结果。
#[derive(Debug)]
pub struct GroupOutcome {
    pub group_index: usize,
    pub results: Vec<(PathBuf, LinkOutcome)>,
}

impl GroupOutcome {
    pub fn linked_count(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, outcome)| matches!(outcome, LinkOutcome::Linked { .. }))
            .count()
    }

    pub fn already_linked_count(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, outcome)| matches!(outcome, LinkOutcome::AlreadyLinked))
            .count()
    }

    pub fn reclaimed(&self) -> u64 {
        self.results
            .iter()
            .filter_map(|(_, outcome)| match outcome {
                LinkOutcome::Linked { reclaimed } => Some(*reclaimed),
                _ => None,
            })
            .sum()
    }

    pub fn failures(&self) -> Vec<&(PathBuf, LinkOutcome)> {
        self.results
            .iter()
            .filter(|(_, outcome)| matches!(outcome, LinkOutcome::Failed { .. }))
            .collect()
    }
}

/// 处理一个重复组：把组内除主副本外的成员全部转换为指向主副本的硬链接。
pub fn link_group(group: &DuplicateGroup, group_index: usize, dry_run: bool) -> GroupOutcome {
    let keeper = group.keeper();
    let mut results = Vec::with_capacity(group.members.len().saturating_sub(1));

    for member in group.pending() {
        let outcome = if dry_run {
            LinkOutcome::Linked {
                reclaimed: member.size,
            }
        } else {
            link_member(keeper, member, group.digest)
        };
        results.push((member.path.clone(), outcome));
    }

    for member in group.already_linked() {
        results.push((member.path.clone(), LinkOutcome::AlreadyLinked));
    }

    GroupOutcome {
        group_index,
        results,
    }
}

/// 把单个副本转换为指向主副本的硬链接。
///
/// 安全序列（顺序本身就是安全承诺，调整顺序会削弱回退能力）：
/// 1. 把副本改名为临时名——原始数据仍独占存在于盘上，任何后续失败都可原样恢复；
/// 2. 在原路径建立指向主副本的硬链接——此时两个路径共享同一份数据；
/// 3. 校验新链接与主副本确为同一份数据——不一致立即撤销；
/// 4. 清除只读位后删除临时文件——到这一步磁盘占用才真正下降。
///
/// ⚠️ 为什么不是"先建链接、再删原件"：那样一旦建链失败，原始数据已经被删除，无法回退。
/// 当前顺序把"数据仍独占存在"的状态延长到建链与校验全部完成之后。
pub fn link_member(keeper: &FileEntry, member: &FileEntry, expected: Digest) -> LinkOutcome {
    let temporary = temporary_path(&member.path);

    if let Err(error) = fs::rename(&member.path, &temporary) {
        return LinkOutcome::Failed {
            reason: format!("改名失败: {error}"),
        };
    }

    if let Err(error) = fs::hard_link(&keeper.path, &member.path) {
        restore(&member.path, &temporary);
        return LinkOutcome::Failed {
            reason: format!("建立硬链接失败: {error}"),
        };
    }

    if let Err(reason) = verify_same_data(&member.path, keeper, expected) {
        restore(&member.path, &temporary);
        return LinkOutcome::Failed { reason };
    }

    if let Err(error) = platform::clear_readonly(&temporary) {
        restore(&member.path, &temporary);
        return LinkOutcome::Failed {
            reason: format!("清除只读位失败: {error}"),
        };
    }

    if let Err(error) = fs::remove_file(&temporary) {
        // 链接已经生效，但原始数据尚未释放；把临时文件留在原处并报告，交由用户处理。
        return LinkOutcome::Failed {
            reason: format!("删除临时文件失败（链接已生效，数据尚未释放）: {error}"),
        };
    }

    LinkOutcome::Linked {
        reclaimed: member.size,
    }
}

/// 校验新建立的链接与主副本指向同一份数据。
///
/// 优先比较文件身份（跨平台可靠的"同一文件"判据）；文件系统不提供身份时，
/// 退回到内容摘要比较，保证校验不会因为平台能力缺失而直接失效。
fn verify_same_data(path: &Path, keeper: &FileEntry, expected: Digest) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("校验失败，无法读取新链接: {error}"))?;
    if metadata.len() != keeper.size {
        return Err(format!(
            "校验失败：大小不一致（期望 {}，实际 {}）",
            keeper.size,
            metadata.len()
        ));
    }

    match platform::is_same_file(path, &keeper.path) {
        Some(true) => Ok(()),
        Some(false) => Err("校验失败：新链接与主副本不是同一个文件".to_string()),
        None => {
            // 无法比较文件身份时退回内容摘要，保证校验不会因为平台能力缺失而直接失效。
            let actual_digest =
                digest_of(path).map_err(|error| format!("校验失败，无法计算摘要: {error}"))?;
            if actual_digest == expected {
                Ok(())
            } else {
                Err("校验失败：内容摘要与期望不一致".to_string())
            }
        }
    }
}

/// 撤销一次未完成的处理：移除可能已创建的半成品链接，把临时文件改回原名。
fn restore(original: &Path, temporary: &Path) {
    // 移除链接只影响这一个目录项，主副本的数据不受影响。
    let _ = fs::remove_file(original);
    let _ = fs::rename(temporary, original);
}

/// 临时文件名：与原文件同目录，保证改名是同一卷内的原子操作。
fn temporary_path(original: &Path) -> PathBuf {
    let mut name = original
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".duplink.tmp");
    original.with_file_name(name)
}
