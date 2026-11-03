# 支持矩阵（M1 原型阶段）

列的含义：

- **形态**：Mod 表示在原版 Herdr 上可用；增强表示只在 Herdr Preview Dev（`nativeView`）中可用。
- **自检**：通过宿主 `native.debug` / `native.call` 得到的内部证据。
- **GUI**：用户在真实窗口中操作确认。

本表只记录实际测过的样例。"实现但未完成 GUI 验收"不代表可用。

| 类型 | 样例（生成方式见 `tests/fixtures/generate`） | 容器/编码 | 渲染器 | 形态 | 自检 | GUI | 限制 |
|---|---|---|---|---|---|---|---|
| 网页 HTTPS | https://example.com | — | WKWebView（非持久 data store） | 增强 | 加载、标题、后退 通过 | 未执行 | 需要联网 |
| 本地 HTML（交互） | `site/index.html` | UTF-8 | WKWebView + loadFileURL（读取范围限于所在目录） | 增强 | 加载、标题 通过 | 未执行 | — |
| 本地 HTML（静态） | 同上 | — | WKWebView，关闭页面脚本 | 增强 | 未测 | 未执行 | **网络尚未拦截**（M3 用 content rules 实现） |
| PDF | `sample-3pages.pdf`（3 页，CoreText 文本层，中英文） | PDF 1.x | PDFKit PDFView | 增强 | 跳页、下一页、越界拒绝、缩放 通过 | 未执行 | 密码 PDF 只报 `pdf.locked`，还不能输入密码 |
| PDF（损坏） | `corrupt-truncated.pdf` | — | PDFView | 增强 | 未测 | 未执行 | 预期 `load.error` |
| 视频 MP4 | `sample-h264-aac.mp4` | MP4 / avc1 640×360 / AAC 2ch 44.1k，6.00s | AVPlayerView | 增强 | 不自动播放、播放、拖动、暂停、静音、时长 6.0 通过 | 未执行 | — |
| 视频 MOV | `sample-h264-aac.mov` | QuickTime / avc1 / AAC | AVPlayerView | 增强 | 未测 | 未执行 | — |
| 音频 MP3 | `sample-speech.mp3`（LAME 128k） | MPEG-1 L3 | AVPlayerView | 增强 | 未测 | 未执行 | — |
| 音频 WAV | `sample-speech-pcm.wav` | LPCM 16-bit 44.1k 立体声 | AVPlayerView | 增强 | 未测 | 未执行 | — |
| 音频 M4A | `sample-speech-aac.m4a` | MP4 / AAC | AVPlayerView | 增强 | 未测 | 未执行 | — |
| DOCX | `sample-3pages.docx`（3 页，中文、表格） | OOXML | QLPreviewView | 增强 | 已加载预览项 | 未执行 | 只有 GUI 能确认后续页面 |
| XLSX | `sample-3sheets.xlsx`（3 表；公式没有缓存值） | OOXML | QLPreviewView | 增强 | 未测 | 未执行 | 不重算公式 |
| PPTX | `sample-4slides.pptx` | OOXML | QLPreviewView | 增强 | 未测 | 未执行 | 不保证动画 |
| 文本/代码/Markdown/图片 | — | — | M2 | Mod | — | — | M2 |

外部打开（`open`、`qlmanage`）本项目没有使用，也不计入窗口内支持。
