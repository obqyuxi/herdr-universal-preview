<!--
Posted as herdrdev/herdr-app#425 (2026-11-03). Template "A mod can't do this yet" (.github/ISSUE_TEMPLATE/mod-api.yml).
Title below; the two headings are the template fields.
-->

**Title:** Mods can't host a native view (needed for PDF/HTML/media previews and a browser mod)

### What did you try to build?

Files previews that open PDF, Office documents, images, audio and video right in the panel (#91), and a side browser panel with persistent logins. I built both as ordinary mods, plus a layout, on a patched 0.1.12. The code is in https://github.com/obqyuxi/herdr-universal-preview:

- `mods/files-plus` is a copy of the shipped Files. On click it opens PDF (PDFKit), Office/iWork/RTF/images (Quick Look) and audio/video (AVKit) in the preview area. Text and code still use the read-only `Editor`.
- `mods/browser` is a browser panel with an address bar, back/forward/reload, find, zoom, print, downloads and history.
- The layout uses only `{ "mod": ..., "props": ... }` blocks, so a released app still parses it (the problem from #218).

The only part a mod can't do is the native view itself. I added that with a small host patch: 236 lines in 9 files. The Chromium engine adds about 30 more lines in vendored GPUI; the WebKit, PDFKit, AVKit and Quick Look views need nothing beyond the 236. The renderers live in a separate crate (`native/herdr-native-view`, objc2 0.6) so the host side stays thin.

### What is missing?

A mod element that hosts a platform view inside the element's own bounds. This is the shape suggested at the end of #218: "a mod element that hosts a native view, so a browser pane is an ordinary mod and the core only adds that one element".

What I implemented, as `native-view/0`:

- `herdr.nativeView.new(id, { kind, url | path })` is a component like `Editor.new`. `kind` is one of `web` (WKWebView with its own data store), `pdf`, `media` or `quicklook`.
- `nativeViewCapabilities()` returns the kinds and actions on this platform. On a build without the element, `herdr.nativeView` is `undefined`, so a mod can fall back.
- `nativeViewCall(id, action, args)` dispatches through a per-kind allowlist, e.g. `browser.back`, `pdf.goToPage`, `media.pause`, `dispose`.
- `nextNativeViewEvent()` is a long-poll in the style of `nextCommand`. Each event carries the view id and a load generation.

Host behaviour that made it work:

- **Frame sync.** The frame is taken at paint time from `bounds ∩ content_mask`. It is applied once at the end of the frame (y-flip, then `convertRect:toView:`), so it doesn't lag a frame behind (the issue in #235).
- **Visibility.** A view is hidden when its copy isn't shown this frame (`SHOWN_COPIES`) or a non-toast overlay or pane drag is active, and disposed when the copy goes away. The palette and menus therefore stay on top, and the view keeps its state while hidden.
- **Focus.** A mouse-down outside every native view returns first responder to the GPUI view, so terminal keys keep working.

If you're open to it, I can send this as a small PR: the element plus frame, visibility and focus handling, WebKit/PDFKit/AVKit/Quick Look only, macOS first, and nothing on other platforms. The Chromium engine I use for the browser would stay out of tree.

Happy to reshape the API to whatever fits your plans for the official browser mod.
