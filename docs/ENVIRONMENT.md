# 环境与隔离证据

记录时间：2026-11-02（M0）。本文档只记录事实和检查方法，不包含任何密钥、令牌、Cookie 或私密配置内容。

## 本机

| 项 | 值 | 怎么查的 |
|---|---|---|
| macOS | 26.6.2 (25G83) | `sw_vers` |
| CPU | arm64 (Apple Silicon) | `uname -m` |
| 内存 | 64 GiB | `sysctl hw.memsize` = 68719476736 |
| 可用磁盘 | 365 GiB（Data 卷） | `df -h ~` |
| Xcode | **未安装**，只有 Command Line Tools，位于 `/Library/Developer/CommandLineTools` | `xcode-select -p`；`xcodebuild` 报"requires Xcode" |
| SDK | MacOSX.sdk 15.2（CLT） | `xcrun --show-sdk-version` |
| clang/libclang/swiftc | 由 CLT 提供 | `ls …/usr/lib/libclang.dylib` |
| Git | 2.39.5 (Apple Git-154) | `git --version` |
| Rust | 本机原本没有；已按项目隔离安装 1.95.0，位于 `.local/toolchain/` | `scripts/bootstrap-toolchain --check` |
| Zig | 本机原本没有；已按项目隔离安装 0.16.0，位于 `.local/toolchain/zig` | 同上 |
| just | 未安装，也不安装；脚本直接调用 cargo | — |
| 其他 | brew、bun、node、uv、python3、gh 已存在；本项目暂时只用 uv 和 python3（生成样例）以及 gh（只读查询上游） | `which` |

开发机是一台 Apple Silicon Mac。其他配置的机器没有实测，性能结论不外推。

## 正式 Herdr（只读观察，从不写入）

| 项 | 值 |
|---|---|
| App | `/Applications/Herdr.app`，bundle id `dev.herdr.app`，版本 0.1.12（DMG 安装；README 说的 `~/Applications` 在本机不成立） |
| `herdr-app` 命令 | `~/.local/bin/herdr-app`，符号链接指向 `/Applications/Herdr.app/Contents/MacOS/Herdr` |
| `hc` 命令 | `~/.local/bin/hc`，符号链接指向 `herdr-core`（0.1.17，sha256 `b0ad77ab78ca8d9a…`） |
| App home | `~/.config/herdr-app`（布局 main/main-folders/terminals/win98；内置 mods 共 21 个，含 space-folders；本次没有发现用户自定义的 mod） |
| core 设置 | `~/.config/herdr-core/` 不存在，使用默认设置 |
| core 状态目录 | `~/.local/state/herdr-core/default/`（session `default`，socket `default.sock`） |
| core 身份 | id `d4714da914be4fb680b1dedc0f792ea7`，pid 3868，live 3 |
| launchd | 只有 `application.dev.herdr.app.*`（App 本身），core 没有作为 launchd job 出现 |

**当前 Agent 自身运行在正式 Herdr 的终端里**：环境中有 `HC_SCOPE`、`HC_TERMINAL_ID`、`HC_TERMINAL_SOCKET`。所有测试入口都会先清除它们（`scripts/lib/env.sh: hup_isolate`）。

## 上游参考状态（gh api 只读查询，2026-11-02）

| 资料 | 状态 |
|---|---|
| herdr-app | 公开仓库，Apache-2.0，默认分支 master。本机版本对应 `v0.1.12` = `4aa3460749947e7771aa39a8a2cec58d46649926`，master 已领先 31 个提交 |
| herdr-core | 公开仓库，Apache-2.0 |
| Issue #91（Files 预览 Markdown/图片/PDF） | OPEN |
| Issue #218（Mod 无法承载原生视图） | CLOSED，原因 NOT_PLANNED。维护者表示浏览器"在路线图上，可能成为官方 mod"。讨论中确认：新增 layout `Content` 变体会让旧版整份布局解析失败 |
| PR #235（原生浏览器 pane） | OPEN，Draft，未合并（head `969c0c4f`）。只作为技术参考，没有套用 |
| 构建要求 | Rust 1.95.0、Zig 0.16.0（README 中的 Build from source 一节）；ghostty 子模块 `291fd050b9e9c293c8adb6a140742649b7e42d3a` |

## 测试隔离方案

唯一入口是 `scripts/lib/env.sh`。依据：上游 `scripts/bench.py`、`scripts/sandbox` 的做法，以及 herdr-core 内置说明 "Tests need their own `HC_SESSION` and scratch XDG paths"。

| 变量 | 测试值 | 作用 |
|---|---|---|
| `HC_SESSION` | `hup-dev` | 使用独立的 core session、socket 和锁 |
| `XDG_STATE_HOME` | `/tmp/hup-$UID/state`（符号链接指向项目内 `.local/state`） | core 状态和 socket。macOS socket 路径上限是 104 字节，项目路径太长，所以要走短链接 |
| `XDG_CONFIG_HOME` | `/tmp/hup-$UID/config`（同上） | core 的 config.toml 和模板 |
| `HERDR_APP_HOME` | `.local/app-home` | App 的配置、mods、布局和控制 socket（`src/mods.rs:51`、`src/control.rs:15`） |
| `HERDR_CORE_DIRS` | `.local/bin` | 替换 App 查找 core 的所有位置（`src/core_bin.rs:90`） |
| `HC_INSTALL_DIR` | `.local/bin` | 万一触发 core 安装，也只装到这里 |
| `HERDR_APP_NO_SOUND` | `1` | 不在真实扬声器上播放提示音 |
| 清除 | `HC_SOCKET HC_TERMINAL_ID HC_TERMINAL_SOCKET HC_SCOPE` | 断开与正式终端或 core 的继承连接 |

- 测试 core 二进制：`.local/bin/hc`，是正式 herdr-core 的副本（sha256 记录在 `upstream.lock.json`），始终用绝对路径调用。
- 测试 App：自建的 `Herdr Preview Dev.app`，bundle id `dev.herdr.app.preview-dev`，可执行文件 `HerdrPreviewDev`。它不会触发正式 App 的命令链接修复：`src/core_bin/cli_link.rs` 只在路径以 `Herdr.app/Contents/MacOS/Herdr` 结尾时才改写 `~/.local/bin/herdr-app`。
- 自动更新：`src/update.rs:9-11, 119-126` 规定只有设置了编译期 `HERDR_APP_RELEASE` 或运行时 `HERDR_RELEASES_URL` 时才检查更新。本项目构建两者都不设，`hup_isolate` 也会清除后者。
- 停止进程：`scripts/dev core-down` 和 `app-down` 只停止 pid 文件中记录、并且 `ps eww` 显示 `HC_SESSION=hup-dev` 的进程，从不使用 `pkill`。

## 隔离验证结果

命令：`scripts/dev isolation-check`。原始证据在 `artifacts/isolation-*.json(l)`，不纳入 Git。

| 检查 | 结果 |
|---|---|
| 测试 core id ≠ 正式 core id | 通过：`8123b3af…` ≠ `d4714da9…` |
| 测试 core pid ≠ 正式 core pid | 通过：61612 ≠ 3868 |
| 标记终端 `hup-isolation-marker` 出现在测试 `hc ls` 中 | 通过 |
| 标记终端不出现在正式 `hc ls` 中 | 通过 |
| 测试期间正式 core 的 id、pid 和各文件指纹不变 | 通过 |
| 测试结束后正式 core 的 id、pid 和各文件指纹不变；live 3 → 3 | 通过 |
| 测试 core 状态目录 | `.local/state/herdr-core/hup-dev/`，socket 为 `hup-dev.sock` 和 `hup-dev.holder.*.sock` |
| 测试期间 launchd | 只有 `application.dev.herdr.app.*`，测试 core 没有注册 launchd job |
| `core-down` | 测试 pid 61612 已退出，正式 pid 3868 不受影响 |

纳入指纹的正式文件：`~/.local/bin/{herdr-app,hc,herdr-core}`、`~/.config/herdr-app/{keys.json,settings.json,layouts/main.json}`、`/Applications/Herdr.app/Contents/{Info.plist,MacOS/Herdr}`。每次启动测试 App 前后都用 `scripts/dev snapshot` 复查。
