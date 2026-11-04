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
