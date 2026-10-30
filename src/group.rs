//! 分组与主副本选择。
//!
//! 📌【2026-10-30】主副本选择规则集中在本模块的 `keeper_cmp` 一处。
//! 这条规则决定"哪条路径保留原始数据"，未来若要改成"保留最新修改时间的副本"，
//! 只需要改这一个函数，不涉及其它阶段。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::model::{has_copy_marker, Digest, DuplicateGroup, FileEntry};
use crate::platform;

/// 选出需要计算摘要的候选文件下标。
///
/// 只有尺寸重复的文件才可能内容重复；先用尺寸把候选收缩到最小集合，避免为无关文件读盘。
pub fn candidate_indices(entries: &[FileEntry]) -> Vec<usize> {
    let mut counts: HashMap<u64, usize> = HashMap::new();
    for entry in entries {
        *counts.entry(entry.size).or_insert(0) += 1;
    }
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| counts.get(&entry.size).copied().unwrap_or(0) > 1)
        .map(|(index, _)| index)
        .collect()
}

/// 主副本优先级比较：返回 `Less` 表示 `a` 更适合当主副本。
///
/// 规则依次为：
/// 1. 没有 `(1)` 这类副本标记的路径优先——它是用户看到的"原名"；
/// 2. 修改时间更早的优先——保留"最初那份"的语义；
/// 3. 路径更短的优先；
/// 4. 路径字典序——保证同一批文件每次都得到相同结果。
fn keeper_cmp(a: &FileEntry, b: &FileEntry) -> Ordering {
    has_copy_marker(&a.name())
        .cmp(&has_copy_marker(&b.name()))
        .then_with(|| a.modified.cmp(&b.modified))
        .then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
        .then_with(|| a.path.as_os_str().cmp(b.path.as_os_str()))
}

/// 按内容摘要聚合重复组。
///
/// 组内 `members[0]` 为主副本；组按单份大小从大到小排列，
/// 让用户先看到最值钱的那几组。
pub fn build_groups(entries: &[FileEntry]) -> Vec<DuplicateGroup> {
    let mut buckets: HashMap<(u64, Digest), Vec<FileEntry>> = HashMap::new();
    for entry in entries {
        if let Some(digest) = entry.digest {
            buckets
                .entry((entry.size, digest))
                .or_default()
                .push(entry.clone());
        }
    }

    let mut groups: Vec<DuplicateGroup> = buckets
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|((size, digest), mut members)| {
            members.sort_by(keeper_cmp);
            DuplicateGroup::new(size, digest, members)
        })
        .collect();
    groups.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.digest.cmp(&b.digest)));
    groups
}

/// 区分"已经是硬链接"与"仍需转换"的成员。
///
/// 📌【2026-10-30】身份比较放在分组之后而不是扫描阶段：比较需要打开句柄，
/// 对目录中全部文件都做一遍会把 I/O 放大到与文件总数成正比，
/// 而绝大多数文件根本不会进入重复组。
pub fn partition_by_identity(groups: &mut [DuplicateGroup]) {
    for group in groups.iter_mut() {
        let paths: Vec<PathBuf> = group
            .members
            .iter()
            .map(|member| member.path.clone())
            .collect();
        let keeper = paths[0].clone();
        for (index, member_path) in paths.iter().enumerate().skip(1) {
            // 无法判断时按"尚未链接"处理：宁可多处理一次，也不能把已链接的文件当成独立副本。
            let linked = platform::is_same_file(member_path, &keeper).unwrap_or(false);
            group.mark_linked(index, linked);
        }
    }
}

/// 统计所有重复组中"已经被硬链接、无需再处理"的成员数量。
pub fn already_linked_count(groups: &[DuplicateGroup]) -> usize {
    groups
        .iter()
        .map(|group| group.already_linked().len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn entry(name: &str, size: u64, age_secs: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from("/root").join(name),
            size,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(age_secs),
            digest: None,
        }
    }

    #[test]
    fn keeper_prefers_clean_name_over_earlier_time() {
        let clean = entry("doc.pdf", 10, 100);
        let marked = entry("doc(1).pdf", 10, 50);
        assert_eq!(keeper_cmp(&clean, &marked), Ordering::Less);
    }

    #[test]
    fn keeper_prefers_earlier_time_when_names_match_style() {
        let older = entry("a.pdf", 10, 50);
        let newer = entry("b.pdf", 10, 100);
        assert_eq!(keeper_cmp(&older, &newer), Ordering::Less);
    }

    #[test]
    fn candidates_only_include_repeated_sizes() {
        let entries = vec![entry("a", 100, 0), entry("b", 100, 0), entry("c", 200, 0)];
        assert_eq!(candidate_indices(&entries), vec![0, 1]);
    }
}
