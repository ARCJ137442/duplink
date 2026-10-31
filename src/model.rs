//! 领域类型：扫描结果、重复分组、内容摘要。
//!
//! 📌【2026-10-30】这些类型是扫描、分组、报告、执行四个阶段之间共享的契约。
//! 阶段之间只通过这些类型传递信息，不共享内部状态。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 文件内容摘要（SHA-256 原始字节）。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Digest(bytes)
    }

    /// 十六进制表示，用于报告与排错。
    pub fn hex(&self) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

impl std::fmt::Debug for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.hex())
    }
}

/// 扫描阶段收集到的单个文件。
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    /// 内容摘要，由摘要阶段填充。
    pub digest: Option<Digest>,
}

impl FileEntry {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.to_string_lossy().to_string())
    }
}

/// 一组内容完全相同的文件。
///
/// `members[0]` 是主副本（保留其路径与磁盘数据），其余成员按 `linked` 标记区分：
/// `true` 表示已经与主副本共享数据（已是硬链接），`false` 表示还需要转换。
#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub size: u64,
    pub digest: Digest,
    pub members: Vec<FileEntry>,
    /// 与主副本共享同一份数据的成员标记，下标与 `members` 对齐；`members[0]` 恒为 `false`。
    linked: Vec<bool>,
}

impl DuplicateGroup {
    /// 构造一组重复文件。`members` 必须非空且首项已选为主副本。
    pub fn new(size: u64, digest: Digest, members: Vec<FileEntry>) -> Self {
        let linked = vec![false; members.len()];
        DuplicateGroup {
            size,
            digest,
            members,
            linked,
        }
    }

    pub fn keeper(&self) -> &FileEntry {
        &self.members[0]
    }

    /// 标记某个成员是否已经与主副本共享数据。
    pub fn mark_linked(&mut self, index: usize, linked: bool) {
        if index < self.linked.len() {
            self.linked[index] = linked;
        }
    }

    /// 尚未与主副本共享磁盘数据的成员——这些才是真正需要建立硬链接的对象。
    pub fn pending(&self) -> Vec<&FileEntry> {
        self.members[1..]
            .iter()
            .enumerate()
            .filter(|(offset, _)| !self.linked[offset + 1])
            .map(|(_, member)| member)
            .collect()
    }

    /// 已经是硬链接、无需处理的成员。
    pub fn already_linked(&self) -> Vec<&FileEntry> {
        self.members[1..]
            .iter()
            .enumerate()
            .filter(|(offset, _)| self.linked[offset + 1])
            .map(|(_, member)| member)
            .collect()
    }

    /// 转换为硬链接后可以释放的磁盘字节数。
    pub fn reclaimable(&self) -> u64 {
        self.size * self.pending().len() as u64
    }
}

/// 这些扩展名代表"会被原地编辑"的文件。
///
/// ⚠️ 硬链接之后多个路径共享同一份数据，用编辑器原地保存其中一个会同步改变其余路径。
/// 报告阶段据此提示用户，避免把"文件被莫名改动"变成难以排查的灵异事件。
pub const EDITABLE_EXTENSIONS: &[&str] = &[
    "md", "txt", "yaml", "yml", "json", "jsonl", "ipynb", "ts", "tsx", "js", "jsx", "py", "sh",
    "bat", "ps1", "html", "css", "csv", "xml", "tex", "log", "note", "rs", "go", "java", "c",
    "cpp", "h", "toml", "ini", "cfg", "conf", "env", "sql", "patch", "diff",
];

/// 判断路径是否属于"会被原地编辑"的文件类型。
pub fn is_editable(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .map(|extension| EDITABLE_EXTENSIONS.contains(&extension.as_str()))
        .unwrap_or(false)
}

/// 这些目录名出现在路径中时，该文件被视为"临时副本"。
///
/// 📌【2026-10-30】主副本应当优先留在真实目录里：agent 工作区、缓存、回收站这类目录
/// 随时可能被清理，主副本放在那里等于把数据的存活挂在临时目录上。
/// 副本内容完全相同，所以换个路径承载数据不会损失任何东西。
const TEMPORARY_DIRECTORY_SEGMENTS: &[&str] = &[
    ".agents",
    "temp",
    "tmp",
    ".tmp",
    ".cache",
    "cache",
    "scratch",
    "trash",
    "$recycle.bin",
];

/// 判断路径是否位于临时/工作目录中。
pub fn is_in_temporary_directory(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|segment| {
                let lowered = segment.to_ascii_lowercase();
                TEMPORARY_DIRECTORY_SEGMENTS.contains(&lowered.as_str())
            })
            .unwrap_or(false)
    })
}

/// 文件名是否带有 `(1)`、`(2)` 这类副本标记。
///
/// 微信收到同名文件时会自动追加编号，`foo(1).pdf` 是 `foo.pdf` 的副本。
/// 主副本选择优先保留没有该标记的路径。
pub fn has_copy_marker(file_name: &str) -> bool {
    let stem = match file_name.rsplit_once('.') {
        Some((head, _)) if !head.is_empty() => head,
        _ => file_name,
    };
    let Some(open) = stem.rfind('(') else {
        return false;
    };
    if !stem.ends_with(')') {
        return false;
    }
    let inner = &stem[open + 1..stem.len() - 1];
    !inner.is_empty() && inner.bytes().all(|byte| byte.is_ascii_digit())
}

/// 尺寸解析失败。
#[derive(Debug)]
pub struct SizeParseError(pub String);

impl std::fmt::Display for SizeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SizeParseError {}

/// 解析 "100MB" / "1.5 GB" / "2048" 为字节数。裸数字按字节解释。
///
/// 📌【2026-10-30】字节数与可读字符串之间的换算只有这一处定义，参数解析与交互式选择
/// 共用它，避免同一套单位约定在两处各自实现。
pub fn parse_size(raw: &str) -> Result<u64, SizeParseError> {
    // 先去掉所有空白："1.5 GB" 与 "1.5GB" 应当等价，单位里混入空格是常见的手误。
    let text: String = raw
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let (number, unit): (String, String) =
        text.chars().partition(|c| c.is_ascii_digit() || *c == '.');

    if number.is_empty() {
        return Err(SizeParseError(format!("无法解析尺寸: {raw}")));
    }
    let value: f64 = number
        .parse()
        .map_err(|_| SizeParseError(format!("无法解析尺寸数字: {raw}")))?;

    const KB: f64 = 1024.0;
    let factor = match unit.as_str() {
        "" | "B" => 1.0,
        "K" | "KB" => KB,
        "M" | "MB" => KB * KB,
        "G" | "GB" => KB * KB * KB,
        "T" | "TB" => KB * KB * KB * KB,
        _ => {
            return Err(SizeParseError(format!(
                "未知尺寸单位: {unit}（支持 B/KB/MB/GB/TB）"
            )))
        }
    };
    Ok((value * factor) as u64)
}

/// 人类可读的字节数。
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;
    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_marker_detects_wechat_suffixes() {
        assert!(has_copy_marker("foo(1).pdf"));
        assert!(has_copy_marker("foo (12).zip"));
        assert!(has_copy_marker("Spark(3).pptx"));
    }

    #[test]
    fn copy_marker_ignores_normal_names() {
        assert!(!has_copy_marker("foo.pdf"));
        assert!(!has_copy_marker("report(最终).pdf"));
        assert!(!has_copy_marker("a(b)c.pdf"));
        assert!(!has_copy_marker("no_extension"));
    }

    #[test]
    fn editable_extensions_are_case_insensitive() {
        assert!(is_editable(Path::new("a.MD")));
        assert!(is_editable(Path::new("a.rs")));
        assert!(!is_editable(Path::new("a.pdf")));
        assert!(!is_editable(Path::new("a")));
    }

    #[test]
    fn temporary_directories_are_detected_by_segment() {
        assert!(is_in_temporary_directory(Path::new(
            "C:/x/.agents/work/ambiguous/a.mp3"
        )));
        assert!(is_in_temporary_directory(Path::new("C:/x/Temp/a.mp3")));
        assert!(is_in_temporary_directory(Path::new(
            "C:/x/CloudMusic/cache/a.mp3"
        )));
        // 只是名字里含有这些字样，不算临时目录
        assert!(!is_in_temporary_directory(Path::new(
            "C:/x/CloudMusic/temporary-mix/a.mp3"
        )));
        assert!(!is_in_temporary_directory(Path::new(
            "C:/x/CloudMusic/特殊/a.mp3"
        )));
    }

    #[test]
    fn size_round_trips_through_human_form() {
        let parsed = parse_size("1.5 MB").unwrap();
        assert_eq!(human_size(parsed), "1.50 MB");
    }
}
