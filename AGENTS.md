# AGENTS.md

> 给在本仓库工作的 AI 协作者的规则手册。
> 人读的文档在 `README.md`（怎么用）与 `docs/design.md`（怎么做的）。

## 项目一句话

递归扫描目录，找出内容完全相同的重复文件，在用户确认后转换为硬链接，让多份副本只占一份磁盘空间。

## 命令速查

```bash
cargo build --release          # 构建（产物 target/release/duplink[.exe]）
cargo test                     # 单元测试
cargo clippy --all-targets     # 静态检查
cargo fmt                      # 格式化（提交前必须跑 fmt --check）
cargo run -- --path <目录> --dry-run --yes   # 本地试跑（--dry-run 不改文件）
```

## 硬边界（违反就是事故级）

1. **`src/link.rs` 是唯一允许改动磁盘数据的地方。** 其它模块只计算、只报告。
   不要把 `fs::rename` / `fs::remove_file` / `fs::hard_link` 散落到别的模块。
2. **执行序列的顺序不可调整。**
   `rename → hard_link → 校验 → 删除`，理由见 `docs/design.md`「安全执行序列」。
   任何一步失败必须能恢复原名，且**原文件内容字节级不变**。
3. **平台差异只允许出现在 `src/platform.rs`。** 上层只依赖跨平台语义
   （例如 `is_same_file(path_a, path_b) -> Option<bool>`）。
   注意 `None` 表示"无法判断"，**不得当作"不是同一个文件"**。
4. **删除只读文件前必须先清只读位。** Windows 上删除只读文件会失败；
   能力放在 `platform::clear_readonly`，Unix 上不改动权限。
5. **改动任何对外契约都要说明。** 命令行参数、退出码、JSON 结构都是公开接口。

## 模块地图

| 模块 | 职责 | 不该出现的东西 |
|------|------|----------------|
| `main.rs` | 薄编排：扫描 → 摘要 → 分组 → 报告 → 确认 → 转换 → 复核 | 算法细节、副作用 |
| `cli.rs` | 命令行参数契约（尺寸在边界就解析成字节） | 业务判断 |
| `model.rs` | 领域类型：FileEntry / DuplicateGroup / Digest / 扩展名判定 | I/O |
| `platform.rs` | 平台差异隔离：文件身份、只读位 | 业务概念 |
| `scan.rs` | 遍历、候选筛选、reparse point 统计 | 摘要计算、分组 |
| `digest.rs` | 并行 SHA-256 | 分组决策 |
| `group.rs` | 分组与主副本选择策略 | I/O（除身份比较） |
| `link.rs` | **唯一**有破坏性副作用的模块 | 报告渲染 |
| `report.rs` | 人类可读输出 + JSON 结构 | 副作用 |
| `confirm.rs` | 交互式选择语法解析 | 副作用 |
| `terminal.rs` | 退出行为（不闪退）、panic 诊断 | 业务概念 |

## 容易踩的坑

- **文件身份 vs 文件内容**：内容相同 ≠ 同一个文件。只有文件身份（inode / file index）
  能区分"两份独立数据"与"已共享的硬链接"。`same-file` 在 Windows 上走
  `GetFileInformationByHandle`；`std::os::windows::fs::MetadataExt` 的
  `file_index()` / `number_of_links()` 至今仍是 unstable，不能用。
- **`DirEntry::stat(follow_symlinks=False)` 在 Windows 上 `st_ino` 恒为 0**。
  写验证脚本时要用会打开句柄的 `os.stat(path)`，否则所有文件会被混成同一个 inode。
- **junction 不是 symlink**：Windows 的目录联接上，Python 的 `os.path.islink()` 返回
  `False`（会跟进），Rust 的 walkdir 则视其为 symlink（不跟）。两者遍历结果会差出
  整棵子树。
- **reparse point 不跟进是有意的**：可能越出扫描根、可能成环、可能重复计数。
  但遮蔽的文件数必须在报告里说明，否则"扫描了 N 个文件"会误导。

## 深入文档

| 想知道 | 读哪 |
|--------|------|
| 怎么用 | `README.md` |
| 怎么做的（执行序列、身份判据、主副本规则、reparse 策略） | `docs/design.md` |
| 改了什么 | `CHANGELOG.md` |
| 怎么参与 | `CONTRIBUTING.md` |
