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

维护者负责打 tag 与发布 Release，流程：

```bash
# 1. 更新 CHANGELOG.md，把 [未发布] 改成版本号与日期
# 2. 更新 Cargo.toml 的 version
cargo build --release
cargo test
git tag -a v0.2.3 -m "v0.2.3"      # 说明用文件传，避免 shell 引号问题
git push origin main --tags
gh release create v0.2.3 --title "..." --notes-file CHANGELOG.md \
   "target/release/duplink.exe#duplink.exe (Windows x64)"
```

---

## 提问

遇到不清楚的地方，开 issue 即可，不必先读源码。
