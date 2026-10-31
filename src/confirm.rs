//! 交互式选择。
//!
//! 📌【2026-10-30】选择语法的解析与提示文本都收在本模块，`main` 只负责调用。
//! 新增一种选择方式时，改动范围限于这里。

use std::io::{BufRead, Write};

use crate::model::{is_editable, parse_size, DuplicateGroup};

/// 用户对"要处理哪些重复组"的选择。
#[derive(Debug)]
pub enum Selection {
    /// 全部处理。
    All,
    /// 全部跳过。
    None,
    /// 按下标处理（下标从 0 开始，对应报告中的第 1、2、…组）。
    Indices(Vec<usize>),
}

/// 解析选择表达式。
///
/// 支持的写法（可用逗号组合）：
/// - `all` / `a`：全部处理
/// - `none` / `n`：全部跳过
/// - `safe`：只处理不含"会被原地编辑"文件的组
/// - `size>10MB`：只处理单份大小超过阈值的组
/// - `1-5,8,12-`：按下标选择（从 1 开始）
pub fn parse_selection(input: &str, groups: &[DuplicateGroup]) -> Result<Selection, String> {
    // ⚠️ 去掉 BOM：Windows 终端把输入重定向到原生程序时可能带上 U+FEFF，
    // 不清理会让第一个 token 永远解析失败。
    let text = input
        .trim_matches(|character: char| character.is_whitespace() || character == '\u{FEFF}')
        .to_ascii_lowercase();
    if text.is_empty() {
        return Err("输入为空".to_string());
    }
    match text.as_str() {
        "all" | "a" | "全部" | "y" | "yes" => return Ok(Selection::All),
        "none" | "n" | "无" | "no" | "q" | "quit" => return Ok(Selection::None),
        _ => {}
    }

    let mut selected = Vec::new();
    for token in text.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        match token {
            "safe" | "安全" => {
                for (index, group) in groups.iter().enumerate() {
                    // 📌【2026-10-31】按组内**全部**成员判定，而不是只看待转换的成员。
                    // 报告里的警告也是按全部成员圈定的组；两处必须一致，
                    // 否则用户用 safe 规避的组与警告提示的组对不上。
                    let risky = group.members.iter().any(|member| is_editable(&member.path));
                    if !risky {
                        selected.push(index);
                    }
                }
            }
            other if other.starts_with("size>") => {
                let threshold =
                    parse_size(&other[5..]).map_err(|error| format!("阈值无效: {error}"))?;
                for (index, group) in groups.iter().enumerate() {
                    if group.size > threshold {
                        selected.push(index);
                    }
                }
            }
            other => {
                selected.extend(parse_range(other, groups.len())?);
            }
        }
    }

    selected.sort_unstable();
    selected.dedup();
    Ok(Selection::Indices(selected))
}

/// 解析 `1-5` / `8` / `12-` 形式的区间，转换为 0 起始下标。
fn parse_range(token: &str, total: usize) -> Result<Vec<usize>, String> {
    let parse_one = |value: &str| -> Result<usize, String> {
        value
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("无法解析下标: {value}"))
    };

    if let Some((start, end)) = token.split_once('-') {
        let start = if start.trim().is_empty() {
            1
        } else {
            parse_one(start)?
        };
        let end = if end.trim().is_empty() {
            total
        } else {
            parse_one(end)?
        };
        if start == 0 || start > end {
            return Err(format!("区间无效: {token}"));
        }
        let end = end.min(total);
        return Ok((start..=end).map(|value| value - 1).collect());
    }

    let value = parse_one(token)?;
    if value == 0 || value > total {
        return Err(format!("下标超出范围: {token}（共 {total} 组）"));
    }
    Ok(vec![value - 1])
}

/// 反复提示直到得到有效选择。
pub fn ask<W: Write, R: BufRead>(
    out: &mut W,
    input: &mut R,
    groups: &[DuplicateGroup],
) -> Result<Selection, String> {
    loop {
        write!(
            out,
            "\n请选择要转换的重复组 [all / none / safe / size>10MB / 1-5,8]: "
        )
        .map_err(|error| error.to_string())?;
        out.flush().map_err(|error| error.to_string())?;

        let mut line = String::new();
        let read = input
            .read_line(&mut line)
            .map_err(|error| format!("读取输入失败: {error}"))?;
        if read == 0 {
            return Ok(Selection::None);
        }
        match parse_selection(&line, groups) {
            Ok(selection) => return Ok(selection),
            Err(reason) => {
                writeln!(out, "  无效输入：{reason}").map_err(|error| error.to_string())?;
            }
        }
    }
}

/// 把选择解析为需要处理的组下标。
pub fn resolve(selection: &Selection, groups: &[DuplicateGroup]) -> Vec<usize> {
    match selection {
        Selection::All => (0..groups.len()).collect(),
        Selection::None => Vec::new(),
        Selection::Indices(indices) => indices
            .iter()
            .copied()
            .filter(|index| *index < groups.len())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Digest, FileEntry};
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn group(names: &[&str], size: u64) -> DuplicateGroup {
        let members = names
            .iter()
            .map(|name| FileEntry {
                path: PathBuf::from("/root").join(name),
                size,
                modified: SystemTime::UNIX_EPOCH,
                digest: Some(Digest::from_bytes([0u8; 32])),
            })
            .collect();
        DuplicateGroup::new(size, Digest::from_bytes([0u8; 32]), members)
    }
    fn groups() -> Vec<DuplicateGroup> {
        vec![
            group(&["a.pdf", "a(1).pdf"], 100),
            group(&["b.md", "b(1).md"], 200),
            group(&["c.pdf", "c(1).pdf"], 300),
        ]
    }

    #[test]
    fn keyword_selections() {
        let list = groups();
        assert!(matches!(
            parse_selection("all", &list).unwrap(),
            Selection::All
        ));
        assert!(matches!(
            parse_selection("none", &list).unwrap(),
            Selection::None
        ));
    }

    #[test]
    fn range_selection_is_one_based() {
        let list = groups();
        match parse_selection("1-2", &list).unwrap() {
            Selection::Indices(indices) => assert_eq!(indices, vec![0, 1]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn safe_excludes_editable_groups() {
        let list = groups();
        match parse_selection("safe", &list).unwrap() {
            Selection::Indices(indices) => assert_eq!(indices, vec![0, 2]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn safe_and_warning_agree_on_which_groups_are_editable() {
        // safe 排除的组必须与警告提示的组完全一致：用户用 safe 规避的，
        // 正是报告里点名的那批。
        let list = groups();
        let editable_groups: Vec<usize> = list
            .iter()
            .enumerate()
            .filter(|(_, group)| group.members.iter().any(|member| is_editable(&member.path)))
            .map(|(index, _)| index)
            .collect();
        match parse_selection("safe", &list).unwrap() {
            Selection::Indices(indices) => {
                let excluded: Vec<usize> = (0..list.len())
                    .filter(|index| !indices.contains(index))
                    .collect();
                assert_eq!(excluded, editable_groups);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn size_filter_selects_by_single_size() {
        let list = groups();
        match parse_selection("size>150", &list).unwrap() {
            Selection::Indices(indices) => assert_eq!(indices, vec![1, 2]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn out_of_range_is_rejected() {
        let list = groups();
        assert!(parse_selection("9", &list).is_err());
        assert!(parse_selection("0", &list).is_err());
    }

    #[test]
    fn byte_order_mark_does_not_break_parsing() {
        let list = groups();
        assert!(matches!(
            parse_selection("\u{FEFF}safe", &list).unwrap(),
            Selection::Indices(_)
        ));
        assert!(matches!(
            parse_selection("\u{FEFF}all", &list).unwrap(),
            Selection::All
        ));
    }
}
