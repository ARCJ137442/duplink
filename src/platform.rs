//! 平台差异隔离层。
//!
//! 📌【2026-10-30】本模块是全局唯一允许出现 `#[cfg(windows)]` / `#[cfg(unix)]` 的位置。
//! 上层模块只依赖"两个路径是否同一个文件"这一跨平台语义，不感知 inode / file index 的差异。
//!
//! 📌【2026-10-30】文件身份必须从"打开的句柄"读取：`std::os::windows::fs::MetadataExt`
//! 的 `file_index()` / `number_of_links()` 至今仍是 unstable，不能作为依赖。
//! `same-file` 提供了稳定封装，Windows 上落到 `GetFileInformationByHandle`，
//! 比较的是卷序列号与文件索引号。

use std::path::Path;

/// 判断两个路径是否指向同一个文件（同一份磁盘数据）。
///
/// 这正是判断"两个路径是否已经是硬链接"的判据：仅比较文件内容无法区分
/// "内容相同的两个独立文件"与"已经硬链接的同一份数据"。
///
/// 返回 `None` 表示无法判断（例如文件被独占锁定）。调用方不得把 `None` 当成"不是同一个文件"，
/// 否则会把已经链接的文件重复处理一遍。
pub fn is_same_file(left: &Path, right: &Path) -> Option<bool> {
    same_file::is_same_file(left, right).ok()
}

/// 清除只读位。
///
/// 📌【2026-10-30】微信等程序会把接收到的文件标记为只读，而 Windows 上删除只读文件
/// 会失败。执行删除前必须先解除只读位，因此该能力放在平台层而非调用方各自处理。
///
/// Unix 上删除文件取决于**目录**的写权限，文件自身的只读位并不阻止删除，
/// 因此那里不做任何权限改动（避免把文件变成 world-writable）。
pub fn clear_readonly(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let mut permissions = std::fs::metadata(path)?.permissions();
        if permissions.readonly() {
            // clippy 的 permissions_set_readonly_false 提醒针对 Unix 语义；
            // 这段代码只在 Windows 上编译，Windows 的只读位正是删除被阻断的原因。
            #[allow(clippy::permissions_set_readonly_false)]
            {
                permissions.set_readonly(false);
            }
            std::fs::set_permissions(path, permissions)?;
        }
    }
    #[cfg(unix)]
    {
        let _ = path;
    }
    Ok(())
}
