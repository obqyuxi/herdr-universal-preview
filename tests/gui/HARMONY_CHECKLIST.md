# 鸿蒙版 DD-up 真机验收清单（D27）

前提：已按 `docs/harmony/SETUP.md` 编译、安装并配对。每项记录：通过 / 不通过 / 现象。结果写进 `docs/ACCEPTANCE.md`。

| # | 操作 | 期望 |
|---|---|---|
| H1 | DevEco Studio 编译 `mobile/harmony` | 编译通过（记下 DevEco 和 SDK 版本，以及改过的报错） |
| H2 | 在 Mac 上 `ddup-bridge pair`，手机扫码 | 提示"配对成功"，列表顶部显示 Mac 名称和绿点 |
| H3 | 粘贴链接配对（另一台手机，或移除后重配） | 同 H2 |
| H4 | Mac 上让一个 Agent 弹出权限确认 | 手机列表里，它出现在"需要你处理"下，并显示原因 |
| H5 | 点进这个 Agent | 能看到实时画面，没有弹出软键盘，中文显示正常 |
| H6 | 用快捷键回答（例如 1 / Enter） | Mac 上的 Agent 继续运行，手机上的状态变成"工作中" |
| H7 | 在输入框输入中文并发送 | Mac 终端里出现同样的文字并自动回车 |
| H8 | 在手机上看过一个"已完成"的 Agent | 桌面版 DD-up 里它也变成已读 |
| H9 | 双击终端画面 | 在适应宽度和放大滚动之间切换 |
| H10 | 把 App 切到后台，再切回来 | 自动重连，列表恢复 |
| H11 | 菜单 →"测试推送"（已配置 Push Kit） | 手机收到"DD-up 测试通知" |
| H12 | App 在后台时，让 Agent 进入 blocked | 收到"需要你处理"推送；点击后打开这个终端 |
| H13 | App 在前台时，让 Agent 进入 blocked | 不收到推送（前台不打扰） |
| H14 | Mac 上 `ddup-bridge revoke <名称>`，手机重新连接 | 提示"已在 Mac 上被移除" |
| H15 | 手机连 Tailscale 类客户端、关掉 Wi-Fi 后打开 App | 通过 Tailscale 地址连上 |

## 设备自检记录（2026-11-08，Claude 用 hdc uitest 操作，有截图，不等于用户验收）

H1–H13 和扫码器能打开，都已在真机（HarmonyOS 7）上自检通过，见 D29。H14（吊销）在桥接服务一侧验证过。H15（Tailscale）未测：Mac 上没有装 Tailscale。
