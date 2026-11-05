# 进度

## 当前阶段

**2026-11-03：日用版改名为 DD-up，换上自己的图标**（D13）。旧的 Herdr Plus 已按清单卸载，DD-up 已安装到 `~/Applications/DD-up.app`，清单位于 `~/Library/Application Support/DD-up/install-manifest.json`。安装前在隔离环境里自检：13/13、7/7、Chromium 6/6；安装前后正式环境指纹不变。等用户退出官方 Herdr、打开 DD-up，并切换到 `dd-up` 布局（GUI 未经用户确认）。

**2026-11-03：已开源**，见 D14。https://github.com/obqyuxi/herdr-universal-preview 。官方 issue #425 已按用户要求暂时撤回（D19），等功能完善后再重新提交。


**2026-11-04：终端 Cmd+点击文件或文件夹链接，在 Files 面板中打开**（D23，补丁 0005）。已完成：上游单元测试 259 项通过，补丁可以干净应用，`check` 通过；在隔离开发版里用 `native.open-link` 自检了图片、文件夹外的中文名 md 和文件夹，并截图确认。真实的 Cmd+点击尚未经用户 GUI 验收。DD-up 已于当天重新安装（build `0.1.12-hup.67703df`，含补丁 0006，修复面板收起时点击无效的问题，在 DD-up 退出后安装，再重新打开，`status` 显示没有错误）。

**2026-11-05：右键终端里的文件或文件夹链接，可以选 "Reveal in Finder"**（D25，补丁 0007）。已完成：编译通过；上游测试通过；补丁可以干净应用；菜单 mod 通过 `check`；开发版里 `native.reveal` 对中文文件名返回 ok，相对路径和不存在的路径都被拒绝。尚未完成：右键菜单的 GUI 验收，以及 Finder 是否真的选中了文件（都要用户确认）。DD-up 要等用户退出后才能用 `scripts/install` 重新安装。

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
