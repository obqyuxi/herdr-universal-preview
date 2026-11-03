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
scripts/build --daily          # 生成 artifacts/apps.noindex/DD-up.app
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

## 用你自己的布局

`dd-up` 布局是照官方 `main` 做的。你平时用的是别的布局（比如装了 space-folders 的 `main-folders`）时，可以在它的基础上生成一份带 DD-up 功能的布局：

```sh
scripts/install --layout-from main-folders   # 生成 layouts/dd-up-main-folders.json
```

它只改这三处：Files 换成 files-plus，顶栏换成 tabs-plus，Files 所在的面板里加一个浏览器（面板默认宽度至少 560）。其余设置原样保留，比如侧边栏用的 mod、尺寸和颜色。原布局文件不会被改动。这一步会记进安装清单，以后重新安装时会按原布局的最新内容重新生成，卸载时一并删除。DD-up 运行时也可以执行。
