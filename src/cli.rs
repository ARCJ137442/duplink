//! 命令行参数契约。
//!
//! 📌【2026-10-30】尺寸在参数边界就被解析成字节数，下游只处理 `u64`，
//! 避免"同一份大小在多处用不同方式解析"这一重复真相源。

use std::path::PathBuf;

use clap::Parser;

use crate::model::{parse_size, SizeParseError};

/// 参数边界的尺寸解析：裸数字按字节解释，其余带 B/KB/MB/GB/TB 单位。
fn parse_size_arg(raw: &str) -> Result<u64, SizeParseError> {
    parse_size(raw)
}

/// duplink 命令行参数
#[derive(Parser, Debug)]
#[command(name = "duplink", version, about, long_about = None)]
pub struct Args {
    /// 扫描根目录（默认：可执行文件所在目录）
    #[arg(short, long, value_name = "DIR")]
    pub path: Option<PathBuf>,

    /// 只处理不小于该大小的文件，如 1MB / 500KB / 2GB
    #[arg(short, long, value_name = "SIZE", value_parser = parse_size_arg)]
    pub min_size: Option<u64>,

    /// 排除匹配的路径（glob，可重复）。不含 "/" 时同时按目录名匹配
    #[arg(short, long, value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// 扫描最大深度（相对根目录；0 表示只看根目录本身）
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,

    /// 并行计算摘要的线程数
    #[arg(long, value_name = "N")]
    pub jobs: Option<usize>,

    /// 预览模式：只报告，不做任何改动
    #[arg(short, long)]
    pub dry_run: bool,

    /// 跳过交互确认，直接处理全部重复组
    #[arg(short, long)]
    pub yes: bool,

    /// 以 JSON 输出结果（结果写到 stdout，进度写到 stderr）
    #[arg(long)]
    pub json: bool,

    /// 报告中最多列出的重复组数量（0 表示不限制）
    #[arg(long, value_name = "N", default_value_t = 200)]
    pub max_listed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_units() {
        assert_eq!(parse_size_arg("1024").unwrap(), 1024);
        assert_eq!(parse_size_arg("1KB").unwrap(), 1024);
        assert_eq!(parse_size_arg("1 MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size_arg("1.5GB").unwrap(), 1_610_612_736);
        assert_eq!(
            parse_size_arg("2tb").unwrap(),
            2 * 1024 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_size_arg("abc").is_err());
        assert!(parse_size_arg("10XB").is_err());
        assert!(parse_size_arg("").is_err());
    }
}
