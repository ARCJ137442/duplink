# 贡献指南

感谢你愿意参与 duplink。这个工具会改动你磁盘上的文件，所以我们在流程上比一般小工具
更看重**可验证**与**可回退**。

---

## 开发环境

| 依赖 | 版本 |
|------|------|
| Rust | 1.80 或更高（当前在 1.93 上开发） |
| Cargo | 随 Rust 附带 |

无需其它依赖，也不需要 GUI 环境。

```bash
git clone https://github.com/ARCJ137442/duplink.git
cd duplink
cargo build --release
```

---

## 提交前必跑

```bash
cargo fmt --check              # 格式
cargo clippy --all-targets     # 静态检查（CI 用 -D warnings）
cargo test                     # 单元测试
cargo build --release
```

CI 会跑同样的四项。**格式与 clippy 的失败会直接挡住 PR**，请先在本地跑一遍。

---

## 代码风格

- 遵循 Rust 社区惯例；注释用中文，与现有代码保持一致
- **平台差异只允许出现在 `src/platform.rs`**。上层模块只依赖跨平台语义
  （例如"两个路径是否同一个文件"），不感知 inode / file index 的差别
- **破坏性副作用只允许出现在 `src/link.rs`**。其它模块只负责计算与报告
- 注释解释**为什么**，不解释代码在做什么。设计取舍、踩坑警示、边界条件写进注释；
  一次性的过程叙事不要写进代码
- 新增行为若能被纯函数表达，就写成纯函数并配单元测试

---

## PR 期待

一个好的 PR 应该包含：

1. **问题陈述**：改之前是什么样，出问题的是什么
2. **可验证的证据**：真实数据上的测量、或一条能复现的测试
   （本项目历史上多个 bug 都是靠"用另一套实现独立复算一遍"抓出来的）
3. **不破坏既有契约**：命令行参数、退出码、JSON 结构都是公开接口，改动需要说明
4. **CHANGELOG 条目**：新增 `CHANGELOG.md` 的 `[未发布]` 段落

### 关于改动破坏性操作

如果你要改 `src/link.rs` 里的执行序列，请先读 `docs/design.md` 的
「安全执行序列」一节 —— 那里的步骤顺序本身就是安全承诺，
调换顺序会削弱"建链失败时原始数据仍在盘上"的保证。

---

## 提交信息

沿用现有风格（祈使句、首行摘要、说明动机）：

```
fix(report): 修正 ×N 显示语义

×N 原先写的是"待转换数 + 1"，整组已链接的组会误显示为 ×1。
改为成员总数；混合组额外标注"N 份待转换 · M 份已是硬链接"。
```

---

## 发布

维护者负责打 tag。**打完 tag 不要再手动 `gh release create`** —— tag 推送后
GitHub Actions 的 `release.yml` 会自动构建三个平台的二进制并创建/更新 Release，
手动创建会和它抢同一个 release。

```bash
# 1. 更新 CHANGELOG.md：把 [未发布] 的内容改成版本号与日期
# 2. 更新 Cargo.toml 的 version
cargo build --release
cargo test
cargo fmt --check
cargo clippy --all-targets

# 3. 提交并打 tag
git add -A
git commit -m "release: v0.2.3"
git tag -a v0.2.3 -F RELEASE_NOTES.txt   # 说明用文件传，避免 shell 引号问题
git push origin main --follow-tags
```

tag 推送后 `release.yml` 会：

1. 在 Ubuntu / Windows / macOS 上分别构建
2. 收集 `duplink-windows-x86_64.exe`、`duplink-linux-x86_64`、`duplink-macos-arm64`
3. 创建或更新对应的 Release 并附上这三个文件

> 平台矩阵里的 `bin`（cargo 产出名，由包名决定）与 `asset`（发布名，必须唯一）
> 是两回事。早期版本把两者混用，导致 macOS 那份找不到文件、整个工作流失败。

---

## 提问

遇到不清楚的地方，开 issue 即可，不必先读源码。
