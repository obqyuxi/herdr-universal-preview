# 能力矩阵：需求 × 当前 API × 实现形态

依据：本机安装的 Herdr 0.1.12 生成的 `herdr.d.ts` / `gpui-kit.d.ts` / `AGENTS.md`，以及上游 `v0.1.12`（`4aa3460`）源码。

- 「普通 Mod」指在原版 Herdr 上不打补丁就能实现。
- 「增强版」指在本项目的个人构建（Herdr Preview Dev）中加入 `nativeView` 宿主扩展后才能实现。
- 实测列只写真正跑过的结果；没跑过的写"未测"。

## 当前已发布 API 中与本项目相关的部分（0.1.12）

| API | 位置 | 说明 |
|---|---|---|
| `readTextFile(path, {on})` | `herdr.d.ts:25`；`src/mods.rs:900` | UTF-8 文本，**上限 1 MiB**（`mod_files::LIMIT`，超过就报错，而不是截断）；二进制会按 lossy 方式解码 |
| `exec(cmd, args, {cwd,on})` | `src/mods.rs:896` | 运行进程，返回 stdout/stderr。本项目不用它执行用户内容 |
| `watchFolder(path)` | `src/mods.rs:1083` | 文件夹变化的一次性 Promise |
| `pickFolder()` | `herdr.d.ts:36` | 只能选文件夹，**没有选文件的接口** |
| `Editor` / `EditorState`（gpui-base） | 内置 Files `main.js:3` | 只读代码视图，带语言高亮（Files 已在使用） |
| `image(path)` / `svg(path)` | `gpui-kit.d.ts:20281-20290` | **只解析 App home 内的路径**；格式由 GPUI 图片加载器决定（SVG/PNG/JPEG 等，GIF/WebP 动画待测） |
| `openOverlay` / `notify` / `nextCommand` | `src/mods.rs:1045-1089` | 浮层、通知、命令通道 |
| 原生视图、WebView、PDF、媒体 | — | **0.1.12 没有**。Issue #218 已以 NOT_PLANNED 关闭；PR #235 是 Draft、未合并 |
| `HostModule::component` | `vendor/gpui-kit/crates/shell/src/host_modules.rs:445` | **宿主侧**机制：Rust 可以向 `herdr` 模块导出元素。0.1.12 的 `herdr` 模块没有用到它，本项目增强版用它实现 `nativeView` |

## 需求矩阵

| 需求 | 当前 API | 普通 Mod 可实现？ | 需要宿主扩展？ | 计划渲染器 | 实测 | 已知限制 |
|---|---|---|---|---|---|---|
| 文件树、发起打开 | `readTextFile`、`exec`、`watchFolder`；内置 Files 是参考 | 是 | 否 | universal-files mod | 未测（M2） | 远程走 `on` |
| 文本/代码只读高亮 | `Editor` readonly | 是（≤1 MiB） | 大文件的有界片段读取需要 range read 接口，或用 `exec head -c` 降级 | Editor | 未测（M2） | `readTextFile` 超过 1 MiB 直接报错 |
| JSON/CSV 视图 | 同上 | 是 | 否 | mod 内解析 | 未测 | — |
| Markdown 渲染 | GPUI 元素 | 部分：标题/列表/表格/代码块可以用 GPUI 元素绘制；本地图片受 `image()` 只认 home 路径的限制 | 图片需要宿主扩展，或在 home 内做有界缓存副本 | mod 渲染，或增强版用 WKWebView（禁用脚本） | 未测 | — |
| 图片 PNG/JPEG/SVG | `image()`（只认 home 内路径） | 部分：需要先复制到 home 内的缓存 | 原位显示需要宿主扩展 | `image()`，增强版可用 NSImageView/WKWebView | 未测 | GIF 动画、WebP 待测 |
| PDF 多页、选择复制 | 无 | 否 | 是 | PDFKit `PDFView` | 未测（M1） | 系统渲染缓存不受本项目控制 |
| DOCX/XLSX/PPTX | 无 | 否 | 是 | QuickLookUI `QLPreviewView` | 未测（M1） | 复杂特性不承诺 |
| 视频 MP4/MOV | 无 | 否 | 是 | AVKit `AVPlayerView` | 未测（M1） | 编解码器逐样例验证 |
| 音频 MP3/WAV/M4A | 只有 `playSound`（提示音，不能拖动进度） | 否：不能暂停、拖动、显示进度 | 是 | AVKit `AVPlayerView` | 未测（M1） | — |
| 网页 HTTP/HTTPS/localhost | 无 | 否 | 是 | WebKit `WKWebView`（独立的非持久 data store） | 未测（M1） | 不支持 Chrome 扩展、DRM、Passkey |
| 本地 HTML 两种信任模式 | 无 | 否 | 是 | WKWebView 配置分开 | 未测 | — |
| 选择项目外文件 | 只有 `pickFolder` | 只能选文件夹 | 选单个文件需要宿主扩展（NSOpenPanel） | — | 未测 | — |
| 项目隔离（machine+workspace） | `spaces()` / `focused()` | 是 | 否 | ResourceRef | 未测（M2） | — |

## 宿主扩展形态（本项目拟新增，不是现有 Herdr API）

接口版本 `native-view/0`。它通过 `herdr` 模块导出，Mod 用 `import * as herdr from "herdr"` 探测：原版上 `herdr.nativeView` 为 `undefined`，Mod 显示"当前版本未提供原生预览"，不会加载失败。

- 元素：`herdr.nativeView.new(id, { kind, url | path, trust?, root? })`，以组件形式导出（与 `Editor.new` 写法相同），宿主在 paint 阶段取得 bounds 并同步 NSView frame。
- 函数：`nativeViewCapabilities()`、`nativeViewCall(id, action, args)`（按渲染器白名单分发，例如 `browser.back`、`pdf.goToPage`、`media.pause`）、`nextNativeViewEvent()`（长轮询；每条事件带 view id 和加载代次）。
- 布局：只使用 `{ "mod": "...", "props": {...} }`，不新增 `Content` 变体，旧版 App 依旧能解析整份布局。
