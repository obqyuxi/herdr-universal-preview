# 任务表

状态取值：未开始 / 进行中 / 完成 / 阻断。"完成"指该任务的交付物已经存在、对应验证已经执行；GUI 验收单独记录在 ACCEPTANCE.md。

| ID | 阶段 | 任务 | 状态 | 阻断/备注 | 测试 ID |
|---|---|---|---|---|---|
| T0.1 | M0 | 盘点目录和 Git，移动任务书到 docs/ | 完成 | sha256 未变 `aeb7ea93…` | — |
| T0.2 | M0 | ENVIRONMENT.md / CAPABILITIES.md | 完成 | — | — |
| T0.3 | M0 | 项目内工具链（Rust 1.95.0、Zig 0.16.0） | 完成 | — | — |
| T0.4 | M0 | upstream.lock.json，以及 prepare-upstream 能重建固定提交 | 完成 | ghostty `291fd050` | A24（部分） |
| T0.5 | M0 | core 隔离验证 | 完成 | `scripts/dev isolation-check` | A22（core 部分） |
| T0.6 | M0 | 基线编译（v0.1.12 + 构建补丁 0001） | 完成 | 补丁前：ghostty 需要 xcodebuild（完整 Xcode），失败；加补丁 0001 后，只用 CLT 编译通过。上游 cargo test：258 通过，1 项 `notices` 因为 build.rs 指纹变化而失败（预期，M1 中重新生成 notices） | — |
| T0.7 | M0 | doctor / dev / build / test / prepare-upstream / bootstrap-toolchain 脚本 | 完成 | install/uninstall/rollback/package 在 M5 提供 | — |
| T1.1 | M1 | `native/herdr-native-view` crate：web/pdf/media/quicklook | 未开始 | — | — |
| T1.2 | M1 | 宿主补丁：`nativeView` 组件、函数、帧末同步、遮挡隐藏、开发版保护、`native.debug` | 未开始 | — | — |
| T1.3 | M1 | 测试样例生成（HTML/PDF/MP4/MOV/WAV/M4A/MP3/DOCX/XLSX/PPTX） | 未开始 | — | — |
| T1.4 | M1 | native-probe mod + 双预览布局 | 未开始 | — | — |
| T1.5 | M1 | 隔离启动 Herdr Preview Dev，宿主自检 | 未开始 | — | — |
| T1.6 | M1 | 人工 GUI 验收清单 + 结果 | 未开始 | 需要用户操作 | A16/A17/A18（原型级） |
| T2.x | M2 | universal-files、ResourceRef、路由、文本/Markdown/图片、标签规则、监听 | 未开始 | — | A01–A03, A12–A15 |
| T3.x | M3 | PDF/Office/媒体/浏览器完整化 | 未开始 | — | A04–A11 |
| T4.x | M4 | 稳定性、安全、兼容 | 未开始 | — | A16–A21, A23 |
| T5.x | M5 | 干净重建、安装、卸载、回退 | 未开始 | 正式安装前需用户批准 | A22, A24 |
