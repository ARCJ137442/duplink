//! 终端交互与退出行为。
//!
//! 📌【2026-10-30】面向用户的命令行程序不能"闪退"。用户双击 exe 运行时，Windows 会为它
//! 新建一个控制台窗口，进程一结束窗口就随之关闭——程序其实是正常运行完的，但用户什么
//! 都看不到，无法区分"正常运行后停止"与"中途异常崩溃"。
//!
//! 本模块统一负责三件事：
//! 1. 判断当前是否有人类在终端前交互；
//! 2. 在窗口即将消失时等待用户确认；
//! 3. 把 panic 转成可读的诊断信息，而不是默认的一行英文后静默退出。

use std::io::{self, IsTerminal, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use console::style;

/// 记录本次运行是否属于"双击启动"，供 panic 钩子读取。
static LAUNCHED_WITHOUT_ARGUMENTS: AtomicBool = AtomicBool::new(false);

/// 记录启动方式。必须在开始任何可能 panic 的工作之前调用。
pub fn record_launch_arguments(args: &[String]) {
    LAUNCHED_WITHOUT_ARGUMENTS.store(launched_without_arguments(args), Ordering::Relaxed);
}

/// 当前运行是否属于"双击启动"。
pub fn is_bare_launch() -> bool {
    LAUNCHED_WITHOUT_ARGUMENTS.load(Ordering::Relaxed)
}

/// 退出时的情形，决定是否需要等待用户确认。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    /// 正常完成。
    Success,
    /// 以错误或部分失败告终——这种情况下用户更需要看到输出。
    Failure,
    /// 主动输出信息（`--help` / `--version`）。
    ///
    /// 📌【2026-10-30] 这类输出同样是在一个即将随进程消失的窗口里打印的，
    /// 用户多半来不及读完，因此与错误一样必须保留窗口。
    Informational,
}

/// 当前是否有人类在终端前交互。
///
/// stdout 与 stdin 必须**同时**是终端：只要有一个被重定向，本次运行就可能处在自动化
/// 流程里，此时任何"等待回车"都会卡死脚本。
pub fn is_interactive() -> bool {
    io::stdout().is_terminal() && io::stdin().is_terminal()
}

/// 判断本次运行是否属于"双击启动"。
///
/// 依据是没有收到任何命令行参数：从终端里显式执行通常会带参数（哪怕只是 `--path`），
/// 而双击运行必然一个参数都没有。
pub fn launched_without_arguments(args: &[String]) -> bool {
    args.len() <= 1
}

/// 决定是否需要在退出前等待用户确认。
pub fn should_wait(kind: ExitKind, launched_without_arguments: bool) -> bool {
    if !is_interactive() {
        return false;
    }
    match kind {
        // 出错时无论如何都该让人看见错误信息。
        ExitKind::Failure => true,
        // 主动输出的信息同样需要停留。
        ExitKind::Informational => true,
        // 正常结束时，只在"双击启动"这一种窗口会随进程消失的场景下等待。
        ExitKind::Success => launched_without_arguments,
    }
}

/// 在退出前等待用户按下回车，避免控制台窗口随进程一起消失。
pub fn wait_before_exit(kind: ExitKind, launched_without_arguments: bool) {
    if !should_wait(kind, launched_without_arguments) {
        return;
    }
    let mut stdout = io::stdout();
    let _ = writeln!(stdout);
    let _ = writeln!(stdout, "{}", style(pause_hint(kind)).dim());
    let _ = stdout.flush();

    // 只读一个字节：终端上按下的回车即满足；Ctrl+C 等异常情况读失败也无妨。
    let mut buffer = [0u8; 1];
    let _ = io::stdin().read_exact(&mut buffer);
}

/// 退出前的提示语。区分情形，让用户知道接下来看到的是什么。
pub fn pause_hint(kind: ExitKind) -> &'static str {
    match kind {
        ExitKind::Success => "按回车键关闭窗口…",
        ExitKind::Failure => "按回车键关闭（出错信息如上，请先读完再关）",
        ExitKind::Informational => "按回车键关闭窗口…",
    }
}

/// 安装 panic 钩子：把 Rust 默认的一行英文 panic 信息换成可读的诊断。
///
/// 📌【2026-10-30】默认钩子只打印 `thread 'main' panicked at ...` 就让进程死掉，
/// 双击运行时用户连这一行都来不及看到。这里额外给出原因、位置与影响说明，
/// 并保持窗口停留，让用户能截图或抄录。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "未知位置".to_string());
        let payload = info.payload();
        let reason = if let Some(text) = payload.downcast_ref::<&str>() {
            (*text).to_string()
        } else if let Some(text) = payload.downcast_ref::<String>() {
            text.clone()
        } else {
            "未知原因".to_string()
        };

        let mut stderr = io::stderr();
        let _ = writeln!(stderr);
        let _ = writeln!(
            stderr,
            "{}",
            style("════════════════════════════════════════════════════════════").red()
        );
        let _ = writeln!(
            stderr,
            "{}",
            style("✗ duplink 遇到未预期的错误并已停止").red().bold()
        );
        let _ = writeln!(
            stderr,
            "{}",
            style("════════════════════════════════════════════════════════════").red()
        );
        let _ = writeln!(stderr, "  原因: {reason}");
        let _ = writeln!(stderr, "  位置: {location}");
        let _ = writeln!(
            stderr,
            "  数据安全: 硬链接转换是原子的——要么完成并校验通过，要么回滚恢复原文件。"
        );
        let _ = writeln!(
            stderr,
            "  复现请连同上面的原因与位置一起反馈；加 RUST_BACKTRACE=1 可获得调用栈。"
        );
        let _ = stderr.flush();

        wait_before_exit(ExitKind::Failure, is_bare_launch());
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_launch_is_detected_by_argument_count() {
        assert!(launched_without_arguments(&[]));
        assert!(launched_without_arguments(&["duplink.exe".to_string()]));
        assert!(!launched_without_arguments(&[
            "duplink.exe".to_string(),
            "--path".to_string(),
            "D:\\x".to_string()
        ]));
    }

    #[test]
    fn automation_is_never_blocked() {
        // 被重定向时（stdout 或 stdin 不是终端）绝不能等待，否则会卡死脚本。
        // is_interactive() 在测试环境下为 false，因此这里必然不等待。
        assert!(!should_wait(ExitKind::Failure, true));
        assert!(!should_wait(ExitKind::Success, true));
    }

    #[test]
    fn failure_hint_tells_user_to_read_first() {
        assert!(pause_hint(ExitKind::Failure).contains("先读完"));
        assert!(pause_hint(ExitKind::Success).contains("按回车"));
        assert!(pause_hint(ExitKind::Informational).contains("按回车"));
    }

    #[test]
    fn panic_hook_is_installable_and_does_not_hang() {
        // 钩子里的"等待回车"在非终端环境下必须立即返回，否则 panic 会挂住进程。
        let previous = std::panic::take_hook();
        install_panic_hook();
        let result = std::panic::catch_unwind(|| panic!("测试用的 panic"));
        std::panic::set_hook(previous);
        assert!(result.is_err());
    }
}
