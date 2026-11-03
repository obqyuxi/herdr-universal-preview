# 技术决策

## D0 项目位置与仓库（M0）

- 使用当前目录 `~/Documents/github/herdr-universal-preview`，而不是任务书默认的 `~/Projects/...`。依据任务书 §00："如果当前目录已经是该项目，直接使用"。
- 本地 Git 已初始化；暂不创建远端仓库。

## D1 宿主扩展路线：固定提交 + 可重现补丁 + 独立原生 crate（M0/M1）

- **事实**：0.1.12 的 Mod API 没有任何原生视图接口。Issue #218 以 NOT_PLANNED 关闭，PR #235 是 Draft、未合并。
- **选择**：
  - 上游固定在 `v0.1.12`（`4aa3460`），与正式安装版本一致，方便对照行为。
  - 宿主改动只以 `patches/herdr-app/*.patch` 为准，由 `scripts/prepare-upstream` 以 `git am` 形式重建。
  - 原生渲染器放在本仓库的 `native/herdr-native-view` crate：只依赖 AppKit/WebKit/PDFKit/AVKit/QuickLookUI，不依赖 GPUI，坐标和规格逻辑可以单独做单元测试。
  - 补丁只做粘合：
    - 一个 `herdr` 模块组件 `nativeView`，加几个函数；
    - 一个帧末同步 canvas；
    - 开发版安全保护。
- **被否决**：
  - 直接套用 PR #235。它新增了 `Content::Web`，会让旧版整份布局解析失败（见 #218 讨论），也没有处理遮挡、焦点和裁剪。
  - 跟随 master。它和正式 0.1.12 的行为不一致，验收难以对照。
  - CEF 或独立浏览器进程。WKWebView 路线尚未被证伪，按任务书 §07，不在没有证据的情况下提出替代。

## D2 Mod 对原版的兼容（M1）

- Mod 用 `import * as herdr from "herdr"` 探测 `herdr.nativeView`。如果写成具名导入，原版缺少这个导出时整个 Mod 会加载失败。
- 布局只用 `{ "mod": ..., "props": ... }`。

## D3 测试隔离（M0）

- 测试 core 用独立的 `HC_SESSION`，以及 `XDG_STATE_HOME`、`XDG_CONFIG_HOME`；测试 App 用独立的 `HERDR_APP_HOME`、`HERDR_CORE_DIRS`、`HC_INSTALL_DIR`。同时清除继承来的 `HC_*` 变量。证据见 ENVIRONMENT.md。
- macOS 的 socket 路径上限是 104 字节，所以 XDG 目录经由短链接 `/tmp/hup-$UID` 指向项目内的 `.local`。
- 测试 App 使用独立的 bundle 名、id 和可执行文件名，Info.plist 的 `LSEnvironment` 中也写入隔离变量。即使用户从 Finder 双击启动，也不会落到正式 home。补丁中还会让开发版在没有 `HERDR_APP_HOME` 时拒绝启动（见 M1）。

## D4 工具链（M0）

- Rust 和 Zig 只安装在项目内，校验 sha256，不修改 PATH、profile 或 brew。
- 不安装 just：上游 just 配方的作用被 `scripts/build` 中直接调用 cargo 取代。

## D5 GUI 验收方式（用户确认，2026-11-02）

- 采用人工验收清单加宿主自检（`native.debug`）。不申请屏幕录制或辅助功能权限。
- Herdr 自带的 `shot` 只抓 GPUI 层，不作为原生视图证据。

## D6 ghostty 不生成 xcframework（M0，补丁 0001）

- **事实**：ghostty 在 lib-vt 模式下，只要 PATH 上有 `xcodebuild` 就会生成 xcframework（`vendor/ghostty/src/build/Config.zig:537-555`）。CLT 的 `/usr/bin/xcodebuild` 只是空壳，没有完整 Xcode 时执行就会失败。
- **选择**：在 `crates/ghostty-sys/build.rs` 中传入 `-Demit-xcframework=false`。Herdr 只链接 `libghostty-vt.a`，不受影响。
- **被否决**：安装完整 Xcode。体积大，属于系统级软件，需要用户同意，而且对本项目没有必要。
- **影响**：上游的 `tests/notices.rs` 会对 build.rs 的指纹报警。等 M1 新增依赖时，用 `cargo-about 0.9.3`（安装在项目内）配合上游 `scripts/notices.ts` 重新生成 notices。
