# herdr-universal-preview：给 Agent 的项目说明

主任务书：`docs/CLAUDE_IMPLEMENTATION_BRIEF.md`（以它为准；本文件只放约束、入口和索引）。

## 硬约束

- **不碰正式环境**：`/Applications/Herdr.app`、`~/.config/herdr-app`、`~/.local/bin`、正式 core（session `default`）都只做只读观察。所有测试一律通过 `scripts/dev`，或先 `. scripts/lib/env.sh && hup_isolate`。
- 本仓库内的 Agent 可能运行在正式 Herdr 的终端里，`HC_TERMINAL_ID` 等变量指向正式 core。不要直接调用 PATH 上的 `hc` 或 `herdr-app` 去测试。
- 不推送、不建远端、不 sudo、不用 brew 装系统依赖、不用 `pkill`，也不申请新的系统隐私权限。这些操作都要先得到用户确认，见任务书 §03。
- 用户已确定 GUI 验收方式为人工验收加宿主自检，不使用屏幕录制或辅助功能权限。
- 宿主改动只以 `patches/herdr-app/*.patch` 为准，`.work/` 是可丢弃的工作副本。在 `.work/herdr-app` 的 `hup` 分支上提交，再执行 `scripts/prepare-upstream --export`。
- 四件事分开记录：静态检查、编译、单元测试、GUI 验收。没有人工确认的能力，写"实现但未完成 GUI 验收"。

## 入口（均支持 `--help`）

| 脚本 | 作用 |
|---|---|
| `scripts/doctor` | 只读检查本机、工具链、上游、测试环境和正式安装的指纹 |
| `scripts/bootstrap-toolchain` | 在项目内安装 Rust 1.95.0 和 Zig 0.16.0（`.local/toolchain`） |
| `scripts/prepare-upstream` | 克隆并校验固定提交，在 `hup` 分支上应用补丁；带 `--check` 做干净应用检查，带 `--export` 导出补丁 |
| `scripts/build` | 编译 `.work/herdr-app`，打包 `artifacts/apps.noindex/Herdr Preview Dev.app` |
| `scripts/dev` | 测试 core 和 App 的启停、`hc`、`app`、隔离检查与快照 |
| `scripts/test` | 项目测试（静态检查、单元测试、补丁检查；`--bridge` 跑桥接服务和鸿蒙核心逻辑的联调） |
| `scripts/bridge-service` / `scripts/tailscale-service` | 日常桥接服务和手机用的 Tailscale 节点（LaunchAgent，D27/D36） |

## 文档索引

`docs/ENVIRONMENT.md`（事实与隔离证据）· `docs/CAPABILITIES.md`（API 能力矩阵）· `docs/DECISIONS.md` · `docs/TASKS.md` · `docs/PROGRESS.md`（恢复时先读）· `docs/ACCEPTANCE.md` · `docs/SUPPORT_MATRIX.md`

## 布局

- `native/herdr-native-view`：原生渲染器 crate，与 GPUI 无关。
- `patches/herdr-app`：宿主粘合补丁。
- `mods/`：本项目的 Mods。
- `layouts/`：布局。
- `bridge/`：`ddup-bridge`，让鸿蒙 App 连到 Mac 上的 core（D27）。
- `mobile/harmony/`：鸿蒙版 DD-up，手机上显示名为“码上”（ArkTS，D33）。设计与安装说明：`docs/harmony/`。
- `tests/`：测试。
- `.local/`、`.work/`、`artifacts/`：均被忽略，不进 Git。
