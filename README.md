# duplink

> **找出所有内容相同的文件，让它们只占一份磁盘空间。**
>
> Find byte-identical files and merge them into hard links — one copy on disk, many paths to it.

[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![release](https://img.shields.io/github/v/release/ARCJ137442/duplink.svg)](https://github.com/ARCJ137442/duplink/releases/latest)
[![rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/tools/install)

`duplink` 递归扫描一个目录，找出**内容完全相同**的重复文件，列清单让你确认，
然后把它们转换成**硬链接** —— 磁盘上只剩一份数据，但每个路径照常能用。

```
before:  /Music/特殊/歌.flac        64 MB  ┐
          /Music/CloudMusic/歌.flac   64 MB  ┤ 三份独立数据，192 MB
          /Music/Telegram/歌.flac     64 MB  ┘

after:   /Music/特殊/歌.flac        64 MB  ┐
          /Music/CloudMusic/歌.flac   ──┐   │ 一份数据，64 MB
          /Music/Telegram/歌.flac     ──┴   ┘ 三个路径指向同一份数据
```

---

## 目录

- [这解决什么问题](#这解决什么问题)
- [快速上手](#快速上手)
- [安全说明](#安全说明)
- [和其它工具的区别](#和其它工具的区别)
- [命令行参数](#命令行参数)
- [交互式选择](#交互式选择)
- [常见场景](#常见场景)
- [常见问题](#常见问题)
- [已知限制](#已知限制)
- [工作原理](#工作原理)
- [项目结构](#项目结构)
- [开发与贡献](#开发与贡献)

---

## 这解决什么问题

下面这些场景，你会反复遇到同一份文件躺在多个地方：

| 场景 | 典型表现 |
|------|----------|
| **聊天软件接收目录** | 微信/QQ 收到同名文件会自动加 `(1)`、`(2)`，攒着攒着就是几十份 |
| **下载文件夹** | 同一个压缩包、镜像、固件下载了好几遍 |
| **音乐/视频库** | 同一个专辑在「CloudMusic」「Telegram导入」「特殊」各存一份 |
| **跨目录复制** | 同一份论文，工作目录一份、归档目录一份 |
| **备份盘** | 主盘和备份盘各一份 —— 注意这**不能**用硬链接（跨卷） |

它们的内容一模一样，却各占一份磁盘。`duplink` 把它们合并成一份数据。

**实测**：一棵 55.63 GB 的音乐库（11049 个文件），处理后占用降到 44.01 GB ——
**硬链接省下 11.62 GB**，文件数量一个没少。

---

## 快速上手

### 1. 拿一个可执行文件

**Windows**：从 [Releases](https://github.com/ARCJ137442/duplink/releases/latest) 下载
`duplink-windows-x86_64.exe`（改名成 `duplink.exe` 也行），单文件，无需安装、无需任何依赖。

**Linux / macOS**：同一页面上有 `duplink-linux-x86_64` 与 `duplink-macos-arm64`；
也可以从源码编译（见下一节），产物在 `target/release/duplink`。

### 2. 先预览，不改任何东西

```bash
duplink.exe --path D:\你的目录 --dry-run
```

它会列出所有重复组、每组保留哪一份、可回收多少空间。**这一步不动你的文件。**

### 3. 确认后执行

```bash
duplink.exe --path D:\你的目录
```

程序会列出清单并停下来问你处理哪些组。输入 `all` 全部处理，或用下标挑几组
（详见[交互式选择](#交互式选择)）。

也可以完全无人值守：

```bash
duplink.exe --path D:\你的目录 --yes
```

### 或者：把 exe 放进目标目录再双击

程序默认扫描**自己所在目录**。把 `duplink.exe` 复制到想清理的目录里双击即可，
它会扫那个目录、做完停下来等你按回车关窗（不会闪退）。

---

## 安全说明

**duplink 不删除任何文件内容。** 请先理解它在做什么：

1. 它把副本路径**替换**成指向主副本的硬链接 —— 磁盘上的数据从 N 份变成 1 份
2. 在建链并校验通过之前，原始数据始终**独占存在**于盘上；任何一步失败都会自动回滚，
   **原文件内容字节级不变**
3. 转换完成后会重新校验每个路径，确认它确实与主副本共享同一份数据

### 你需要知道的唯一风险

硬链接之后，多个路径**指向同一份数据**。用编辑器**原地保存**其中一个（"保存"而不是
"另存为"），其余路径的内容会跟着变。图片、音频、视频、压缩包、PDF 等二进制文件不受影响。

程序会在报告里标出受影响的文本/代码类组，可以用 `safe` 选项整体排除它们。

### 想撤销？

复制一份回去就行 —— 用**复制**（copy）而不是移动：

```bash
copy D:\目录\主副本.txt D:\目录\另一份.txt   # Windows，两个文件从此互不相干
```

注意：**删掉硬链接本身不会丢数据**，只要还有任何一个链接指向它。

---

## 和其它工具的区别

| 工具 | 定位 |
|------|------|
| `jdupes` / `fdupes` | 以**报告**重复文件为主，擅长找，不负责合并 |
| `rmlint` | 功能面更广（含 GUI），也能建链接，但依赖更多，Windows 上支持一般 |
| **`duplink`** | **只做一件事**：把重复文件合并成硬链接 |

`duplink` 的取舍：

- **只做硬链接这一件事**，不做 GUI、不做标签、不做删除
- **中文交互界面**，提示语说清每个组发生了什么
- **安全序列做得扎实**：改名 → 建链 → 校验 → 删除，失败自动回滚，转换后复核
- **能识别"已经是硬链接"**，重复运行不会重复处理
- **提供 JSON 输出**，便于接入你自己的脚本或报表

---

## 命令行参数

```
duplink [选项]

  -p, --path <DIR>       扫描根目录（默认：程序所在目录）
  -s, --min-size <SIZE>  只处理不小于该大小的文件，如 1MB / 500KB / 2GB
  -e, --exclude <GLOB>   排除匹配的路径，可重复使用
      --max-depth <N>    扫描最大深度（0 = 只看根目录）
      --jobs <N>         并行计算摘要的线程数（默认 = CPU 核数）
  -d, --dry-run          预览模式：只报告，不做任何改动
  -y, --yes              跳过交互确认，直接处理全部
      --json             以 JSON 输出结果（JSON 到 stdout，人类信息到 stderr）
      --max-listed <N>   报告中最多列出几组（0 = 不限制，默认 200）
  -h, --help             显示帮助
  -V, --version          显示版本
```

### 排除规则

模式同时匹配「相对扫描根目录的完整路径」和「路径中的任意一段」：

| 模式 | 效果 |
|------|------|
| `target` | 排除任意层级的 target 目录 |
| `*.tmp` | 排除任意层级的 .tmp 文件 |
| `sub/*.tmp` | 只排除 sub 目录第一层的 .tmp 文件 |

---

## 交互式选择

列出重复组之后，程序会停下来问你要处理哪些：

```
请选择要转换的重复组 [all / none / safe / size>10MB / 1-5,8]:
```

| 输入 | 含义 |
|------|------|
| `all` / `a` / `全部` | 处理全部重复组 |
| `none` / `n` / `无` | 全部跳过 |
| `safe` | 只处理**不含**文本/代码类文件的组 |
| `size>10MB` | 只处理单份大小超过阈值的组 |
| `1-5,8,12-` | 按下标选择（从 1 开始，`12-` 表示到末尾） |

以上可用逗号组合，例如 `1-5,safe`。

### 输出里那些标记

```
     3. [ 83.38 MB ×2] Kevin MacLeod - Peace of Mind.flac
        ⭐ 保留 Music\CloudMusic\Kevin MacLeod - Peace of Mind.flac
           → 转换 Music\Telegram导入\Kevin MacLeod - Peace of Mind.flac
```

| 标记 | 含义 |
|------|------|
| `×N` | 组内**成员总数** |
| `⭐ 保留` | 主副本，它继续持有这份数据 |
| `→ 转换` | 会被替换成指向主副本的硬链接 |
| `= 已链接` | 已经是硬链接，无需处理 |

---

## 常见场景

### 清理聊天软件接收目录

```bash
duplink.exe --path "D:\xwechat_files\...\msg\file"
```

微信收到同名文件会自动加 `(1)`、`(2)`，这里最容易攒出大量重复。
**建议先跑 `--dry-run` 看一眼**，这些目录里通常没什么需要小心的文本文件。

### 只清理大文件

```bash
duplink.exe --path D:\Downloads --min-size 100MB
```

小文件省不了多少空间，先看大的。

### 跳过某个目录

```bash
duplink.exe --path D:\Media --exclude "*.tmp" --exclude ".agents"
```

### 定期跑一次

```bash
duplink.exe --path D:\Media --yes >> D:\logs\duplink.log 2>&1
```

已经是硬链接的会被自动跳过，不会重复处理。
**注意**：带参数运行时程序不会等待回车，所以可以安全地放进计划任务。

### 接入你自己的报表

```bash
duplink.exe --path D:\Media --dry-run --yes --json > report.json
```

`--json` 时 JSON 走 stdout、人类信息走 stderr，两边互不干扰。

---

## 常见问题

<details>
<summary><b>它会删我的文件吗？</b></summary>

不会删除任何**内容**。它把副本路径替换成硬链接，磁盘上的数据从 N 份变成 1 份。
建链失败时原文件会被完整恢复，工具会报告错误并继续处理其它组。
</details>

<details>
<summary><b>硬链接和复制有什么区别？</b></summary>

复制 = 两份独立数据，各占一份空间，改一个不影响另一个。
硬链接 = 一份数据，多个路径，改一个**所有**路径都变。

对音乐库、下载目录这种"文件不会改"的地方，硬链接省空间又没副作用。
对会反复编辑的文本文档，要留意上面的「原地编辑会互相影响」。
</details>

<details>
<summary><b>转换后我还能改文件吗？</b></summary>

能，但要看是什么文件：

- **二进制**（图片、音频、视频、压缩包、PDF）：随便改，不影响别的路径
- **文本/代码**：用「另存为」没问题；用「保存」（原地覆盖）会让其它路径一起变。
  程序会标出这类组，`safe` 选项可以整体跳过它们
</details>

<details>
<summary><b>支持 macOS / Linux 吗？</b></summary>

代码里两条路径都实现了（Windows 用卷序列号+文件索引号，Unix 用 device+inode）。
**但只在 Windows 上做过充分实测**，Unix 路径尚未在真实系统上验证过，
欢迎 issue 反馈。
</details>

<details>
<summary><b>为什么有些文件被跳过了？</b></summary>

报告里会说明原因：

- **空文件** —— 不占空间，没有去重价值
- **符号链接 / 目录联接** —— 不跟进，避免越出扫描根、成环、重复计数。
  但报告会列出它们**遮蔽了多少文件**，让你知道哪些内容没进扫描范围
- **小于 `--min-size`** —— 阈值过滤
- **读取失败** —— 文件被其它程序独占锁定
</details>

<details>
<summary><b>文件的时间戳变了？</b></summary>

是的。硬链接共享同一份数据，所有路径的修改时间会统一成主副本的时间。
这是硬链接的固有性质，无法避免。
</details>

<details>
<summary><b>能在不同盘符之间合并吗？</b></summary>

不能。硬链接不能跨卷。主副本与副本必须在同一个卷上，否则建链会失败 ——
工具会报告错误并把原文件完整恢复。
</details>

<details>
<summary><b>扫描很慢怎么办？</b></summary>

扫描本身很快（只读元数据）。耗时的是给"尺寸重复"的候选算 SHA-256，
这部分是并行的。磁盘慢就加 `--jobs` 调线程数，或用 `--min-size` 减少候选。

另外用 `--exclude` 排除你根本不关心的目录（比如 `node_modules`、缓存）能显著减少候选。
</details>

<details>
<summary><b>会重复处理吗？</b></summary>

不会。工具用操作系统的**文件身份**（不是文件内容）判断"这两个路径是不是同一份数据"，
已经共享数据的成员会被标成 `= 已链接` 并跳过。跑第二遍你会看到「待转换 0 份」。
</details>

<details>
<summary><b>双击运行窗口一闪就没了？</b></summary>

那是旧版本（≤ v0.1.0）的行为。v0.1.1 起，双击运行时程序会停在
「按回车键关闭窗口…」，输出不会消失。崩溃时也会打印可读诊断而不是静默退出。
</details>

---

## 已知限制

- **硬链接不能跨卷** —— 主副本与副本必须在同一个盘符
- **时间戳会统一** —— 所有路径共享主副本的修改时间
- **原地编辑会传播** —— 文本/代码文件用编辑器"保存"而非"另存为"时，其它路径一起变
- **符号链接与目录联接不参与去重** —— 避免越出扫描根、成环与重复计数（报告会说明遮蔽量）
- **空文件跳过** —— 不占空间，无去重价值
- **只读属性会自动清除** —— Windows 上删除只读文件必须先清只读位
- **Unix 未经真实验证** —— 只在 Windows 上充分实测

---

## 工作原理

```
扫描 → 按尺寸筛候选 → 并行 SHA-256 → 聚合重复组 → 报告 → 你确认 → 转硬链接 → 复核
```

摘要只对"尺寸重复"的候选计算，所以读盘量与文件**总数**无关，只与**候选数**有关。

转换的核心是一段**顺序即安全承诺**的序列：

```
1. rename(副本) → 临时名    原始数据仍在盘上，任何失败都可原样恢复
2. hard_link(主副本, 副本)   两个路径共享同一份数据
3. 校验：确实是同一份数据    不一致立即撤销
4. 清只读位 → 删除临时文件   到这一步占用才真正下降
```

深入原理（文件身份判据、主副本选择规则、reparse point 策略、性能特征、盲审验证方法）
见 **[docs/design.md](docs/design.md)**。

---

## 项目结构

```
duplink/
├── src/
│   ├── main.rs      # 薄编排：扫描 → 摘要 → 分组 → 报告 → 确认 → 转换 → 复核
│   ├── cli.rs       # 命令行参数契约
│   ├── model.rs     # 领域类型：FileEntry / DuplicateGroup / Digest
│   ├── platform.rs  # 平台差异隔离：文件身份比较、只读位处理
│   ├── scan.rs      # 目录遍历、候选筛选、reparse point 统计
│   ├── digest.rs    # 并行 SHA-256
│   ├── group.rs     # 分组与主副本选择策略
│   ├── link.rs      # 硬链接执行（唯一有破坏性副作用的模块）
│   ├── report.rs    # 报告渲染（人类可读 + JSON）
│   ├── confirm.rs   # 交互式选择
│   └── terminal.rs  # 退出行为（不闪退）、panic 诊断
├── docs/design.md   # 设计说明
├── .github/workflows/
│   ├── ci.yml       # 格式 / clippy / 测试 / 构建
│   └── release.yml  # 打 tag 后自动构建并发布各平台二进制
├── AGENTS.md        # 给 AI 协作者的规则手册
├── CHANGELOG.md
├── CONTRIBUTING.md
├── LICENSE
├── Cargo.toml
└── README.md
```

---

## 开发与贡献

需要 Rust stable（开发环境为 1.93）。

```bash
cargo build --release
cargo test              # 单元测试
cargo clippy --all-targets
cargo fmt --check
```

- 怎么参与：[CONTRIBUTING.md](CONTRIBUTING.md)
- 改了什么：[CHANGELOG.md](CHANGELOG.md)
- 给 AI 协作者：[AGENTS.md](AGENTS.md)

---

## 许可证

[MIT](LICENSE) © 2026 ARCJ137442
