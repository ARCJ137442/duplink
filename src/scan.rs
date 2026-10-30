//! 遍历与候选筛选。
//!
//! 📌【2026-10-30】排除规则在这里定义一次：路径匹配与"目录名匹配"两种语义都收敛到
//! `ExcludeMatcher`，其它阶段不再各自解释 glob。

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
}

/// 递归收集文件元数据。
///
/// 📌【2026-10-30】只跟随普通文件：符号链接不参与去重（跨目录成环与"链接到目录外的同一份数据"
/// 都会让分组结果失真），排除命中的目录在遍历阶段就剪枝，避免无谓的元数据读取。
pub fn collect_files(root: &Path, config: &ScanConfig) -> ScanOutcome {
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut unreadable_dirs = 0usize;

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
            skipped.push((entry.path().to_path_buf(), SkipReason::Symlink));
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

    ScanOutcome {
        entries,
        skipped,
        unreadable_dirs,
    }
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
}
