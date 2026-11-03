# Third-party notices

This repository contains original code (Apache-2.0, see LICENSE) plus patches
against, and build scripts for, the following projects. Their source is fetched
at build time into `.work/` and is not redistributed in this repository.

| Project | Use | License | Where verified |
|---|---|---|---|
| herdr-app (`herdrdev/herdr-app`, v0.1.12) | upstream host; `patches/herdr-app/*.patch` are derivative works of it | Apache-2.0 | `gh repo view`, upstream `LICENSE` |
| herdr-core (`herdrdev/herdr-core`, 0.1.17) | test core binary copied from the local install, never redistributed | Apache-2.0 | `gh repo view` |
| ghostty (submodule of herdr-app) | built by upstream as libghostty-vt | MIT | upstream `licenses/` (to re-check per release) |
| objc2 family crates (objc2, block2, objc2-*) | Rust bindings for AppKit/WebKit/PDFKit/AVKit/QuickLookUI in `native/` | MIT OR Apache-2.0 per crate (re-check when versions change) | crates.io metadata |
| CEF 156.0.15 (Chromium 156) | Chromium browser framework bundled in Herdr Preview Dev (`Contents/Frameworks`) | BSD-3-Clause (CEF) plus Chromium's third-party licenses | `CREDITS.html` from the CEF distribution (`.local/cef`), copied to `Contents/Resources/CEF-CREDITS.html` by `scripts/build` |
| cef, cef-dll-sys (tauri-apps/cef-rs 156.3.0) | Rust bindings for CEF in `native/` (feature `chromium`) | MIT OR Apache-2.0 | crates.io metadata, upstream THIRD-PARTY-NOTICES |
| serde, serde_json | JSON in `native/` | MIT OR Apache-2.0 | crates.io metadata |

Built bundles carry upstream's `LICENSE` and `THIRD-PARTY-NOTICES` in
`Contents/Resources/` (see `scripts/build`). Dependencies added by this project
are listed here and must be re-verified when versions change.
