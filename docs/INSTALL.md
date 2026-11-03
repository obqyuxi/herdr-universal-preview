# 安装 Herdr Plus（日用版）

Herdr Plus 是在官方 Herdr 0.1.12 上加了本项目补丁的个人构建，装在官方 Herdr 旁边，**不覆盖** `/Applications/Herdr.app`。

| 项 | 内容 |
|---|---|
| 应用 | `~/Applications/Herdr Plus.app`（bundle id `dev.herdr.app.plus`，包含 Chromium 156） |
| 配置 | 使用日常的 `~/.config/herdr-app`：工作区、终端和 Agent 都是原来那套（同一个 herdr-core） |
| 新增 mod | `files-plus`（Files，可直接查看 Office/PDF/图片/音视频）、`browser`（侧边 Chromium 浏览器）、`tabs-plus`（顶栏多出"浏览器"按钮） |
| 新增布局 | `herdr-plus`（官方 main 布局，把其中的 Files 和顶栏换成上面的 mod） |
| 浏览器资料 | `~/Library/Application Support/HerdrPlus/Chromium`（独立于 Chrome 和 Safari） |
| 安装清单 | `~/Library/Application Support/HerdrPlus/install-manifest.json`（记录路径和内容指纹） |
| 更新 | **不会自动更新**。官方升级后需要重新执行 `scripts/prepare-upstream`、`scripts/build --daily`、`scripts/install` |

## 步骤

```sh
scripts/build --daily          # 生成 artifacts/Herdr Plus.app
scripts/install --dry-run      # 先看会新增什么
scripts/install
```

然后：

1. 在官方 Herdr 中按 ⌘Q 退出。终端和 Agent 由 herdr-core 托管，会继续运行。
2. 打开 Herdr Plus。
3. 按 ctrl-shift-l 切换到 `herdr-plus` 布局，或者执行 `herdr-app do layout.switch '{"name":"herdr-plus"}'`。

两个 App 共用同一个 home，**同一时间只能开一个**。另一个已经打开时，后打开的那个只会把前一个的窗口带到前面。
