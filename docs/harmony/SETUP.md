# 鸿蒙版 DD-up：安装与使用（D27）

整体结构见 `docs/harmony/DESIGN.md`：Mac 上运行 `ddup-bridge`，手机装 DD-up 鸿蒙 App，两者通过加密的 WebSocket 连接。

## 一、Mac：桥接服务（已安装并在运行）

```sh
scripts/bridge-service install     # 编译 release 版，装到 ~/Library/Application Support/DD-up/bin，注册 LaunchAgent dev.ddup.bridge
scripts/bridge-service status      # launchd 状态和 ddup-bridge status
scripts/bridge-service logs        # ~/Library/Logs/DD-up/bridge.log
scripts/bridge-service uninstall   # 停止并移除（配对记录和密钥保留）
```

- 登录后自动启动，进程退出后自动拉起。它连的是日常 core（`~/.local/bin/hc`，DD-up 和官方 Herdr 共用的那个）。
- 状态目录 `~/Library/Application Support/DD-up/bridge/`：`config.json`、`identity.json`、`devices.json`、`push-key.json`（全部 0600）。
- 只接受本机、局域网私有地址和 Tailscale 地址的连接。
- Tailscale（2026-11-08 已配好）：
  - Mac 地址 100.m.n.o，开机自启靠 LaunchAgent `dev.ddup.tailscale-open`。
  - 手机装了 MeshArc（`io.github.tailscaleohos`，同一个账号，地址 100.p.q.r）。
  - 二维码里同时带局域网地址和 Tailscale 地址，App 会同时去连，谁先连上就用谁。
  - 两边之间暂时没有直连，走的是新加坡的中继（DERP），往返 0.6 到 4 秒，所以连接超时设成 12 秒。
- **鸿蒙同一时间只能有一个 VPN。** MeshArc 连上后会顶掉 代理 App，反过来也一样。在家用 Wi-Fi 不需要 MeshArc；在外面要连 Mac，就切到 MeshArc。

### 配对：点 DD-up 侧边栏底部 Settings 右边的手机图标

弹窗里会显示二维码（10 分钟内有效，只能用一次）。手机扫完后，弹窗显示"已连接"并自动关闭。如果桥接服务没在运行，弹窗里会直接提示。实现：`mods/workspace-chrome-plus`（加了按钮的侧边栏底部）和 `mods/phone-pair`（弹窗，调用 `ddup-bridge pair --json`）。

命令行也能配对、管理设备：`ddup-bridge pair` / `devices` / `revoke <id|名称>`（二进制在 `~/Library/Application Support/DD-up/bin/`）。

## 二、华为开发者后台（已完成的部分）

| 项 | 状态 |
|---|---|
| 应用 HM-Herdr（`herdr.hm.ddup`） | 用户已建 |
| 推送服务（Push Kit） | **已开通**，并已对该应用生效 |
| 服务账号 `ddup-bridge-push-gen`（角色：运营） | **已建**，密钥在 `push-key.json`，`push-test` 已验证鉴权能过 |
| 调试证书 `ddup-debug`（有效期到 2027-11-07） | **已建**，私钥在 `~/Library/Application Support/DD-up/harmony-signing/` |
| 测试手机、调试 Profile `ddup-debug`（到 2027-11-07） | **已建**，`ddup-debug.p7b` 在 harmony-signing 目录 |
| 通知消息自分类权益 | 控制台"推送服务 → 配置"里没有出现申请入口，没能申请。暂时用测试消息（每个项目每天 1000 条） |

注意：服务账号要用控制台"生成公私钥"生成的那一对。自己填公钥的方式，推送接口一直返回 80200001。

## 三、手机：鸿蒙 App

本机已有编译工具链（项目内）：Command Line Tools 26.0.0.865 在 `.local/toolchain/harmony`（官方 SHA-256 已核对），Temurin JDK 21 在 `.local/toolchain/jdk`（SHA-256 已核对）。

```sh
scripts/harmony build      # 已通过，生成 entry-default-unsigned.hap
scripts/harmony udid       # 手机连上后取 UDID
scripts/harmony install    # 签名、安装、打开（需要 ddup-debug.p7b）
```

手机第一次安装前，需要你做的：
1. 手机打开"设置 → 关于手机"，连续点版本号打开开发者模式；在"开发者选项"里打开"USB 调试"，用数据线连上 Mac，手机上点"允许"。
2. 之后剩下的事我来做：`scripts/harmony udid` 取 UDID → 在控制台注册设备、生成调试 Profile（`ddup-debug.p7b`）→ `scripts/harmony install`。

## 四、日常使用

- **回答 Agent 的提问**：Agent 在等你时，首页列表那一行和终端页底部都会出现选项按钮，点一下就回答。选"自己输入"类的选项后，在输入框写回答再发送。
- **桌面卡片“等你处理”**：长按桌面空白处 →“编辑桌面”→“卡片”，搜“码上”，添加到桌面。有 Agent 在等你时，卡片上显示问题和“允许 / 拒绝 / 继续”这类按钮，点一下就回答，不用打开 App（App 会在后台连上 Mac、确认问题没变再回答）。点卡片其他地方打开那个终端。添加后打开一次 App，Mac 才知道有这张卡片。
- **交给 Agent 文件**：输入框左边的"+"可以选相册、拍照或文件。文件会传到 Mac 上那个终端旁边，路径自动填进输入框。
- **看交付**：终端页右上角"改动"可以看 git 改了哪些文件和每个文件的 diff。在终端里点文件路径可以直接预览（Markdown、代码、图片、HTML）。
- **新建任务**：首页右上角"+"，选目录、选 Agent、写第一句话，然后启动。
- **快捷键**：长按是备用键。↑ 是 PgUp，↓ 是 PgDn，← 是 Home，→ 是 End，Esc 是连按两次 Esc，^C 是 ^D，Enter 是退格。双指捏合调整终端字号。

- 列表分组：需要你处理 / 工作中 / 已完成 / 空闲 / 其他终端，和桌面版侧边栏的规则一样。在手机上看过的 Agent，桌面上也会变成"已读"（`herdr.seen`）。
- 点进一个终端能看到实时画面。手机只观看，不会改 Mac 上终端的尺寸。双击画面在"适应宽度"和"放大后横向滚动"之间切换。
- 下方快捷键：Esc、Enter、方向键、Tab、^C、1/2/3、y/n，用来回答权限确认和选择题。
- 输入框发送一段文字并自动回车。Agent 正在等你确认时，普通发送会被拒绝；长按"发送"可以强制输入。
- App 退到后台后连接会断开，回到前台自动重连。App 不在前台时，"需要你处理"和"已完成"靠推送提醒。

## 五、出问题时

| 现象 | 查什么 |
|---|---|
| 一直连不上 | Mac 上 `serve` 是否在运行；手机和 Mac 是否在同一个 Wi-Fi 或同一个 tailnet；`ddup-bridge status` 里的 `listen` |
| 提示"已在 Mac 上被移除" | 重新运行 `ddup-bridge pair` 再扫一次 |
| 测试推送提示没有推送 token | 检查 `client_id`、推送服务是否已开通、签名是否关联了该应用 |
| 测试推送提示 HTTP 4xx | 检查 `push-key.json` 是否来自同一个项目（`project_id`）；看 `serve` 输出的 Push Kit 返回内容 |
