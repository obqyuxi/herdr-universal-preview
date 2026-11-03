# 安装 DD-up（日用版）

DD-up 是在官方 Herdr 0.1.12 上加了本项目补丁的个人构建，装在官方 Herdr 旁边，**不覆盖** `/Applications/Herdr.app`。名称、可执行文件名、bundle id 和布局名只在仓库根目录的 `brand.env` 里设置一次；图标源文件是 `assets/icon/logo.svg`，用 `scripts/make-icon` 生成 `AppIcon.icns`。

| 项 | 内容 |
|---|---|
| 应用 | `~/Applications/DD-up.app`（bundle id `dev.ddup.app`，包含 Chromium 156，本机临时签名） |
| 配置 | 使用日常的 `~/.config/herdr-app`：工作区、终端和 Agent 都是原来那套（同一个 herdr-core） |
| 新增 mod | `files-plus`（Files，可直接查看 Office/PDF/图片/音视频）、`browser`（侧边 Chromium 浏览器）、`tabs-plus`（顶栏多出"浏览器"按钮） |
| 新增布局 | `dd-up`（官方 main 布局，把其中的 Files 和顶栏换成上面的 mod） |
| 浏览器资料 | `~/Library/Application Support/DD-up/Chromium`（独立于 Chrome 和 Safari） |
| 安装清单 | `~/Library/Application Support/DD-up/install-manifest.json`（记录路径和内容指纹） |
| 更新 | **不会自动更新**。官方升级后需要重新执行 `scripts/prepare-upstream`、`scripts/build --daily`、`scripts/install` |

## 步骤

```sh
scripts/build --daily          # 生成 artifacts/DD-up.app
scripts/install --dry-run      # 先看会新增什么
scripts/install
```

然后：

1. 在官方 Herdr 中按 ⌘Q 退出。终端和 Agent 由 herdr-core 托管，会继续运行。
2. 打开 DD-up。
3. 按 ctrl-shift-l 切换到 `dd-up` 布局，或者执行 `herdr-app do layout.switch '{"name":"dd-up"}'`。

两个 App 共用同一个 home，**同一时间只能开一个**。另一个已经打开时，后打开的那个只会把前一个的窗口带到前面。

## 改名

修改 `brand.env`，然后按顺序执行：`scripts/uninstall`（**用旧名字的代码**，先卸掉旧名字的安装）、`scripts/build --daily`、`scripts/install`。浏览器资料按可执行文件名分目录存放，改名后会从空的资料开始。
