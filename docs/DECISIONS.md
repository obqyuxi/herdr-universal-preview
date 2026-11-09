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

## D18 浏览器改成 Codex 式：卡片菜单浮在网页上、常用网站页、搜索引擎、无页面也能导入 Cookie（2026-11-03）

- 用户要求对照 Codex 的浏览器（不包括"工具"栏）。
- 宿主新增 `setHoles`（所有类型的视图都支持）：每个原生视图外面多了一层 `HoleHost` 容器，按 mod 给的矩形（可贴右边缘、带圆角，最多 8 个）挖洞。
  - 挖洞用 CAShapeLayer 的奇偶填充遮罩；洞里由 `hitTest:` 返回空，点击落到下层 Herdr 画的卡片上。
  - 点网页其他位置时上报 `view.click`，mod 收到后关掉菜单，行为和弹出框一样。
  - 只调用系统 API（CoreGraphics 的 C 函数、按名字取 CAShapeLayer 类），没有新增依赖，`Cargo.lock` 和第三方声明都不变。
- 自检里的 `state` 多了 `holes`、`masked` 和 `holeClicksReachHerdr` 三项。最后一项按窗口真实的 hit test，检查洞中心的点击会不会落到 Herdr 上，用来替代截图（截图要屏幕录制权限，不申请）。
- `browser.importChromeCookies` 在还没有页面时也能用：宿主会先启动 Chromium，结果仍然通过事件返回。
- mod 侧：
  - 地址栏输入文字时按选定的搜索引擎搜索（Google、必应、百度、DuckDuckGo，默认 Google）。
  - 新标签页是"常用网站"（按历史记录统计访问次数），用首字母色块，因为宿主还拿不到网站图标。
  - 菜单项按 Codex 的顺序排列：在页面中查找、打印、缩放、导入 Cookie、下载、历史记录、清除浏览数据、浏览器设置。
- 验证：
  - 原生单元测试 11 项通过。
  - 隔离环境里的自检全部通过：Chromium 6/6、网页与 PDF 13/13、音视频与 Quick Look 7/7；挖洞后遮罩生效，洞中心的点击落到 Herdr；非法参数会被拒绝。
  - 卡片的外观和点击需要用户在真实界面上确认。真实的 Chrome 导入会弹出钥匙串授权，没有自动测试。

## D19 暂时撤回官方 issue（用户决定，2026-11-03）

- 用户要求等功能打磨完善后再提。已经用 obqyuxi 账号把 herdrdev/herdr-app#425 关闭为 not planned，没有留言；普通用户删不掉 issue，只能关闭。
- 重新提交时，以 `docs/upstream/issue-native-view.md` 为底稿，并补上 `setHoles`（菜单卡片浮在网页上）这类新能力。

## D20 浏览器 UI 打磨（/impeccable polish，2026-11-03）

- 新增按键命令 `browser.menu`、`browser.panel:<卡片名>`、`browser.close`、`browser.address`、`browser.find`、`browser.home`，可以在 keys.json 里绑定快捷键。测试时用 `herdr-app do browser.panel:history` 打开某张卡片，再用 `shot` 截图；卡片是 GPUI 画的，能截进去，网页本身截不到。
- 根据截图逐项修正：
  - 文字符号（← → ↻ ⌂ ⋯ ‹ ✕ ✓）全部换成 Lucide 线条图标（与 Herdr 同一套，ISC 许可，见 THIRD_PARTY_NOTICES）。
  - 卡片改用 Herdr 菜单的参数：`t.popover` 背景，32px 行高，13px 字号，7px 圆角。
  - 卡片高度按内容计算，历史记录和下载随条目增加（最多 8 行，再多就滚动）。
  - 中文说明拆成短句，每句一行，避免标点出现在行首。
  - 历史记录同一网页只列一次。
  - 地址栏改用 Herdr chrome 的浅色底，聚焦时换成强调色描边。
  - 菜单里只有需要确认的项（导入 Cookie…、清除浏览数据…）带省略号。
- 限制：洞只显示卡片自身，无法带出卡片外的柔和阴影，层次只能靠描边和 popover 底色。
- 验证：
  - 每张卡片在隔离环境里截图检查过（`.local/ui-*.png`，不入库）。
  - impeccable detect 没有发现问题。
  - Chromium 自检 6/6 通过；打开历史卡片时有遮罩，洞内点击落到 Herdr。

## D21 卡片阴影、网站图标、加载进度、快捷键；测试构建改用模拟钥匙串（2026-11-03）

- 阴影：`HoleHost` 给每个洞叠加一层 CALayer 阴影（`shadowPath` 与洞的圆角矩形相同，不透明度 0.18，半径 14，向下偏移 6）。洞本身被遮罩挖空，阴影只落在卡片外的网页上。state 新增 `shadow` 字段。
- 网站图标：
  - Chromium 报告 favicon 地址后，用 `download_image`（64px）下载，转成 PNG，存到 `<app home>/data/browser/favicons/<host>.png`。文件名只取纯主机名，见 `favicon_host` 及其测试。
  - mod 记下 host 到路径的对应关系，常用网站页和历史记录用 `image()` 显示。
  - 注意 cef-rs 的坑：借用的 `CefStringList` 调 `clone()` 或转成 `*const` 都会得到空列表，必须用 `std::mem::take` 取出原包装再迭代。
- 加载进度：转发 `on_loading_progress_change`，事件为 `load.progress`，state 新增 `progress`。工具栏底边显示 2px 强调色进度条，用绝对定位，不会挤动布局。
- 快捷键：
  - 网页获得键盘焦点时，`on_pre_key_event` 拦截 ⌘L、⌘F、⌘R、⌘[、⌘]、⌘=、⌘-、⌘0 交给 mod 处理；Esc 也通知 mod，同时照常传给网页。
  - 焦点在 Herdr 一侧时，由 mod 的 `on_key_down` 处理 ⌘L、⌘F、Esc。
  - ⌘L 和 ⌘F 会先把键盘焦点交回 Herdr，再聚焦地址栏或查找框。
- **钥匙串（本轮查到的根因）**：CEF 在 macOS 上用固定名称的钥匙串条目 "Chromium Safe Storage" 加密 Cookie，名称不能改（CEF 二进制里只有 `use-mock-keychain` 开关）。每次重新编译，临时签名都会变，macOS 就要重新询问能否读取该条目。没人回答之前，所有网络请求都卡在 0.1。
  - 测试构建（`hup_isolate` 和开发版的 LSEnvironment）设置 `HUP_CHROMIUM_MOCK_KEYCHAIN=1`，即 `--use-mock-keychain`，不再弹窗。
  - 日用版怎么处理，等用户决定。
- 验证：
  - 原生单元测试 12 项通过；Chromium 6/6、网页与 PDF 13/13、音视频与 Quick Look 7/7 全部通过。
  - 4 个网站的图标保存成功，图标、历史记录和加载进度条都截图看过；impeccable detect 没有发现问题。
  - 快捷键需要真实按键才能触发，没有自动测试。

## D22 内嵌浏览器默认不用钥匙串（用户选 A，2026-11-03）

- 那个 "Chromium Safe Storage" 条目是 agent-browser 下载的 Google Chrome for Testing（`~/.agent-browser/browsers/chrome-149…`）在 2026-06-26 建的。CEF 用的是同一个写死的名字，选 B 就等于和它共用一把钥匙。另外，它的访问名单里还有 7 项本项目的测试版，都是之前在弹窗里点了允许加进去的，用户可以在"钥匙串访问"里自行删掉。
- 所有构建默认加 `--use-mock-keychain`：不弹窗，更新后网页也不会卡住；登录状态仍然保存在 DD-up 自己的资料目录里，靠 macOS 账户的文件权限保护。设置 `HUP_CHROMIUM_SYSTEM_KEYCHAIN=1` 可以改回用钥匙串。
- 日用版此前没有启动过浏览器（`~/Library/Application Support/DD-up/Chromium` 不存在），所以没有丢失任何登录状态。

## D23 终端里 Cmd+点击文件链接，在右侧 Files 面板打开（2026-11-04）

- 用户要求：Claude Code 等 Agent 输出里的蓝色文件路径，按住 Cmd 点击后在右侧 Files（files-plus）里打开，不再交给 macOS 默认应用。
- 宿主补丁 0005：点击的链接如果是本机上存在的文件或文件夹（普通路径，或 OSC 8 的 `file://` URL，会做百分号解码，并去掉 `#L3` 这类后缀），就把装着 files-plus 的那个栈切到前面，再发 `files-plus.open:<路径>` / `files-plus.reveal:<路径>`。面板刚切出来时还没在监听，所以每 50 ms 重试一次，最多 2 秒，超时后改用系统打开。布局里没有 files-plus 时，行为和以前一样。网址仍然交给系统打开。
- files-plus：文件在当前文件夹内时，展开它所在的目录再预览；在文件夹外时按完整路径预览，标题只显示文件名。文件夹在当前文件夹内时在树里展开，在文件夹外时用 Finder 打开。点击可能刚把焦点换到另一个面板，导致根目录跟着变，所以 2 秒内会对新目录再执行一次。标题栏新增"用默认应用打开"，以前 Cmd+点击的效果仍然可以用到。
- `native.open-link {path}` 走同一条路径，供脚本和自检使用。
- 补丁 0006（同日修复）：用户实测点击没有反应。原因是右侧面板被顶栏的 Files 按钮收起了，files-plus 没在显示，命令发不进去。现在连同收起的面板一起展开（`layout.show`）。开发版自检确认：面板收起后调用 `native.open-link`，面板会展开并显示文件。

## D24 DD-up 侧边栏不再用 space-folders（用户选 A，2026-11-04）

- 用户的 10 个 folder 和 10 个 space 基本一一对应，每个 space 也只有 1 个 tab，folder 只多出一层，没起到分组作用。
- 用户的 `main-folders` 布局和官方 `main` 唯一的区别就是侧边栏用的 mod，所以改为 `scripts/install --layout-from main`，侧边栏用回自带的 `spaces`。这个基础布局会被记住，以后安装都用它。程序本身没变，也不需要退出 DD-up。
- 恢复办法：`scripts/install --layout-from main-folders`，或者用备份 `~/.config/herdr-app/layouts/dd-up.json.backup-20261104-035100`。`~/.config/herdr-space-folders.json` 和 space-folders mod 都原样保留。

## D25 右键终端里的文件链接，可以在 Finder 中显示（2026-11-05）

- 用户要求：在 Agent 输出的文件路径上点右键，可以打开访达。
- 宿主补丁 0007：右键时，如果鼠标下是本机上存在的文件或文件夹链接（判断方式和 D23 相同，`file://` URL 也算），就把路径作为 `target.link` 传给 pane 菜单。菜单最上面多一项 "Reveal in Finder"，下面加一条分隔线，执行的是 `native.reveal {path}`，也就是 `open -R`，在 Finder 里选中该文件。不在链接上右键时，菜单和以前一样。
- 菜单改的是内置的 `defaults/mods/menu`。用户 `~/.config/herdr-app/mods/menu` 那一份没改过，`sync_defaults` 会随新版本更新它。官方 Herdr 不传 `link`，所以这一项不会出现在官方 Herdr 里。

## D26 TUI 按词折行的长路径也能点击（2026-11-05）

- 用户实测：omp 输出的 `/Users/me/Documents/github/some-project/outputs/月度报表_销售汇总_2026-11.xlsx`，Cmd+点击和右键都没反应。
- 原因：omp 的 `wrapTextWithAnsi` 按词折行（已用它本身复现），把 `_2026-11.xlsx` 整体挪到下一行，上一行离右边缘最多空出一个词的宽度。上游只拼接写到离右边缘 4 格以内的行，所以只识别出不存在的半截路径。这和 Agent 种类无关，Claude Code 只是碰巧把行写满了。
- 宿主补丁 0008：上一行离右边缘的空白，小于"下一行第一个词的宽度 + 4"时也拼接。这正是按词折行的条件：词放不下才换行。拼接后的路径仍须在本机存在才算链接，所以短行和普通文字不会被误拼。单元测试 `a_word_that_did_not_fit_joins_the_row_above` 覆盖了这个情况（旧规则下会失败）。

## D27 鸿蒙版 DD-up：手机远程客户端，加 Mac 桥接服务（用户决定，2026-11-07）

- 用户要求：在 HarmonyOS 7 手机上使用 DD-up。用户选择：做远程客户端（终端仍在 Mac 的 core 里运行）；通过 Mac 桥接服务连接；代码放在本仓库；第一版就做推送。
- 不移植桌面版：GPUI、Ghostty 和 CEF 在鸿蒙上都不可用。手机端用 ArkTS/ArkUI 原生开发，代码在 `mobile/harmony/`。
- 桥接服务 `bridge/`（Rust，`ddup-bridge`）的职责：
  - 运行 `hc connect`，把 core 原有的帧协议原样经加密 WebSocket 转给手机。手机端和桌面版说同一套协议，桥接服务不翻译。
  - 状态判断按 core 的 OSC 7501 `status`、`turn` 和 `herdr.seen`（`docs/labels.md`），不从屏幕文字猜。
- 安全：
  - 扫码配对，配对码一次性、10 分钟内有效。配对后签发设备令牌，Mac 上只存哈希，可以吊销。
  - 每条连接都做 X25519 握手：桥接服务的静态公钥由二维码固定，再加双方临时密钥，有前向保密。之后用 AES-256-GCM 加密。即使跑在局域网明文 `ws://` 上也安全。
  - 设备令牌等于完全控制这个 core（能在终端里输入），第一版不分权限范围。
- 推送：手机进程被杀后，本地通知发不出去，所以由桥接服务调用华为 Push Kit V3 REST，用服务账号 PS256 JWT 鉴权。
  - 没有申请"通知消息自分类权益"时，消息会被归为资讯营销类：静默展示，每天 2 到 5 条。调试阶段用 `testMessage`，每个项目每天 1000 条。
  - 华为开发者后台的项目、Push Kit 开通和服务账号密钥由用户自己创建。密钥只放在 Mac 的桥接服务状态目录（0600），不进仓库。
- 参照：ALMA（alma.now，herdr 的 GUI 客户端，有 iPhone 端）、T3 Code、Vibe42。借鉴的做法：扫码配对、列表按"需要你 / 工作中 / 已完成"分组、快捷键栏、推送由桌面端代发。
- 详细设计和协议见 `docs/harmony/DESIGN.md`。
- 鸿蒙应用包名 `herdr.hm.ddup`、应用级 Client ID 来自用户建好的 AGC 应用（`harmonyOS/agconnect-services.json`，含 client_secret，已加入 `.gitignore`）。

## D28 鸿蒙版：桥接服务常驻、侧边栏手机按钮、开发者后台配置（用户要求，2026-11-07）

- 用户要求：把能做的都做好（安装、开通、申请）；配对不要敲命令，在 Settings 旁边加个手机图标，点一下弹出二维码。
- 桥接服务用 LaunchAgent `dev.ddup.bridge` 常驻，连日常 core。这是用户授权的、对正式 core 的长期读写连接，测试仍然只用 `hup-dev`。脚本：`scripts/bridge-service`。
- DD-up 侧边栏底部换成 `workspace-chrome-plus`（复制自内置 workspace-chrome，只多了手机按钮），弹窗是 `mods/phone-pair`。`scripts/install` 把布局里的 workspace-chrome 换成它。新增 `--mods-only`，可以在 DD-up 开着时只更新 mod 和布局，不替换 App。
- 开发者后台：
  - 推送服务已开通。
  - 服务账号用华为生成的密钥对。自己填公钥那一份始终 80200001，已删除。
  - 调试证书已建。
  - 自分类权益：控制台没有申请入口，暂时没申请。
- 编译：Command Line Tools 26.0.0.865 和 Temurin JDK 21 都放在项目内的 `.local/toolchain`，`scripts/harmony build` 已经通过。
  - 第一次真编译修掉了 7 个 ArkTS 严格模式错误：隐式返回类型、结构类型、`Row` 和 ArkUI 组件同名。
  - API 26 起，SDK 版本要写成 `"26.0.0"`。

## D29 鸿蒙版真机测试中修的问题（2026-11-08）

用户反馈终端画面"全挤在一起"，要求完整测一遍。测试方法：
- 手机临时配到一个连测试 core 的桥接服务（端口 17860），用 `tests/harmony-device/phone`（基于 hdc uitest）点按、输入、截图，测完配回日常 core。
- 测试 Agent 是 `tests/fixtures/phone-agent.sh`。

修掉的问题：
1. **文字重叠**：把 200 列塞进手机屏宽时，字号被算到约 5px，ArkWeb 实际按最小字号显示，xterm.js 按 5px 计算格子，于是字和行都叠在一起。
   - 改为默认"手机宽度"：用 12px 量出屏幕能放多少列、多少行，按这个尺寸 attach（非 observe）。core 在手机接着时把终端改成手机尺寸，断开后自动还给桌面（实测 47×31 → 50×30 → 47×31）。
   - 另有"原尺寸"按钮：只观看，用 CSS transform 整体缩放，双击放大到 1:1 并可横向滚动。
   - Web 组件设了 `minFontSize(1)`。
2. **消息乱序导致掉线**：鸿蒙的 `ws.send` 是异步的，两条同时发出时可能交换顺序到达。桥接服务报 `message failed authentication`（后来报 `replayed or reordered record`），连接被断开。
   - 客户端改为排队发送，上一条发完才发下一条。
   - 记录里带上 u64 计数器，接收方只要求递增，允许跳号（发送失败不再连带后面的记录）。
   - 测试替身的 `send` 现在会随机延迟；去掉队列时这项测试会失败，用来防止回归。
3. **键盘把整页顶上去**：改用 `KeyboardAvoidMode.RESIZE`。弹出键盘后终端从 30 行缩到 14 行，标题栏保持可见。
4. **配对页**：提示文字和输入框的边距、文字颜色。新增 `ddup://pair` 链接：系统相机扫码就能直接打开 App 并配对，测试时也可以用 `aa start -U <link>`。
5. **双击放大**：同时监听 `dblclick`，并清除 xterm 的选中。
6. **测试推送不显示**：App 在前台时也要显示（`foregroundShow`）。
7. **日志提示不准**："没有设备登记推送"改成"手机在前台，不推送"。

真机确认通过的项目（截图在 `.local/phone-shots/`，不进 Git）：
- 用链接配对。
- 列表分组（需要你处理 / 工作中 / 空闲 / 其他终端）。
- 手机宽度渲染（中英文混排、颜色、长路径折行）。
- 被拦截时的提示，以及长按强制发送。
- 快捷键（1/2、Enter，连按 12 次仍保持顺序）。
- 中文发送。
- 原尺寸缩放、双击放大、横向滚动。
- 离开页面后终端尺寸复原；`herdr.seen` 回写。
- 后台、前台切换。
- 推送送达："需要你处理"、"已完成"、测试通知，都在"更多通知"（营销类）里。
- 点通知直达对应终端。
- 键盘弹出时终端缩小。
- 系统扫码器能打开。

## D30 Tailscale 和同时拨号（2026-11-08）

- Mac 装了 Tailscale 1.104.5 官方独立包。安装包核对过 SHA-256、开发者签名和苹果公证。安装、登录和批准 VPN 配置都由用户操作。
- 开机自启：Tailscale 设置窗口的界面读不出来（`agent-desktop` 一直报 window churn），所以没改它自带的开关，改用 LaunchAgent 在登录时 `open -a Tailscale`。
- 手机：从应用市场装了 MeshArc，后端选官方控制面。登录由用户完成；"同意"应用市场隐私声明也是用户点的。
- 实测：两端没能直连，走 DERP(sin)，`tailscale ping` 在 0.6 到 4.3 秒之间。
  - App 原来按顺序拨号、每个地址等 4 秒，结果超时。改为所有地址同时拨号、先通者胜，超时 12 秒。
- 一开始手机上 代理 App 占着 VPN，MeshArc 连不上。用户自己关了 代理 App，也自己点了 MeshArc 的 VPN 授权。之后变成直连（`direct 192.168.1.30:41641`）。
- 只带 Tailscale 地址的配对已经验证过：桥接服务日志里看到 `100.p.q.r ... connected`。
- 最终用完整链接（局域网地址加 Tailscale 地址）重新配对。同时拨号时，Tailscale 那一路先到、完成配对，局域网那一路用同一个配对码晚到一步，被拒为 `pair_invalid`，这是预期行为。多余的配对记录已经吊销。
- 遗留：MeshArc 和 代理 App 不能同时开。macOS 不回应 ping，所以测连通要用真实连接，不能靠 ping。

## D31 推送权益和手机端交互第二轮（2026-11-08）

**推送**
- 实测：DD-up 的横幅、铃声、振动都打开后，营销类推送仍然只进"更多通知"。系统把这一类固定为"其他·仅通知中心"，这个开关是灰的，所以在手机上改设置解决不了。App 菜单里加了"通知设置"入口（`openNotificationSettings`）。
- 自分类权益：控制台没有申请入口。已提交华为工单（用户审阅过内容），申请 WORK 类。华为 AI 客服回复：所有应用都可以申请，可能和应用处于"准备提交"状态或应用分类有关。
- 用户决定：先只用华为推送，不加飞书提醒。
- 应用分类改不改，要等用户确认。

**交互**（参考调研：Happy、ALMA、T3 Code、Moshi、Termius、Claude Remote Control）
- "需要你"按钮卡片：`core/Prompt.ets` 用 `terminal.read` 读屏，解析出编号选项、问题和选项说明，支持把选项写在同一行（`1) 继续 2) 停下`），也支持 y/n。
  - 屏幕上有高亮项时，用 ↑↓ 移到目标再按 Enter；没有高亮时，直接输入编号再按 Enter。
  - "Type something"一类的选项只移动高亮，再把输入框聚焦，接着强制发送。
  - 首页列表和终端页都能直接回答。
  - Agent 刚报 blocked 时可能还没把问题画完，所以读屏会在 0.6 秒和 1.5 秒后各重读一次。
  - 修了一个问题：连接就绪之前已经在等的 Agent 原来不会读屏，现在连上后补读。
- 附件：相册、拍照、文件。HEIC 或 1.5 MB 以上的图片转成 JPEG，长边不超过 2560。用 `terminal.put` 放到终端旁边，再把路径填进输入框。
- 交付物页 `pages/Preview`：
  - 改动列表：用 `git status` 和 `--numstat`，显示每个文件的 diff，也能"看文件"。
  - 在终端里点路径（会把折行接起来识别），可以预览 Markdown（marked 18.1.3，原始 HTML 转义成文本）、代码（带行号、等宽字体）、图片（data URL）、HTML（放进 sandbox iframe，不运行脚本）。
- 输入：长按快捷键发备用键（Termux 风格）；双指捏合调字号，App 运行期间记住字号；断线时显示状态条；"+"按钮改用 Text 绘制，不再显示不全。
- 新建任务 `pages/NewTask`：选最近用过的目录或手填路径，选 Claude Code、Codex、omp 或 Shell，写第一句话。命令通过登录交互 shell 运行（`$SHELL -lic 'eval "$DDUP_CMD"'`，提示语放在 env 里，不拼进命令行）。
- 真机已测（测试 core）：列表内回答、终端页卡片回答、改动列表、diff、看文件、Markdown 表格和 script 转义、图片预览、文件附件上传（中文文件名会被 core 存成下划线，填进去的路径是对的）、长按备用键、新建 Shell 任务和第一句话。
- 未在真机上测：双指捏合（uitest 不支持捏合手势）、相册和拍照入口（避免翻看用户的私人相册，用"文件"入口验证了同一条上传路径）。
- 下一轮：聊天视图（读 Claude Code 和 Codex 的 JSONL 会话记录）、开发服务器预览（需要桥接服务做端口转发）。

## D32 手机端：对话视图和网页预览（2026-11-08）

- 应用分类已改成"应用 · 效率"（标签"日程清单"），自分类权益的入口随之出现，已申请"工作事项提醒"（WORK），状态"申请完成"，等华为审核（约 3 个工作日）。批下来后把桥接服务 `config.json` 里的 `push.category` 改成 `WORK`、`test_message` 改成 `false`（先备份）。
- **对话视图**（参考 T3 Code、ALMA、Happy）：Claude Code 和 Codex 终端默认打开"对话"，顶部"对话 | 终端"切换，选择在 App 运行期间记住。
  - 内容来自 Agent 自己的 JSONL 会话记录，由桥接服务读取、整理（`bridge/src/transcript.rs`，协议见 DESIGN.md §4），不靠解析屏幕。
  - 你的话是右侧气泡，Agent 的文字原样显示（代码块等宽、可长按复制），工具调用一行一个：✓/✕/进行中、中文动作名、摘要；点开看命令、diff（红绿行）、输出，文件类工具有"打开文件"。连续超过 3 个工具调用折叠成"已执行 N 个操作"，只露最后 2 个，有失败的标红。
  - 正在工作时底部显示"正在工作…"；在页面上时每 2 秒取一次增量，发完消息 0.5 秒后再取；滚到底时新内容自动跟上，往上翻时不打扰。
  - 对话模式下快捷键只留 Esc、Enter、Tab、^C；回答卡片和附件照常可用；终端一直挂着，切回"终端"是即时的。
  - 找会话：按前台进程 pid（见 DESIGN.md），实测这个终端的 shell 目录是另一个项目、Claude 实际在 herdr-universal-preview，按 pid 找到的是对的那一份。顺带：列表和标题里的目录也改用前台进程的目录（和桌面版一致）。
- **网页预览**：首页"网页"按钮列出 Mac 上能回 HTTP 的端口（进程、项目目录、页面标题），点开在 App 内浏览；也能手输端口。终端里点 `http://localhost:…`、对话里 Agent 提到的本机地址（"预览 localhost:5173 ›"）都直接打开预览。
  - 桥接服务为端口开带一次性令牌的转发（DESIGN.md §4），手机直接连 Mac 的局域网或 Tailscale 地址，热更新的 WebSocket 也能过。
  - 页面内：刷新、"浏览器"（交给系统浏览器）、返回键先在网页历史里后退，退完回到列表。
- 验证：
  - 桥接服务单元测试 26 项（新增会话记录 3 项、转发 4 项），clippy、rustfmt 通过；`bridge-e2e` 24/24（新增：会话记录全量和增量、端口列表、转发的 403/错误令牌/cookie/后续请求）。
  - `harmony-logic` 42/42（新增：前台进程跟踪、ports 请求按 id 回复、没有会话时的提示）。
  - 用本机真实的 Claude 和 Codex 会话记录抽查过；据此修了 Codex 新格式（JS 包着的命令、JSON 包着的输出）。
  - 真机（日常 core，走 Tailscale）：对话视图显示当前会话、展开工具行、折叠、切到终端、网页列表、打开本地测试页、页内按钮和链接（cookie 生效）、返回键。有截图，不等于用户验收。
- 未测：Codex 终端的对话视图（当时没有在跑的 Codex，只用记录文件验证了解析）；从对话视图发消息后的刷新（发送路径没变）；热更新 WebSocket 在真机上的表现（单元测试覆盖了升级请求的改写）。
- 补充（同日）：
  - 对话视图顶部"加载更早的内容"：桥接服务的 `transcript` 请求可带 `before`（字节位置），返回这之前一段（按整行切，不重不漏，回复带 `start` 和 `older`）；每条记录带所在行的位置 `o`。加载后内容加在上面，画面停在原处；更早部分里结果已不在的工具行不再转圈。
  - 网页列表长按：隐藏这个端口，或隐藏同一目录下同一程序的全部服务（例如 .hermes 的 9 个 python3.12）；存在手机上，列表底部"已隐藏 N 个 · 显示"可找回，长按已隐藏的可取消隐藏。
  - 验证：桥接服务单元测试 27 项（新增：往前读不重不漏），`bridge-e2e` 24/24，`harmony-logic` 42/42；真机上加载更早内容、隐藏 .hermes 一组、重进后仍隐藏、显示和收起都测过。手机上 .hermes 那组现在是隐藏状态。

## D33 手机 App 改名"码上"，换像素风图标（2026-11-08）

- 用户要求手机端不叫 DD-up，换一个"专属我的开发工具"的名字；网页版 GPT 打不开（Chrome 扩展导航无效），改为用代码画像素风图标。
- 名字：**码上**（"马上"的谐音，码在手上、随时开工）。只改显示名（`app_name`、`EntryAbility_label`）、首页标题、测试推送标题、预览链接失效页和 Mac 上配对弹窗里的提示。包名 `herdr.hm.ddup`、`ddup://pair`、`ddup-bridge`、桌面版 DD-up 都不变，否则签名、推送、AGC 和已配对的设备都要重来。AGC 后台的应用名仍是 HM-Herdr，没动。
- 图标：`scripts/make-phone-icon`（纯 Python，不依赖库）画 16×16 格、每格 64 px 的 1024 图。第一版（终端窗口加冲出的箭头、抖动背景）用户觉得丑；又出了 4 个候选（像素"码"字、大号 >_、深色底机器人、青色底机器人），用户选了**青色底的小机器人**：脸是终端 `>_`，头顶天线亮着黄灯，青绿到天蓝的斜向色带背景。满版不透明，交给鸿蒙自己裁圆角。改图就改脚本里的 `ART`/`PALETTE` 再运行，两份 `app_icon.png` 都会更新。
- 验证：编译、安装到手机；桌面和"工作"文件夹里显示新图标和"码上"（截图）。桥接服务测试 27 项通过并已重装，配对弹窗 mod 已用 `--mods-only` 更新。


## D34 推送改为工作提醒；桌面卡片直接回答（2026-11-09）

- 自分类权益“工作事项提醒”（WORK）已审核通过（申请记录：2026-11-08 15:31 提交，状态“审核通过”）。桥接服务 `config.json` 改为 `category: WORK`、`test_message: false`（原文件备份为 `config.json.backup-20261108-201919`）。真机：通知进通知栏主区域（不再在“更多通知”），桌面时有横幅。横幅上只显示“码上 / 1 个通知”、不显示标题正文，原因未查明（截屏时系统可能隐藏内容），待用户肉眼确认。
- 用户想在通知上直接点“确认 / 继续 / 驳回”。查了华为文档：推送通知（V3 `notification`）只有 `clickAction`，没有按钮和快捷回复；本地通知可以带按钮，但 App 在后台很快被冻结，收不到事件，发不了。用户选择两个都做：
  1. 点通知直接到回答卡片：已有（点通知进对应终端，回答卡片默认展开），真机确认。
  2. **桌面卡片**（DESIGN.md §6b）：显示在等你的 Agent 和按钮，App 不在前台时由桥接服务用 Push Kit 卡片刷新消息更新；点按钮在后台拉起 App 回答。
- 卡片额度：华为对工具/效率类每张卡每天 50 次推送刷新，桥接服务自限 45 次、相同内容不重发、手机在前台时不发（App 自己刷）。
- 顺带修了一个旧问题：用配对链接冷启动 App 时，新配对和“重连上次的 Mac”同时进行，后连上的那条会顶掉新配对（这次真机上就发生了：界面还是日常 core）。现在配对期间不重连，且只有最新一次拨号能成为当前连接。
- `scripts/test` 的 `bash -n` 不再检查 Python 脚本（`scripts/make-phone-icon` 从 D33 起让这项一直失败）。
- 验证：
  - 桥接服务单元测试 32 项（新增：屏幕问题解析与 App 同一批样例、卡片值、每日额度、等你列表的排序、卡片消息格式和 version 范围），clippy、rustfmt 通过；`bridge-e2e` 24/24；`harmony-logic` 44/44（新增卡片值 2 项）；`scripts/test --bridge` 全部通过。
  - 真机（日常桥接服务）：卡片加到桌面；`ddup-bridge card-test` 经 Push Kit 刷新卡片（App 在后台），卡片显示问题和“允许 / 允许，不再问”。第一次华为拒绝了 `version`（超出 int32），改成 0.1 秒计数后成功。
  - 真机（测试 core 上的假 Agent，“要继续下一轮吗？1) 继续 2) 停下”）：App 被强制停止后点卡片“继续”，App 在后台拉起、连上、核对问题、发送，Agent 收到“1”进入第 2 轮；点卡片空白处打开那个终端，回答卡片在底部。测完已配回日常 core，并吊销了旧的设备记录（现在日常桥接服务只有 1 台设备，推送和卡片都已登记）。
- 未测：日常 core 上真实 Agent 触发的卡片推送刷新（逻辑和 card-test 同一条路，读屏幕部分由单元测试和测试 core 覆盖）；“问题已经变了”的分支；配对竞态修复只在本次重新配对时走过一次（当时旧桥接服务已停，不能完全证明竞态已消除）。

## D35 在外面连接不稳：查明原因，加心跳、网络变化重连、地址自动更新；页面铺满；默认对话（2026-11-09）

- 用户反馈：在外面常连不上；页面没有全屏；进 Agent 应该默认是对话。
- 原因（查实）：在外面手机只能经 Tailscale 连 Mac。11-07 晚上手机经 MeshArc（100.p.q.r）连上过 43 次，那时 Mac 跑的是官方 Tailscale App（100.m.n.o）。现在官方 App 是“已停止”，用户改用 代理 App 内置的 Tailscale（节点 （另一个节点），100.a.b.c）。实测它不接受连入：手机走 5G 经 MeshArc 连 `100.a.b.c:7841` 配对，超时，桥接服务日志里一条连接都没有。另外今天重新配对时 Mac 没有 Tailscale 地址，配对链接里只剩局域网地址，就算 Tailscale 恢复，App 也不会去连。
- 代码改动（DESIGN.md §3b）：
  - 心跳：App 前台每 15 秒 `ping`，10 秒没回就重连；回前台和网络变化时立即探测。桥接服务 60 秒没收到手机任何帧就断开（以免它以为 App 还在前台而压住推送）。
  - 地址自动更新：`ok`/`paired`/`pong` 带上 Mac 当前地址，App 合并保存（旧地址保留，最多 8 个）。
  - 首页提示：Mac 没有 Tailscale 地址时提醒“离开这个网络就连不上”，连接失败时说明该检查什么；Mac 的 Tailscale 地址消失约 1 分钟后，桥接服务推一条通知。
  - 页面铺满：窗口背景、状态栏、导航条区域改成和页面一样的深色，状态栏图标改白色。
  - 新配对失败不再丢掉原来的配对（见下）。
  - Claude Code / Codex 终端每次都默认打开“对话”（以前切到“终端”后，本次运行里一直停在终端）。
- 没做：不依赖 Tailscale 的中转（例如 Cloudflare Tunnel 或自己的服务器）。等用户决定 Mac 上用不用官方 Tailscale 再说。
- 验证：
  - 桥接服务单元测试 32 项、clippy、rustfmt；`bridge-e2e` 通过；`harmony-logic` 47 项（新增 Tailscale 地址判断、地址合并、心跳回带地址）。
  - 真机（日常桥接服务，手机走 5G，经 USB 反向端口转发 `hdc rport tcp:7841 tcp:7841` 连 Mac）：前台放置 75 秒以上连接不断（超过桥接服务 60 秒的空闲上限，说明心跳在起作用）；拔掉转发后 App 显示“连接中”，恢复转发 5 秒内重连；首页显示 Tailscale 提示；状态栏和底部是深色；点 Claude Code 终端默认是对话。已配对时打开一个连不上的配对链接：约 12 秒后配对失败，App 自动连回原来的 Mac，设备记录不变。
  - 测试中 App 清掉了旧配对：以前用新链接配对时先删旧配对再连，新的连不上就两头落空。已改成新配对成功后才替换，失败则重连原来的 Mac。手机已经通过 USB 重新配对（新设备 （上一台），推送和卡片已登记），吊销了旧的 （旧设备）。
- 未测：系统网络变化回调触发的重连（没有在真机上切换 Wi-Fi/5G）；桥接服务的 Tailscale 断开推送（Mac 上官方 Tailscale 一直没开，没有出现“有→无”的变化）；真正在外面经 Tailscale 连接（要等 Mac 上官方 Tailscale 打开）。

## D36 Mac 上用 DD-up 自己的 userspace tailscaled（2026-11-09）

- 背景（D35）：官方 Tailscale App 和 代理 App 互相抢网络，用户确认；代理 App 内置的 Tailscale 不接受连入。用户同意改用开源 `tailscaled` 的 userspace 模式。
- 做法：`scripts/tailscale-service install` 用本机已有的 Go 编译 `tailscale` 和 `tailscaled` v1.86.5，放到 `~/Library/Application Support/DD-up/tailscale/bin`。以 LaunchAgent `dev.ddup.tailscaled` 运行，参数 `--tun=userspace-networking`，状态和 socket 也在这个目录下。不用 brew，不用 sudo，不占 VPN 通道。`login` 用 `--hostname=mac-ddup --accept-dns=false --accept-routes=false` 加入 tailnet，用户在浏览器里登录并批准。节点地址 100.x.y.z。
- 桥接服务：`near_hosts` 会向这个 `tailscaled` 询问地址（2 秒超时，缓存 20 秒），所以配对链接、`ok`/`pong` 里的地址和“Tailscale 断了”的检测都包含它。
- 验证（真机，手机关 Wi-Fi、走 5G，经 MeshArc；已移除 USB 端口转发）：
  - 用只带 `100.x.y.z` 和局域网地址的链接配对成功，设备 （新设备）（吊销了上一台 （上一台））。强制停止 App 再打开，仍然经 `mac-ddup` 连上，首页“已连接”。
  - 停掉 `dev.ddup.tailscaled` 80 秒：桥接服务约 1 分钟后推送“Mac 的 Tailscale 断了”，华为返回 Success。恢复后日志记录“Tailscale is up again”。
  - `ddup-bridge status` 的 addresses：`192.168.1.20, 100.x.y.z`。
- 切网络实测（真机，App 在前台，Claude 用控制中心开关 WLAN）：
  - 经 Tailscale 连着时，5G→Wi-Fi、Wi-Fi→5G：都不断线（Tailscale 自己切换底层网络），列表持续更新。
  - 经家里 Wi-Fi 直连时关掉 Wi-Fi：8 秒后经 Tailscale 重新连上；桥接服务在 60 秒空闲上限时丢掉了那条死掉的 Wi-Fi 连接（日志“nothing from the phone for 60 s”）。D35 里“网络变化回调触发的重连”这一项由此补测。
- 已知：`tailscaled` 的出站流量（控制面、中继）会经过 代理 App 的分流规则；目前能用，延迟没有测。官方 App 仍是“已停止”；登录时打开它的 LaunchAgent `dev.ddup.tailscale-open` 没有删，要不要删由用户决定。

