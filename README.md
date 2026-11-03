# herdr-universal-preview

目标：在 Herdr 主窗口中打开网页、查看文件、播放音视频，同时保留现有的文件树、工作区和 Agent 终端。

**状态：开发中（M0/M1）。尚未安装到正式 Herdr，也没有可供日常使用的版本。** 当前进度见 `docs/PROGRESS.md`；哪些功能真正可用见 `docs/SUPPORT_MATRIX.md`（M1 起维护）。

本项目有两种交付形态，二者要区分清楚：

- **普通 Mod 版**：在原版 Herdr 上能实现的部分（文本、代码、Markdown 等）。
- **完整增强版**：个人构建的 `Herdr Preview Dev.app`，在固定的上游 v0.1.12 上打本仓库的补丁，提供同一窗口内的网页、PDF、Office 和音视频预览。

## 前提

需要 macOS（Apple Silicon）和 Command Line Tools。工具链只安装在项目内部，不改全局 PATH。

```sh
scripts/doctor                 # 只读检查
scripts/bootstrap-toolchain    # 在 .local/toolchain 安装 Rust 1.95.0 和 Zig 0.16.0
scripts/prepare-upstream       # 克隆 herdr-app v0.1.12，并应用 patches/herdr-app
scripts/build                  # 编译并打包 artifacts/Herdr Preview Dev.app
scripts/test                   # 静态检查和单元测试（加 --all 包含补丁与上游测试）
```

## 测试环境（与正式 Herdr 隔离）

```sh
scripts/dev isolation-check    # 证明测试 core 与正式 core 互不影响
scripts/dev core-up | core-down
scripts/dev app-up  | app-down # 隔离 home、core 和 session 中的 Herdr Preview Dev
scripts/dev status             # 测试环境状态和正式环境指纹
```

隔离方式和验证证据见 `docs/ENVIRONMENT.md`。安装、卸载和回退脚本在 M5 提供；在此之前，本项目不会写入正式 Herdr。

## 许可

Apache-2.0，见 `LICENSE`。第三方组件见 `THIRD_PARTY_NOTICES.md`。
