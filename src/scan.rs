//! 遍历与候选筛选。
//!
//! 📌【2026-10-30】排除规则在这里定义一次：路径匹配与"目录名匹配"两种语义都收敛到
//! `ExcludeMatcher`，其它阶段不再各自解释 glob。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use glob::{MatchOptions, Pattern};
use walkdir::WalkDir;

use crate::model::FileEntry;

/// 排除匹配器。
///
/// 规则（两条同时生效，命中任一即排除）：
/// 1. 模式匹配"相对扫描根目录"的完整路径，`*` 不跨越目录分隔符；
/// 2. 不含 `/` 的模式同时与路径中的任一段匹配——段既可能是目录名，也可能是文件名。
///    因此 `target` 可排除任意层级的 target 目录，`*.tmp` 可排除任意层级的 .tmp 文件。
///
/// 📌【2026-10-30】第 2 条是有意为之：用户写 `*.tmp` 时期待的是"所有 .tmp 文件"，
/// 而不是"根目录下第一层的 .tmp 文件"。需要精确限定层级时写成 `sub/*.tmp` 即可，
/// 那种写法只走第 1 条。
pub struct ExcludeMatcher {
    patterns: Vec<(String, Pattern)>,
}

impl ExcludeMatcher {
    pub fn new(raw: &[String]) -> Result<Self> {
        let mut patterns = Vec::with_capacity(raw.len());
        for item in raw {
            let normalized = item.replace('\\', "/");
            let pattern =
                Pattern::new(&normalized).with_context(|| format!("排除模式无效: {item}"))?;
            patterns.push((normalized, pattern));
        }
        Ok(ExcludeMatcher { patterns })
    }

    /// 判断路径是否应被排除。
    pub fn matches(&self, path: &Path, root: &Path) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let relative = relative_slash(path, root);
        self.patterns.iter().any(|(raw, pattern)| {
            if pattern.matches_with(&relative, match_options()) {
                return true;
            }
            if !raw.contains('/') {
                return relative
                    .split('/')
                    .any(|segment| pattern.matches_with(segment, match_options()));
            }
            false
        })
    }
}

fn match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
}

/// 把路径转换成相对根目录、以 `/` 分隔的字符串。
fn relative_slash(path: &Path, root: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// 文件被跳过的原因。
#[derive(Clone, Debug)]
pub enum SkipReason {
    /// 空文件：不占实际空间，无去重价值。
    EmptyFile,
    /// 小于 `--min-size` 阈值。
    BelowMinSize,
    /// 符号链接：不参与去重，避免跨目录成环。
    Symlink,
    /// 元数据不可读。
    MetadataUnavailable,
}

impl SkipReason {
    pub fn label(&self) -> &'static str {
        match self {
            SkipReason::EmptyFile => "空文件",
            SkipReason::BelowMinSize => "小于阈值",
            SkipReason::Symlink => "符号链接",
            SkipReason::MetadataUnavailable => "元数据不可读",
        }
    }
}

/// 扫描配置。
pub struct ScanConfig {
    pub min_size: u64,
    pub excludes: ExcludeMatcher,
    pub max_depth: Option<usize>,
}

/// 扫描结果。
pub struct ScanOutcome {
    pub entries: Vec<FileEntry>,
    pub skipped: Vec<(PathBuf, SkipReason)>,
    /// 遍历过程中无法读取的目录数量。
    pub unreadable_dirs: usize,
    /// 扫描范围内遇到的 reparse point（符号链接 / 目录联接）。
    pub reparse_points: Vec<ReparsePoint>,
}

/// 一个 reparse point：Windows 上把符号链接与目录联接统称 reparse point。
///
/// 📌【2026-10-30】这类条目不参与去重是正确的——跟进它们可能越出扫描根，
/// 也可能把同一批文件重复计入。但它们**遮蔽**的文件必须让用户知道：
/// 否则"扫描了 11049 个文件"会让人以为整棵树都覆盖到了。
#[derive(Clone, Debug)]
pub struct ReparsePoint {
    /// 该条目在扫描树中的位置。
    pub path: PathBuf,
    /// 指向的目标（无法解析时为 `None`）。
    pub target: Option<PathBuf>,
    /// 目标内被遮蔽的文件数（有界统计，可能为 `None` 表示未能统计）。
    pub hidden_files: Option<u64>,
}

/// 统计 reparse point 遮蔽文件数时的遍历上限。
const HIDDEN_SCAN_MAX_DEPTH: usize = 8;
/// 单次统计允许访问的文件总数上限，避免 junction 指向超大目录时卡住。
const HIDDEN_SCAN_FILE_BUDGET: u64 = 500_000;

/// 递归收集文件元数据。
///
/// 📌【2026-10-30】只跟随普通文件：符号链接与目录联接不参与去重（跨目录成环、
/// 越出扫描根、"链接到目录外的同一份数据"都会让分组结果失真），
/// 排除命中的目录在遍历阶段就剪枝，避免无谓的元数据读取。
pub fn collect_files(root: &Path, config: &ScanConfig) -> ScanOutcome {
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut unreadable_dirs = 0usize;
    let mut reparse_points: Vec<ReparsePoint> = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(config.max_depth.unwrap_or(usize::MAX))
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !config.excludes.matches(entry.path(), root));

    for item in walker {
        let entry = match item {
            Ok(entry) => entry,
            Err(_) => {
                unreadable_dirs += 1;
                continue;
            }
        };
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            let path = entry.path().to_path_buf();
            skipped.push((path.clone(), SkipReason::Symlink));
            reparse_points.push(ReparsePoint {
                target: resolve_target(&path),
                path,
                hidden_files: None,
            });
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                skipped.push((entry.path().to_path_buf(), SkipReason::MetadataUnavailable));
                continue;
            }
        };

        let size = metadata.len();
        if size == 0 {
            skipped.push((entry.path().to_path_buf(), SkipReason::EmptyFile));
            continue;
        }
        if size < config.min_size {
            skipped.push((entry.path().to_path_buf(), SkipReason::BelowMinSize));
            continue;
        }

        entries.push(FileEntry {
            path: entry.path().to_path_buf(),
            size,
            modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
            digest: None,
        });
    }

    let mut outcome = ScanOutcome {
        entries,
        skipped,
        unreadable_dirs,
        reparse_points,
    };
    count_hidden_files(&mut outcome.reparse_points);
    outcome
}

/// 统计每个 reparse point 遮蔽的文件数。
///
/// 📌【2026-10-31] 遍历必须是有界的，否则一个指向自身祖先的 junction 会让程序挂死。
/// 三重保险：限制深度、用规范化路径去重防环、限制文件总量。
///
/// 📌【2026-10-31] 每个 reparse point **独立**统计：共享去重集合会让"两个目标指向同一批文件"
/// 的情况漏计（实测漏掉了 1808 个）。独立统计给出的是每个目标子树各自的真实体量，
/// 汇总值可能因目标重叠而偏大——报告里会注明这一点。
pub fn count_hidden_files(reparse_points: &mut [ReparsePoint]) {
    let mut budget = HIDDEN_SCAN_FILE_BUDGET;
    for reparse in reparse_points.iter_mut() {
        let mut visited: HashSet<PathBuf> = HashSet::new();
        reparse.hidden_files = Some(count_files_bounded(
            &reparse.path,
            HIDDEN_SCAN_MAX_DEPTH,
            &mut budget,
            &mut visited,
        ));
    }
}

/// 解析 reparse point 的目标，输出可读的绝对路径。
///
/// `read_link` 对相对目标会原样返回 `..\..\Develop\...` 这类写法，
/// 对用户没有参考价值；这里补成绝对路径，并尽可能展开成真实位置。
fn resolve_target(link: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_link(link).ok()?;
    let absolute = if raw.is_absolute() {
        raw.clone()
    } else {
        link.parent()
            .map(|parent| parent.join(&raw))
            .unwrap_or_else(|| raw.clone())
    };
    Some(strip_verbatim_prefix(
        std::fs::canonicalize(&absolute).unwrap_or(absolute),
    ))
}

/// 去掉 Windows 的 `\\?\` 逐字路径前缀。
///
/// `canonicalize` 固定返回这种形式，它能正确工作但人读不了；报告里应当给出常规路径。
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().to_string();
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return path;
    };
    // `\\?\UNC\server\share` 的常规写法是 `\\server\share`
    if let Some(unc) = rest.strip_prefix(r"UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    PathBuf::from(rest)
}

/// 有界地统计目录树中的文件数。
fn count_files_bounded(
    root: &Path,
    max_depth: usize,
    budget: &mut u64,
    visited: &mut HashSet<PathBuf>,
) -> u64 {
    let mut count = 0u64;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((directory, depth)) = stack.pop() {
        if depth > max_depth || *budget == 0 {
            continue;
        }
        // 规范化路径把 junction 展开成真实目标，指向祖先的链接因此会命中同一 key。
        let key = std::fs::canonicalize(&directory).unwrap_or_else(|_| directory.clone());
        if !visited.insert(key) {
            continue;
        }
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return count;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                // 嵌套的 reparse point 也要跟进：它遮蔽的内容同样没进扫描范围。
                // 指向目录的当作目录继续遍历，指向文件的计一个。
                match std::fs::metadata(entry.path()) {
                    Ok(target) if target.is_dir() => {
                        stack.push((entry.path(), depth + 1));
                    }
                    Ok(_) => {
                        count += 1;
                        *budget -= 1;
                    }
                    Err(_) => continue,
                }
                continue;
            }
            if file_type.is_dir() {
                stack.push((entry.path(), depth + 1));
            } else {
                count += 1;
                *budget -= 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(patterns: &[&str]) -> ExcludeMatcher {
        let owned: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        ExcludeMatcher::new(&owned).unwrap()
    }

    #[test]
    fn matches_relative_path() {
        let root = Path::new("/root");
        let m = matcher(&["a/b.txt"]);
        assert!(m.matches(Path::new("/root/a/b.txt"), root));
        assert!(!m.matches(Path::new("/root/a/c.txt"), root));
    }

    #[test]
    fn bare_name_matches_any_segment() {
        let root = Path::new("/root");
        let m = matcher(&["target"]);
        assert!(m.matches(Path::new("/root/x/target"), root));
        assert!(m.matches(Path::new("/root/target/y"), root));
        assert!(!m.matches(Path::new("/root/x/targeted"), root));
    }

    #[test]
    fn star_does_not_cross_separator() {
        let root = Path::new("/root");
        let m = matcher(&["sub/*.tmp"]);
        assert!(m.matches(Path::new("/root/sub/a.tmp"), root));
        assert!(!m.matches(Path::new("/root/a.tmp"), root));
        assert!(!m.matches(Path::new("/root/sub/deep/a.tmp"), root));
    }

    #[test]
    fn bare_pattern_matches_any_segment_at_any_depth() {
        let root = Path::new("/root");
        let m = matcher(&["*.tmp"]);
        assert!(m.matches(Path::new("/root/a.tmp"), root));
        assert!(m.matches(Path::new("/root/sub/deep/a.tmp"), root));
    }

    #[test]
    fn relative_slash_normalises_separators() {
        let root = Path::new("/root");
        assert_eq!(relative_slash(Path::new("/root/a/b"), root), "a/b");
    }

    #[test]
    fn verbatim_prefix_is_stripped_for_display() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\C:\data\Media")),
            PathBuf::from(r"C:\data\Media")
        );
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\server\share")
        );
        // 没有前缀的路径原样返回
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"C:\data")),
            PathBuf::from(r"C:\data")
        );
    }
}
