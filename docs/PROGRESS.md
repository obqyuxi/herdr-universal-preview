# 进度

## 当前阶段

**2026-11-02：已安装为日用版 Herdr Plus**，见 INSTALL.md 和 ROLLBACK.md。安装清单位于 `~/Library/Application Support/HerdrPlus/install-manifest.json`。等用户退出官方 Herdr、打开 Herdr Plus，并切换到 `herdr-plus` 布局。


**用户 2026-11-02 收窄范围**：只要两件事，验收以这两件事为准。

1. Files 面板点 Word/Excel/PPT/PDF/图片/音视频就能直接看（`mods/files-plus`）。
2. 顶栏"浏览器"按钮打开侧边浏览器：输入网址打开，登录状态保留（`mods/browser`，WebKit 持久存储，属于开发版自己）。

入口：布局 `herdr-preview`，由 `scripts/dev app-install` 安装到测试 home。原先的 M1 全量清单（G1–G24）暂缓。

（以下为原 M1 记录）

M1（原生视图承载原型）：代码和宿主自检已完成，**正在等待人工 GUI 验收**（`tests/gui/M1_CHECKLIST.md`）。

## 最新提交

见 `git log`：先是 M0，然后是 "M1: native view hosting prototype (self-checked, GUI acceptance pending)"。

## 已验证

- M0 的全部内容（见 ENVIRONMENT.md）。
- M1：
  - 补丁 0001–0002 能在固定提交上重建；
  - 编译通过；
  - 上游测试全部通过；
  - native crate 单元测试通过；
  - 开发版和正式 0.1.12 都通过了 `check`；
  - 宿主自检 13/13 与 7/7；
  - 命令面板遮挡时视图让位，关闭后恢复且状态保留；
  - 每次运行 App 前后，正式环境指纹都不变。
- **未验证**：任何需要人眼或人耳确认的 GUI 项（G1–G24）。

## 下一条可执行动作

```sh
scripts/dev app-up              # 如果没在运行
tests/integration/native-selfcheck && tests/integration/native-selfcheck-media
# 然后请用户按 tests/gui/M1_CHECKLIST.md 操作，把结果写入 ACCEPTANCE.md
```

M1 GUI 验收期间可以并行推进的 M2 工作：

- 复制 Files，做出 universal-files；
- ResourceRef 和路由；
- Markdown：可以使用 gpui-kit 的 `TextView`，它支持 markdown 和 html 格式，是普通 Mod 版的候选方案。

## 中断恢复

1. 先读本文件、TASKS.md 和 `git status`，再执行 `scripts/doctor`。
2. `.work/herdr-app` 中未提交的改动属于未导出的开发工作。先在 `hup` 分支上提交，再执行 `scripts/prepare-upstream --export`。
