# 进度

## 当前阶段

M0 完成，M1 进行中：原生视图承载原型。

## 最新提交

见 `git log`。M0 的提交是 "M0: inventory, isolation, toolchain, baseline build"。

## 已验证（分层记录）

| 层 | 内容 | 结果 |
|---|---|---|
| 环境 | 本机、工具、正式 Herdr，见 ENVIRONMENT.md | 已核实 |
| 隔离 | `scripts/dev isolation-check`：测试 core 与正式 core 的 id/pid 不同，标记终端不串，正式环境指纹前后不变 | 通过 |
| 工具链 | rustc 1.95.0、zig 0.16.0，均安装在项目内 | 通过 |
| 上游 | v0.1.12 `4aa3460` + ghostty `291fd050`；补丁导出后重建，提交哈希可复现（`c2e66b1`） | 通过 |
| 编译 | v0.1.12 + 补丁 0001（跳过 ghostty xcframework），debug 构建 | 通过 |
| 上游单元测试 | `cargo test --locked`：258 通过；`notices` 失败，原因是 build.rs 指纹变化（预期，M1 中重新生成） | 部分 |
| 本项目单元测试 | `scripts/test`：native crate 7 项，clippy 与 rustfmt 无问题 | 通过 |
| GUI | 尚未启动任何 App | 未执行 |

## 下一条可执行动作

M1：完成宿主粘合补丁（`.work/herdr-app/src/native_view.rs` 等），然后执行：

```sh
scripts/prepare-upstream --export && scripts/build && scripts/dev app-up
```

## 中断恢复

1. 先读本文件、TASKS.md 和 `git status`，再执行 `scripts/doctor`。
2. `.work/herdr-app` 中未提交的改动属于未导出的开发工作。先在 `hup` 分支上提交，再执行 `scripts/prepare-upstream --export`，不要直接重建。
