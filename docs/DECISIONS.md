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

## D7 `nativeView` 是宿主组件，不是新的布局节点（M1）

- **实现**：`HostModule::component`。JS 端的写法是 `herdr.nativeView.new(id, props)`，与 `Editor.new` 一致，因为 gpui-kit 把组件包装成 `{ new }` 对象。
- **尺寸**：元素自己 `size_full`，宿主 materialize 不会给它套样式。
- **位置**：在 paint 阶段记录 `bounds ∩ content_mask`；窗口最后绘制的 canvas 统一换算到 NSView 坐标（y 翻转、`convertRect:toView:`）。
- **生命周期**：
  - mod copy 消失时销毁视图；
  - copy 不在屏幕上时隐藏，媒体同时暂停；
  - mod 的 props 变化时，在同一视图中重新加载，加载代次加 1；
  - 视图内导航不改变 mod 的 props。

## D8 遮挡协调：第一版让位并保留状态（M1）

- 浮层的矩形无法枚举，见 `window.rs` 的 `render_overlays`。只要存在非 toast 浮层（dialog、popup、window）或正在拖动 pane，就隐藏所有原生视图。NSView 不销毁，状态保留；mod 的占位文字说明"暂时遮挡"。
- **已知限制**：
  - toast 会被原生视图盖住；
  - mod 自己的 GPUI popover 也会被盖住（mod 可以自行 `setVisible`）；
  - float 尚未处理。
- **后续**：考虑在 `render_overlays` 中捕获矩形，改为 mask 挖洞。

## D9 焦点（M1）

- 加一个本地事件监视器（只监视 mouseDown）：点击落在所有原生视图之外，而当前 first responder 在某个原生视图内时，把 first responder 交还 GPUIView。
- mod 也可以调用 `blur` 主动交还焦点。

## D10 侧边浏览器改用 Chromium 内核（用户决定，2026-11-02）

- **决定**：用户在了解代价（应用体积增加数百 MB、需要长期跟进安全补丁、开发量按天计）后，选择"直接换 Chrome 内核"。需要的功能：查找/缩放/打印、下载/历史、清除浏览数据、导入 Chrome 登录、扩展程序。
- **路线**：CEF 156，与本机 Chrome 156.0.8078.80 同一代；Rust 绑定使用 tauri-apps/cef-rs `156.3.0+156.0.15`（`224a785c`），二进制导出到 `.local/cef`。
- **集成要点（待逐项实测）**：
  1. GPUI 的 `GPUIApplication` 需要实现 `CrAppControlProtocol`（`isHandlingSendEvent` / `setHandlingSendEvent:`），给 vendored gpui-pre-macos 打补丁。
  2. 使用 external message pump：`on_schedule_message_pump_work` 加后备定时器调用 `do_message_loop_work`，不接管 GPUI 的 run loop。
  3. 浏览器视图以 `SetAsChild` 挂到窗口内容视图，复用现有的帧同步、遮挡和焦点逻辑（新增 kind `chromium`）。
  4. 打包：`Contents/Frameworks/Chromium Embedded Framework.framework`，以及 Helper、GPU、Renderer、Plugin、Alerts 五个 helper app。
  5. 资料目录：开发版使用项目内 `.local/chromium-profile`，不使用、也不复制日常 Chrome 的资料目录。
- **用户隐私**：导入 Chrome 登录要读取用户 Chrome 的 Cookie/密码，并经过钥匙串授权。真正执行前要再次单独确认。
- **已知不确定**：CEF 对 Chrome 扩展支持有限；Chrome style 浏览器在 macOS 上能否挂到原生父视图，需要实测。
- **WebKit 版保留**：作为原版 Herdr 不可用时的后备，以及 Chromium 失败时的回退。

## D11 日用安装：独立的 Herdr Plus，与官方 Herdr 并列（用户批准，2026-11-02）

- 安装到 `~/Applications/Herdr Plus.app`，共用 `~/.config/herdr-app` 和官方 herdr-core。只新增 3 个 mod 和 1 个布局，并记录带指纹的清单。官方 Herdr.app 没有被覆盖，可以用 `scripts/rollback` 回退。
- 不自动更新：官方升级后，需要在新的上游上重新打补丁、编译、安装。

## D12 开源分发方向（待用户决定）

- 现状：mod 是普通插件，但窗口内嵌原生内容依赖宿主补丁，所以单装插件在原版上做不到预览和浏览器。
- 选项：
  1. 发布已签名、已公证的 Herdr Plus，加一键安装脚本（需要 Apple Developer ID）；
  2. 向上游提交窄接口 `nativeView`；被接受后就是纯 mod 安装（WebKit 版的接受可能性更高）；
  3. 一键脚本从源码编译。
- 建议 1 和 2 并行。

## D13 日用版改名为 DD-up，换用自己的图标（用户决定，2026-11-03）

- 名称集中在 `brand.env`：`APP_NAME=DD-up`、`APP_EXE=DD-up`、`APP_ID=dev.ddup.app`、`LAYOUT=dd-up`。`scripts/build --daily` 和 `scripts/lib/install.py` 都从这里读取。
- 旧的 Herdr Plus 安装已经按清单卸载（它在日常环境里从未打开过，没有浏览器资料），之后再装 DD-up。
- 图标：`assets/icon/logo.svg`（矢量源文件，由 Claude 设计：两块"D"形面板代表终端和预览，右侧那块抬高表示"up"），用 `scripts/make-icon`（项目内的 resvg 0.49.0 加系统 iconutil）生成 `assets/icon/AppIcon.icns`。两个文件都提交到仓库，构建时不需要 resvg。
- 只影响日用版；开发版仍然叫 Herdr Preview Dev，用官方图标。首次运行时缺少 core 才出现的设置窗口里，内置图片和标题仍然是 Herdr，没有打补丁。
- 签名：本机临时签名（ad-hoc），本机编译且没有隔离属性，自己使用不需要 Apple 开发者账号。发给别人下载时的签名问题留到 D12 再定。

## D14 开源发布（用户授权，2026-11-03）

- 公开仓库：https://github.com/obqyuxi/herdr-universal-preview（用户事先建好并设为 `origin`），Apache-2.0。只推送了 `main`。
- 提交作者统一改为 `obqyuxi <29856194+obqyuxi@users.noreply.github.com>`，避免暴露本机用户名和主机名；改写前的本地历史保留在分支 `backup/pre-identity-20261103`（未推送）。
- 推送前从全新 clone 按 README 完整走了一遍：bootstrap 815 s，应用补丁后 903 s，`build --daily` 完成时共 1454 s，得到 `DD-up.app`。
- 官方规范（`CONTRIBUTING.md`、App 自带 `AGENTS.md` 的"Layouts and mods other people made"）：官方仓库里自带的 mod 和布局只接受修复；个人做的放到自己的公开仓库，加 `RECIPE.md`、`preview.png` 和 topic `herdr-layout`/`herdr-mod`；缺少 API 时提"A mod can't do this yet" issue，大的改动先在 issue 里讨论。
- 因此不向官方提交 mod 或布局 PR，而是提 issue 请官方加原生视图元素：已发布为 https://github.com/herdrdev/herdr-app/issues/425（obqyuxi，2026-11-03，正文即 `docs/upstream/issue-native-view.md`）。暂不打 `herdr-layout` topic：在官方 Herdr 上这些 mod 显示不出预览和浏览器，会误导别人。

## D15 应用列表里出现多个 DD-up（2026-11-03 修复）

- 现象：用户的"应用程序"列表里出现 4 个 DD-up。分别是安装好的 DD-up.app、仓库里 `artifacts/DD-up.app` 这份构建产物，以及 `DD-up Helper (GPU)` 和 `DD-up Helper (Plugin)` 两个辅助程序。
- 原因：
  1. 安装时先复制到临时名 `DD-up.app.hup-tmp-<pid>`。这个名字没有 `.app` 后缀，Spotlight 把它当成普通文件夹建索引，于是把里面的 Helper 当成独立应用收录；改名之后，这些索引仍然留着。
  2. `artifacts/` 位于 Documents 之下，构建产物同样会被收录。
- 修复：
  1. 临时名和旧版本名改为隐藏且保留后缀（`.DD-up.hup-tmp-<pid>.app`）。
  2. 构建产物统一放进 `artifacts/apps.noindex/`，Spotlight 不索引 `.noindex` 目录。
  3. 用 `lsregister -u` 注销旧路径和辅助程序的注册，然后重新安装。之后 Spotlight 和 LaunchServices 里都只剩 `~/Applications/DD-up.app`。

## D16 基于用户自己的布局生成 DD-up 布局（2026-11-03）

- 现象：切到 `dd-up` 布局后，用户今天装的 space-folders 侧边栏不见了。`dd-up` 是照官方 `main` 做的，而用户平时用的是 `main-folders`。数据都在，切回 `main-folders` 就恢复了。
- 新增 `scripts/install --layout-from NAME`，在用户的布局上只做三处修改，生成 `dd-up-NAME`；原文件不动，生成的布局计入清单（`bases` 字段），重新安装时会再生成一次。用户已经切换到 `dd-up-main-folders`。
- 顺带修了两处：
  - 指纹计算忽略 mod 目录下的 `*.d.ts`。Herdr 每次启动都会按自己的版本重写这些文件，否则重新安装时会被误判成"用户改过"。真实清单已备份并刷新，刷新前确认过除 `.d.ts` 以外的内容与仓库一致。
  - "是否在运行"只检查要被替换的那一份 App，在临时目录演练时不会被正在运行的日用版挡住。

## D17 只保留一个 dd-up 布局；浏览器不再显示内部错误（2026-11-03）

- 设置里出现了 `dd-up` 和 `dd-up-main-folders` 两个布局。改为只生成一个 `dd-up`：基础布局由 `--layout-from` 选择（清单里的 `base` 字段，兼容旧的 `bases`），以前安装留下、这次不再生成的项会自动删除（改动过的会保留）。App 的构建没变时跳过替换，因此 DD-up 开着也能更新 mod 和布局。
- 浏览器：还没打开过网页时点 ⋯，菜单会读取缩放级别，把宿主报错"no native view"直接显示了出来。现在没有页面时不调用宿主，调用失败只写进日志、不显示给用户；只有网页本身加载失败时才提示。按用户要求去掉了"输入网址，回车打开……"这类提示文字。
- `chromium-selfcheck` 改为等页面加载完成（最多 30 s）再检查，不再固定 sleep。之前在网速慢时，"back"一项会误报失败。
