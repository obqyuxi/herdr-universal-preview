# 验收记录

状态只有四种：未执行、通过、失败、阻断。四类结果分开记录：静态检查、编译、单元测试、GUI 验收。宿主自检不等于 GUI 验收。

构建：Herdr Preview Dev `0.1.12-hup.<work 提交>`，由 v0.1.12 `4aa3460` 加 `patches/herdr-app/0001–0002` 构成。环境：macOS 26.6.2、arm64、64 GB。

## M1 原型：分层结果（2026-11-02）

| 层 | 内容 | 结果 | 证据 |
|---|---|---|---|
| 静态检查 | `scripts/test`（bash -n、JSON、clippy -D warnings、rustfmt） | 通过 | 命令输出 |
| 静态检查 | `herdr-app check mods/native-probe`：开发版和**正式 0.1.12** 都检查了（正式版检查时指定临时 home） | 通过 | 命令输出 |
| 编译 | `scripts/build`（cargo build --locked，打包并 ad-hoc 签名） | 通过 | `.local/logs/build-debug.log` |
| 单元测试 | native crate：7 项 | 通过 | `scripts/test` |
| 单元测试 | 上游 `cargo test --locked`，补丁版：258 项加集成测试，包括 notices | 通过 | `.local/logs/m1-upstream-test.log` |
| 补丁可重现 | 补丁导出后用 `git am` 重建 | 通过 | `scripts/prepare-upstream` |
| 宿主自检 | `tests/integration/native-selfcheck`：网页/PDF、动作白名单、导航、显隐 | 通过（13/13） | `artifacts/m1-selfcheck*.json*` |
| 宿主自检 | `tests/integration/native-selfcheck-media`：MP4 不自动播放、播放、拖动、暂停、静音、DOCX Quick Look、切换布局后销毁 | 通过（7/7） | `artifacts/m1-selfcheck-media.jsonl` |
| 宿主自检 | 命令面板打开时两个原生视图 `suppressed`/`hidden`，关闭后恢复，网页标题和 PDF 页码保留 | 通过 | 本文档与会话记录 |
| GUI 验收 | `tests/gui/M1_CHECKLIST.md` G1–G24（需要用户实际操作：看画面、听声音、输入法、复制粘贴等） | **未执行** | — |

### 开发中发现并修复的缺陷（由自检或日志发现）

1. 探针 canvas 为绝对定位但没有设 inset，于是落在静态位置（元素下方），导致尺寸为 0、视图始终隐藏。修复：设 `top_0/left_0` 并放到第一个子元素。
2. `browser.navigate` 之后，mod 重绘时又传入原来的 props，把页面重新加载回去。修复：mod 的 props 和当前导航到的内容分开保存。
3. 媒体加载时 RefCell 嵌套借用导致 panic，进程随之 abort。修复：`load_entry` 先结束借用，再保存 player。
4. `setVisible(true)` 只在下一帧生效，而通过控制命令调用时没有帧跟上。修复：调用时立即生效，`native.call` 之后请求重绘。

## P0 验收表（A01–A26）

本阶段只涉及 M1 原型。完整验收从 M2/M3 开始填写。A16、A17、A18 的原型级部分对应 M1 清单中的 G3–G5/G23、G19、G20–G21/G24，目前都是**未执行**。
