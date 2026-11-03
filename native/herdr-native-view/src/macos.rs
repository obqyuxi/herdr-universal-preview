//! AppKit side: one native view per key, placed in the window's content view
//! just above GPUI's Metal view and moved to the rect its mod element painted.
//!
//! Everything here runs on the main thread (checked with `MainThreadMarker`).
//! State lives in thread-locals; AppKit callbacks never borrow the registry
//! while it is borrowed (events go through a separate sink).

use crate::{Event, Kind, Rect, Source, Spec, Trust};
use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::encode::{Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSEvent, NSEventMask, NSResponder, NSView, NSWindowOrderingMode,
};
use objc2_av_foundation::{AVPlayer, AVPlayerItemStatus};
use objc2_av_kit::AVPlayerView;
use objc2_core_media::CMTime;
use objc2_foundation::{
    NSError, NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize, NSString, NSURL,
    NSURLRequest,
};
use objc2_pdf_kit::{PDFDisplayMode, PDFDocument, PDFView, PDFViewPageChangedNotification};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewView, QLPreviewViewStyle};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationDelegate, WKUIDelegate, WKWebView,
    WKWebViewConfiguration, WKWebsiteDataStore, WKWindowFeatures,
};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

type Sink = Box<dyn Fn(Event)>;

thread_local! {
    static VIEWS: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static LOG: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}

/// Receives every view event (on the main thread).
pub fn set_event_sink(sink: impl Fn(Event) + 'static) {
    SINK.with(|s| *s.borrow_mut() = Some(Box::new(sink)));
}

/// For renderers outside this file (chromium.rs).
#[allow(dead_code)]
pub(crate) fn emit_event(key: &str, generation: u64, name: &'static str, data: Value) {
    emit(key, generation, name, data)
}

fn emit(key: &str, generation: u64, name: &'static str, data: Value) {
    let ev = Event {
        key: key.to_owned(),
        generation,
        name,
        data,
    };
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        l.push(json!({ "key": ev.key, "generation": generation, "event": name }));
        let n = l.len();
        if n > 200 {
            l.drain(..n - 200);
        }
    });
    SINK.with(|s| {
        if let Some(f) = s.borrow().as_ref() {
            f(ev);
        }
    });
}

enum Renderer {
    #[cfg(feature = "chromium")]
    Chromium(crate::chromium::ChromiumView),
    Web {
        view: Retained<WKWebView>,
        delegate: Retained<WebDelegate>,
    },
    Pdf {
        view: Retained<PDFView>,
        observer: Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    },
    Media {
        view: Retained<AVPlayerView>,
        player: Option<Retained<AVPlayer>>,
    },
    QuickLook {
        view: Retained<QLPreviewView>,
    },
}

struct Entry {
    kind: Kind,
    spec_json: Value,
    spec: Spec,
    generation: u64,
    /// The container that is moved and hidden (a `HoleHost`); `inner` is the
    /// renderer's own view inside it, which takes the keyboard.
    view: Retained<NSView>,
    host: Retained<HoleHost>,
    inner: Retained<NSView>,
    renderer: Renderer,
    /// GPUI's view in the window that owns this one, and the content view both live in.
    gpui: Weak<NSView>,
    container: Weak<NSView>,
    /// Where the mod element was last painted (window points, top-left).
    painted: Option<Rect>,
    /// What we last applied, to skip redundant AppKit calls.
    applied: Option<(Rect, bool)>,
    /// The mod asked for it to be hidden (`setVisible false`).
    wanted_hidden: bool,
    /// Its copy was on screen at the last frame end.
    shown: bool,
    /// Hidden because a Herdr overlay may be above it.
    suppressed: bool,
}

/// Creates the view for `key` if needed, or reloads it when the spec changed.
/// `gpui_view` is the window's GPUI `NSView` (raw-window-handle's `ns_view`).
/// Returns the load generation.
pub fn ensure(key: &str, gpui_view: *mut c_void, props: &Value) -> Result<u64, String> {
    let mtm = MainThreadMarker::new().ok_or("native views must be created on the main thread")?;
    let changed = VIEWS.with(|v| {
        v.borrow()
            .get(key)
            .map(|e| (e.spec_json != *props, e.generation))
    });
    match changed {
        Some((false, g)) => return Ok(g),
        Some((true, _)) => {}
        None => {}
    }
    let spec = Spec::parse(props)?;
    // Same kind: load the new content into the existing view (keeps its place).
    let reuse = VIEWS.with(|v| v.borrow().get(key).is_some_and(|e| e.kind == spec.kind));
    if reuse {
        let generation = VIEWS.with(|v| {
            let mut v = v.borrow_mut();
            let e = v.get_mut(key).unwrap();
            e.generation += 1;
            e.spec_json = props.clone();
            e.spec = spec.clone();
            e.generation
        });
        let r = load_entry(mtm, key, generation);
        if let Err(err) = &r {
            emit(key, generation, "load.error", json!({ "message": err }));
        }
        return Ok(generation);
    }
    dispose(key);
    // SAFETY: the pointer comes from raw-window-handle for a live GPUI window,
    // on the main thread; we only take a weak reference to it.
    let gpui: Retained<NSView> =
        unsafe { Retained::retain(gpui_view.cast::<NSView>()) }.ok_or("no GPUI view")?;
    let container = unsafe { gpui.superview() }.ok_or("GPUI view is not in a window")?;
    let generation = 1;
    let zero = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    // The host goes into the window first, so a Chromium browser can be created inside it.
    let host = HoleHost::new(mtm, zero);
    let view: Retained<NSView> = Retained::into_super(host.clone());
    view.setHidden(true);
    container.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Above, Some(&gpui));
    let (inner, renderer) = match make(mtm, key, &spec, zero, generation, &view) {
        Ok(r) => r,
        Err(e) => {
            view.removeFromSuperview();
            return Err(e);
        }
    };
    if unsafe { inner.superview() }.is_none_or(|s| !std::ptr::eq(Retained::as_ptr(&s), Retained::as_ptr(&view))) {
        view.addSubview(&inner);
    }
    inner.setFrame(view.bounds());
    inner.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    install_focus_monitor(mtm);
    let entry = Entry {
        kind: spec.kind,
        spec_json: props.clone(),
        spec: spec.clone(),
        generation,
        view,
        host,
        inner,
        renderer,
        gpui: Weak::from_retained(&gpui),
        container: Weak::from_retained(&container),
        painted: None,
        applied: None,
        wanted_hidden: false,
        shown: true,
        suppressed: false,
    };
    VIEWS.with(|v| v.borrow_mut().insert(key.to_owned(), entry));
    let r = load_entry(mtm, key, generation);
    emit(
        key,
        generation,
        "view.created",
        json!({ "kind": spec.kind.name() }),
    );
    if let Err(err) = r {
        emit(key, generation, "load.error", json!({ "message": err }));
    }
    Ok(generation)
}

fn make(
    mtm: MainThreadMarker,
    key: &str,
    spec: &Spec,
    frame: NSRect,
    generation: u64,
    parent: &NSView,
) -> Result<(Retained<NSView>, Renderer), String> {
    let _ = parent;
    if spec.kind == Kind::Chromium {
        #[cfg(feature = "chromium")]
        {
            let Source::Url(url) = &spec.source else {
                return Err("Chromium views take a `url`".into());
            };
            let (view, c) = crate::chromium::create(mtm, key, parent, url, generation)?;
            return Ok((view, Renderer::Chromium(c)));
        }
        #[cfg(not(feature = "chromium"))]
        return Err("this build has no Chromium".into());
    }
    unsafe {
        match spec.kind {
            Kind::Web => {
                let config = WKWebViewConfiguration::new(mtm);
                // Never the person's Safari/Chrome data: a throwaway store, or
                // (persist) this app's own one.
                let store = if spec.persist {
                    // This app's own persistent store (bundle dev.herdr.app.preview-dev):
                    // logins survive restarts; never Safari's or Chrome's data.
                    WKWebsiteDataStore::defaultDataStore(mtm)
                } else {
                    WKWebsiteDataStore::nonPersistentDataStore(mtm)
                };
                config.setWebsiteDataStore(&store);
                // Some sign-in pages refuse unknown embedded browsers; identify as Safari's engine.
                config.setApplicationNameForUserAgent(Some(&NSString::from_str(
                    "Version/26.0 Safari/605.1.15",
                )));
                config
                    .preferences()
                    .setJavaScriptCanOpenWindowsAutomatically(false);
                if let Source::File {
                    trust: Trust::Static,
                    ..
                } = spec.source
                {
                    config
                        .defaultWebpagePreferences()
                        .setAllowsContentJavaScript(false);
                }
                let view =
                    WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config);
                let delegate = WebDelegate::new(mtm, key, generation);
                view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                view.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                Ok((
                    Retained::into_super(view.clone()),
                    Renderer::Web { view, delegate },
                ))
            }
            Kind::Pdf => {
                let view = PDFView::initWithFrame(PDFView::alloc(mtm), frame);
                view.setAutoScales(true);
                view.setDisplayMode(PDFDisplayMode::SinglePageContinuous);
                let k = key.to_owned();
                let weak = Weak::from_retained(&view);
                let block = RcBlock::new(move |_n: NonNull<NSNotification>| {
                    if let Some(v) = weak.load() {
                        let g = VIEWS
                            .with(|m| {
                                m.try_borrow()
                                    .ok()
                                    .and_then(|m| m.get(&k).map(|e| e.generation))
                            })
                            .unwrap_or(0);
                        emit(&k, g, "pdf.page", pdf_state(&v));
                    }
                });
                let observer = NSNotificationCenter::defaultCenter()
                    .addObserverForName_object_queue_usingBlock(
                        Some(PDFViewPageChangedNotification),
                        Some(&view),
                        None,
                        &block,
                    );
                Ok((
                    Retained::into_super(view.clone()),
                    Renderer::Pdf {
                        view,
                        observer: Some(observer),
                    },
                ))
            }
            Kind::Media => {
                let view = AVPlayerView::initWithFrame(AVPlayerView::alloc(mtm), frame);
                Ok((
                    Retained::into_super(view.clone()),
                    Renderer::Media { view, player: None },
                ))
            }
            Kind::Chromium => Err("unreachable: Chromium handled above".into()),
            Kind::QuickLook => {
                let view = QLPreviewView::initWithFrame_style(
                    QLPreviewView::alloc(mtm),
                    frame,
                    QLPreviewViewStyle::Normal,
                )
                .ok_or("Quick Look preview view unavailable")?;
                view.setShouldCloseWithWindow(false);
                Ok((
                    Retained::into_super(view.clone()),
                    Renderer::QuickLook { view },
                ))
            }
        }
    }
}

fn file_url(path: &std::path::Path) -> Retained<NSURL> {
    // fileURLWithPath percent-encodes for us: spaces, `#`, `%`, quotes, CJK.
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn load(
    mtm: MainThreadMarker,
    key: &str,
    r: &Renderer,
    spec: &Spec,
    generation: u64,
) -> Result<Option<Retained<AVPlayer>>, String> {
    unsafe {
        match (r, &spec.source) {
            #[cfg(feature = "chromium")]
            (Renderer::Chromium(c), Source::Url(u)) => {
                c.load(u, generation);
                Ok(None)
            }
            (Renderer::Web { view, delegate }, src) => {
                delegate.ivars().generation.set(generation);
                match src {
                    Source::Url(u) => {
                        let url =
                            NSURL::URLWithString(&NSString::from_str(u)).ok_or("invalid URL")?;
                        view.loadRequest(&NSURLRequest::requestWithURL(&url));
                    }
                    Source::File {
                        path, read_root, ..
                    } => {
                        view.loadFileURL_allowingReadAccessToURL(
                            &file_url(path),
                            &file_url(read_root),
                        );
                    }
                }
                Ok(None)
            }
            (Renderer::Pdf { view, .. }, Source::File { path, .. }) => {
                let doc = PDFDocument::initWithURL(PDFDocument::alloc(), &file_url(path))
                    .ok_or("not a readable PDF")?;
                if doc.isLocked() {
                    view.setDocument(Some(&doc));
                    emit(key, generation, "pdf.locked", json!({}));
                    return Ok(None);
                }
                view.setDocument(Some(&doc));
                emit(key, generation, "load.finish", pdf_state(view));
                Ok(None)
            }
            (Renderer::Media { view, .. }, Source::File { path, .. }) => {
                let player = AVPlayer::playerWithURL(&file_url(path), mtm);
                // Never autoplay; the person presses play.
                player.pause();
                view.setPlayer(Some(&player));
                emit(key, generation, "load.finish", json!({}));
                Ok(Some(player))
            }
            (Renderer::QuickLook { view }, Source::File { path, .. }) => {
                let url = file_url(path);
                view.setPreviewItem(Some(ProtocolObject::<dyn QLPreviewItem>::from_ref(&*url)));
                emit(key, generation, "load.finish", json!({}));
                Ok(None)
            }
            _ => Err("this kind needs a local file".into()),
        }
    }
}

/// Loads an entry's current spec. The registry is borrowed only while AppKit
/// is told what to load, and a new media player is stored after that borrow
/// ends (nested borrows would panic, and a panic here aborts the app).
fn load_entry(mtm: MainThreadMarker, key: &str, generation: u64) -> Result<(), String> {
    let player = VIEWS.with(|v| {
        let v = v.borrow();
        let e = v.get(key).ok_or("view is gone")?;
        load(mtm, key, &e.renderer, &e.spec, generation)
    })?;
    if let Some(player) = player {
        VIEWS.with(|v| {
            if let Some(e) = v.borrow_mut().get_mut(key)
                && let Renderer::Media { player: p, .. } = &mut e.renderer
                && let Some(old) = p.replace(player)
            {
                unsafe {
                    old.pause();
                    old.replaceCurrentItemWithPlayerItem(None);
                }
            }
        });
    }
    Ok(())
}

fn pdf_state(view: &PDFView) -> Value {
    unsafe {
        let doc = view.document();
        let count = doc.as_ref().map_or(0, |d| d.pageCount());
        let page = match (doc.as_ref(), view.currentPage()) {
            (Some(d), Some(p)) => d.indexForPage(&p) as i64 + 1,
            _ => 0,
        };
        json!({ "page": page, "pageCount": count, "scale": view.scaleFactor(), "autoScales": view.autoScales(), "locked": doc.as_ref().is_some_and(|d| d.isLocked()) })
    }
}

fn secs(t: CMTime) -> Option<f64> {
    let s = unsafe { t.seconds() };
    s.is_finite().then_some(s)
}

fn media_state(player: Option<&AVPlayer>) -> Value {
    let Some(p) = player else {
        return json!({ "loaded": false });
    };
    unsafe {
        let item = p.currentItem();
        let status = item.as_ref().map(|i| i.status());
        json!({
            "loaded": item.is_some(),
            "status": match status { Some(AVPlayerItemStatus::ReadyToPlay) => "ready", Some(AVPlayerItemStatus::Failed) => "failed", Some(_) => "unknown", None => "none" },
            "error": item.as_ref().and_then(|i| i.error()).map(|e: Retained<NSError>| e.localizedDescription().to_string()),
            "rate": p.rate(),
            "time": secs(p.currentTime()),
            "duration": item.as_ref().and_then(|i| secs(i.duration())),
            "muted": p.isMuted(),
            "volume": p.volume(),
        })
    }
}

fn web_state(view: &WKWebView) -> Value {
    unsafe {
        json!({
            "url": view.URL().and_then(|u| u.absoluteString()).map(|s| crate::redact_url(&s.to_string())),
            "title": view.title().map(|s| s.to_string()),
            "loading": view.isLoading(),
            "progress": view.estimatedProgress(),
            "canGoBack": view.canGoBack(),
            "canGoForward": view.canGoForward(),
        })
    }
}

/// Records where the mod element was painted this frame.
pub fn painted(key: &str, rect: Rect) {
    VIEWS.with(|v| {
        if let Some(e) = v.borrow_mut().get_mut(key) {
            e.painted = Some(rect);
        }
    });
}

/// Applies frames and visibility at the end of a window's frame.
/// `alive(key)`: its mod copy still exists (else the view is disposed).
/// `shown(key)`: its copy is on screen now. `suppressed`: a Herdr overlay may
/// be drawn above native views, so they step aside (state kept).
pub fn frame_end(
    gpui_view: *mut c_void,
    alive: &dyn Fn(&str) -> bool,
    shown: &dyn Fn(&str) -> bool,
    suppressed: bool,
) {
    let Some(_mtm) = MainThreadMarker::new() else {
        return;
    };
    let dead: Vec<String> = VIEWS.with(|v| {
        v.borrow()
            .iter()
            .filter(|(k, e)| {
                e.gpui.load().is_some_and(|g| {
                    Retained::as_ptr(&g).cast::<c_void>() == gpui_view.cast_const()
                }) && !alive(k)
            })
            .map(|(k, _)| k.clone())
            .collect()
    });
    for k in dead {
        dispose(&k);
    }
    let mut paused = Vec::new();
    VIEWS.with(|v| {
        for (k, e) in v.borrow_mut().iter_mut() {
            let Some(gpui) = e.gpui.load() else { continue };
            if Retained::as_ptr(&gpui).cast::<c_void>() != gpui_view.cast_const() {
                continue;
            }
            let now_shown = shown(k);
            if e.shown
                && !now_shown
                && let Renderer::Media {
                    player: Some(p), ..
                } = &e.renderer
            {
                // Switching away pauses (brief §10.6); coming back doesn't resume.
                unsafe { p.pause() };
                paused.push((k.clone(), e.generation));
            }
            e.shown = now_shown;
            e.suppressed = suppressed;
            let rect = e.painted.unwrap_or(Rect::ZERO);
            let visible = now_shown && !suppressed && !e.wanted_hidden && !rect.is_empty();
            if e.applied == Some((rect, visible)) {
                continue;
            }
            e.applied = Some((rect, visible));
            if visible && let Some(container) = e.container.load() {
                let b = gpui.bounds();
                let (x, y, w, h) = if gpui.isFlipped() {
                    (rect.x, rect.y, rect.w, rect.h)
                } else {
                    rect.flipped(b.size.height)
                };
                let local = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
                let frame = gpui.convertRect_toView(local, Some(&container));
                e.view.setFrame(frame);
            }
            e.view.setHidden(!visible);
        }
    });
    for (k, g) in paused {
        emit(&k, g, "media.paused", json!({ "reason": "hidden" }));
    }
}

/// Runs one whitelisted action. Unknown actions are refused, never guessed.
pub fn call(key: &str, action: &str, args: &Value) -> Result<Value, String> {
    let mtm = MainThreadMarker::new().ok_or("main thread only")?;
    let kind = VIEWS.with(|v| v.borrow().get(key).map(|e| e.kind));
    // Importing Chrome's cookies doesn't need a page: the browser panel offers it
    // before the first address too.
    #[cfg(feature = "chromium")]
    if kind.is_none() && action == "browser.importChromeCookies" {
        return crate::chromium::import_chrome_cookies(mtm, key, None);
    }
    let kind = kind.ok_or_else(|| format!("no native view `{key}`"))?;
    if !kind.allows(action) {
        return Err(format!(
            "`{action}` is not an action of {} views",
            kind.name()
        ));
    }
    match action {
        // The mod no longer shows this element: release the view now.
        "dispose" => {
            dispose(key);
            return Ok(Value::Null);
        }
        "setVisible" => {
            let on = args
                .get("visible")
                .and_then(Value::as_bool)
                .ok_or("setVisible needs { visible: bool }")?;
            VIEWS.with(|v| {
                if let Some(e) = v.borrow_mut().get_mut(key) {
                    e.wanted_hidden = !on;
                    // Apply now: a call may come without a frame following it.
                    let can_show =
                        e.shown && !e.suppressed && e.painted.is_some_and(|r| !r.is_empty());
                    e.view.setHidden(!(on && can_show));
                    e.applied = None;
                }
            });
            return Ok(Value::Null);
        }
        // Cut-outs where the mod draws over the view (a menu card): the view
        // isn't drawn there and clicks there go to Herdr.
        "setHoles" => {
            let holes = parse_holes(args)?;
            VIEWS.with(|v| {
                if let Some(e) = v.borrow().get(key) {
                    e.host.set_holes(holes);
                }
            });
            return Ok(Value::Null);
        }
        _ => {}
    }
    let mut reload: Option<Value> = None;
    let out = VIEWS.with(|v| -> Result<Value, String> {
        let v = v.borrow();
        let e = &v[key];
        unsafe {
            match (action, &e.renderer) {
                #[cfg(feature = "chromium")]
                ("browser.navigate", Renderer::Chromium(_)) => {
                    let url = args
                        .get("url")
                        .and_then(Value::as_str)
                        .ok_or("browser.navigate needs { url }")?;
                    reload = Some(
                        json!({ "kind": "chromium", "url": crate::normalize_chromium_url(url)? }),
                    );
                    Ok(Value::Null)
                }
                #[cfg(feature = "chromium")]
                ("focus", Renderer::Chromium(c)) => {
                    if let Some(w) = e.view.window() {
                        w.makeFirstResponder(Some(&e.inner));
                    }
                    c.action("focus", args)
                }
                #[cfg(feature = "chromium")]
                (a, Renderer::Chromium(c)) if a.starts_with("browser.") => c.action(a, args),
                ("focus", _) => {
                    let w = e.view.window().ok_or("not in a window")?;
                    w.makeFirstResponder(Some(&e.inner));
                    Ok(Value::Null)
                }
                ("blur", _) => {
                    let w = e.view.window().ok_or("not in a window")?;
                    if let Some(g) = e.gpui.load() {
                        w.makeFirstResponder(Some(&g));
                    }
                    Ok(Value::Null)
                }
                ("state", r) => Ok(entry_state(e, r)),
                ("browser.navigate", Renderer::Web { .. }) => {
                    let url = args
                        .get("url")
                        .and_then(Value::as_str)
                        .ok_or("browser.navigate needs { url }")?;
                    reload = Some(json!({ "kind": "web", "url": crate::normalize_url(url)? }));
                    Ok(Value::Null)
                }
                ("browser.back", Renderer::Web { view, .. }) => Ok(json!(view.goBack().is_some())),
                ("browser.forward", Renderer::Web { view, .. }) => {
                    Ok(json!(view.goForward().is_some()))
                }
                ("browser.reload", Renderer::Web { view, .. }) => {
                    Ok(json!(view.reload().is_some()))
                }
                ("browser.stop", Renderer::Web { view, .. }) => {
                    view.stopLoading();
                    Ok(Value::Null)
                }
                ("pdf.next", Renderer::Pdf { view, .. }) => {
                    view.goToNextPage(None);
                    Ok(pdf_state(view))
                }
                ("pdf.previous", Renderer::Pdf { view, .. }) => {
                    view.goToPreviousPage(None);
                    Ok(pdf_state(view))
                }
                ("pdf.goToPage", Renderer::Pdf { view, .. }) => {
                    let n = args
                        .get("page")
                        .and_then(Value::as_u64)
                        .ok_or("pdf.goToPage needs { page } (1-based)")?;
                    let doc = view.document().ok_or("no document")?;
                    if n == 0 || n as usize > doc.pageCount() {
                        return Err(format!("page {n} is outside 1–{}", doc.pageCount()));
                    }
                    let page = doc.pageAtIndex(n as usize - 1).ok_or("no such page")?;
                    view.goToPage(&page);
                    Ok(pdf_state(view))
                }
                ("pdf.zoomIn", Renderer::Pdf { view, .. }) => {
                    view.zoomIn(None);
                    Ok(pdf_state(view))
                }
                ("pdf.zoomOut", Renderer::Pdf { view, .. }) => {
                    view.zoomOut(None);
                    Ok(pdf_state(view))
                }
                ("pdf.fitWidth", Renderer::Pdf { view, .. }) => {
                    view.setAutoScales(true);
                    Ok(pdf_state(view))
                }
                (
                    "media.play",
                    Renderer::Media {
                        player: Some(p), ..
                    },
                ) => {
                    p.play();
                    Ok(media_state(Some(p)))
                }
                (
                    "media.pause",
                    Renderer::Media {
                        player: Some(p), ..
                    },
                ) => {
                    p.pause();
                    Ok(media_state(Some(p)))
                }
                (
                    "media.seek",
                    Renderer::Media {
                        player: Some(p), ..
                    },
                ) => {
                    let t = args
                        .get("time")
                        .and_then(Value::as_f64)
                        .ok_or("media.seek needs { time } in seconds")?;
                    p.seekToTime(CMTime::with_seconds(t.max(0.0), 600));
                    Ok(media_state(Some(p)))
                }
                (
                    "media.setMuted",
                    Renderer::Media {
                        player: Some(p), ..
                    },
                ) => {
                    p.setMuted(
                        args.get("muted")
                            .and_then(Value::as_bool)
                            .ok_or("needs { muted }")?,
                    );
                    Ok(media_state(Some(p)))
                }
                (
                    "media.setVolume",
                    Renderer::Media {
                        player: Some(p), ..
                    },
                ) => {
                    let vol = args
                        .get("volume")
                        .and_then(Value::as_f64)
                        .ok_or("needs { volume } 0–1")?;
                    p.setVolume(vol.clamp(0.0, 1.0) as f32);
                    Ok(media_state(Some(p)))
                }
                (a, _) => Err(format!("`{a}` is unavailable right now")),
            }
        }
    })?;
    if let Some(spec) = reload {
        // Navigation through the address bar is a new load (new generation) of
        // the same view. `spec_json` keeps the mod's props, so the mod's next
        // render doesn't see a change and reload its own URL over this one.
        let spec = Spec::parse(&spec)?;
        let generation = VIEWS.with(|v| {
            let mut v = v.borrow_mut();
            let e = v.get_mut(key).ok_or("view is gone")?;
            e.generation += 1;
            e.spec = spec;
            Ok::<u64, String>(e.generation)
        })?;
        load_entry(mtm, key, generation)?;
        return Ok(json!({ "generation": generation }));
    }
    Ok(out)
}

fn entry_state(e: &Entry, r: &Renderer) -> Value {
    let content = match r {
        #[cfg(feature = "chromium")]
        Renderer::Chromium(c) => c.state(),
        Renderer::Web { view, .. } => web_state(view),
        Renderer::Pdf { view, .. } => pdf_state(view),
        Renderer::Media { player, .. } => media_state(player.as_deref()),
        Renderer::QuickLook { view } => {
            json!({ "hasItem": unsafe { view.previewItem() }.is_some() })
        }
    };
    let f = e.view.frame();
    let first = e
        .view
        .window()
        .and_then(|w| w.firstResponder())
        .is_some_and(|fr| responder_inside(&fr, &e.view));
    json!({
        "kind": e.kind.name(),
        "generation": e.generation,
        "source": e.spec.redacted(),
        "painted": e.painted.map(|r| json!([r.x, r.y, r.w, r.h])),
        "frame": [f.origin.x, f.origin.y, f.size.width, f.size.height],
        "hidden": e.view.isHidden(),
        "inWindow": e.view.window().is_some(),
        "shown": e.shown,
        "suppressed": e.suppressed,
        "wantedHidden": e.wanted_hidden,
        "holes": e.host.ivars().holes.borrow().len(),
        "holeClicksReachHerdr": hole_clicks_reach_herdr(e),
        "masked": layer_masked(&e.view),
        "shadow": e.host.ivars().shadow.borrow().as_ref().is_some_and(|sh| { let hidden: bool = unsafe { msg_send![&**sh, isHidden] }; !hidden }),
        "firstResponderInside": first,
        "content": content,
    })
}

fn layer_masked(view: &NSView) -> bool {
    let layer: Option<Retained<AnyObject>> = unsafe { msg_send![view, layer] };
    layer.is_some_and(|l| {
        let mask: Option<Retained<AnyObject>> = unsafe { msg_send![&*l, mask] };
        mask.is_some()
    })
}

/// For the self-check: a click at each hole's centre, hit-tested by the window
/// like a real one, lands outside this view (on Herdr). `None` without holes.
fn hole_clicks_reach_herdr(e: &Entry) -> Option<bool> {
    let holes = e.host.ivars().holes.borrow().clone();
    if holes.is_empty() || e.view.isHidden() {
        return None;
    }
    let window = e.view.window()?;
    let content = window.contentView()?;
    let frame_view = unsafe { content.superview() };
    let width = e.view.bounds().size.width;
    Some(holes.iter().all(|h| {
        let r = h.rect(width);
        let c = NSPoint::new(r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height / 2.0);
        let in_window = e.view.convertPoint_toView(c, None);
        let p = match &frame_view { Some(f) => f.convertPoint_fromView(in_window, None), None => in_window };
        content.hitTest(p).is_none_or(|v| !v.isDescendantOf(&e.view))
    }))
}

fn responder_inside(fr: &NSResponder, root: &NSView) -> bool {
    // A responder is "inside" when it is the view or one of its descendants.
    let Some(view) = fr.downcast_ref::<NSView>() else {
        return false;
    };
    view.isDescendantOf(root)
}

/// Removes a view and releases what it holds; late callbacks find nothing.
pub fn dispose(key: &str) {
    let Some(e) = VIEWS.with(|v| v.borrow_mut().remove(key)) else {
        return;
    };
    unsafe {
        match &e.renderer {
            #[cfg(feature = "chromium")]
            Renderer::Chromium(c) => c.close(),
            Renderer::Web { view, .. } => {
                view.stopLoading();
                view.setNavigationDelegate(None);
                view.setUIDelegate(None);
            }
            Renderer::Pdf { view, observer } => {
                if let Some(o) = observer {
                    NSNotificationCenter::defaultCenter().removeObserver(o.as_ref());
                }
                view.setDocument(None);
            }
            Renderer::Media { view, player } => {
                if let Some(p) = player {
                    p.pause();
                    p.replaceCurrentItemWithPlayerItem(None);
                }
                view.setPlayer(None);
            }
            Renderer::QuickLook { view } => view.close(),
        }
    }
    e.view.removeFromSuperview();
    emit(key, e.generation, "view.disposed", json!({}));
}

/// Disposes every view whose key matches.
pub fn dispose_where(pred: impl Fn(&str) -> bool) {
    let keys: Vec<String> =
        VIEWS.with(|v| v.borrow().keys().filter(|k| pred(k)).cloned().collect());
    for k in keys {
        dispose(&k);
    }
}

/// Everything the test harness wants to see, for `herdr-app do native.debug`.
pub fn debug() -> Value {
    let views: serde_json::Map<String, Value> = VIEWS.with(|v| {
        v.borrow()
            .iter()
            .map(|(k, e)| (k.clone(), entry_state(e, &e.renderer)))
            .collect()
    });
    json!({ "interface": crate::INTERFACE, "views": views, "count": views.len(), "monitor": MONITOR.with(|m| m.borrow().is_some()), "events": LOG.with(|l| l.borrow().clone()) })
}

/// A click outside every native view hands the keyboard back to GPUI, so a
/// terminal clicked after a web page gets the keys again.
fn install_focus_monitor(_mtm: MainThreadMarker) {
    if MONITOR.with(|m| m.borrow().is_some()) {
        return;
    }
    let block = RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
        let event = unsafe { ev.as_ref() };
        if let Some(window) = event.window(_mtm) {
            let p = event.locationInWindow();
            let mut inside = false;
            let mut clicked: Vec<(String, u64)> = Vec::new();
            let mut gpui_for_window: Option<Retained<NSView>> = None;
            VIEWS.with(|v| {
                if let Ok(v) = v.try_borrow() {
                    for (k, e) in v.iter() {
                        if e.view
                            .window()
                            .is_some_and(|w| std::ptr::eq(Retained::as_ptr(&w), &*window))
                        {
                            if gpui_for_window.is_none() {
                                gpui_for_window = e.gpui.load();
                            }
                            if !e.view.isHidden() {
                                let f = e.view.convertRect_toView(e.view.bounds(), None);
                                if p.x >= f.origin.x
                                    && p.y >= f.origin.y
                                    && p.x < f.origin.x + f.size.width
                                    && p.y < f.origin.y + f.size.height
                                {
                                    let local = e.view.convertPoint_fromView(p, None);
                                    if e.host.in_hole(local) {
                                        continue;
                                    }
                                    inside = true;
                                    if !e.host.ivars().holes.borrow().is_empty() {
                                        clicked.push((k.clone(), e.generation));
                                    }
                                }
                            }
                        }
                    }
                }
            });
            if !inside
                && let Some(g) = gpui_for_window
                && let Some(fr) = window.firstResponder()
                && !std::ptr::eq(Retained::as_ptr(&fr).cast::<NSView>(), Retained::as_ptr(&g))
                && fr.downcast_ref::<NSView>().is_some_and(|fv| {
                    !fv.isDescendantOf(&g)
                        && VIEWS.with(|v| {
                            v.try_borrow()
                                .is_ok_and(|v| v.values().any(|e| fv.isDescendantOf(&e.view)))
                        })
                })
            {
                window.makeFirstResponder(Some(&g));
            }
            // Outside the card, on the page: the mod closes its menu, like a popover.
            for (k, g) in clicked {
                emit(&k, g, "view.click", json!({}));
            }
        }
        ev.as_ptr()
    });
    let mask =
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    MONITOR.with(|m| *m.borrow_mut() = monitor);
}

pub struct WebIvars {
    key: String,
    generation: Cell<u64>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = WebIvars]
    struct WebDelegate;

    unsafe impl NSObjectProtocol for WebDelegate {}

    unsafe impl WKNavigationDelegate for WebDelegate {
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start(&self, web: &WKWebView, _nav: Option<&WKNavigation>) {
            self.report("load.start", web_state(web));
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, web: &WKWebView, _nav: Option<&WKNavigation>) {
            self.report("load.commit", web_state(web));
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web: &WKWebView, _nav: Option<&WKNavigation>) {
            self.report("load.finish", web_state(web));
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, web: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            let mut s = web_state(web);
            s["message"] = json!(error.localizedDescription().to_string());
            s["code"] = json!(error.code());
            self.report("load.error", s);
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(
            &self,
            web: &WKWebView,
            _nav: Option<&WKNavigation>,
            error: &NSError,
        ) {
            let mut s = web_state(web);
            s["message"] = json!(error.localizedDescription().to_string());
            s["code"] = json!(error.code());
            self.report("load.error", s);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn did_terminate(&self, _web: &WKWebView) {
            self.report("web.processTerminated", json!({}));
        }
    }

    unsafe impl WKUIDelegate for WebDelegate {
        // target=_blank and window.open: open in this same view rather than a
        // new window. Only user-initiated ones reach here, since
        // javaScriptCanOpenWindowsAutomatically is off.
        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            web: &WKWebView,
            _config: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> *mut WKWebView {
            unsafe {
                if action.targetFrame().is_none() {
                    web.loadRequest(&action.request());
                    self.report("web.newWindowInPlace", json!({}));
                }
            }
            std::ptr::null_mut()
        }
    }
);

impl WebDelegate {
    fn new(mtm: MainThreadMarker, key: &str, generation: u64) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WebIvars {
            key: key.to_owned(),
            generation: Cell::new(generation),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn report(&self, name: &'static str, data: Value) {
        let i = self.ivars();
        emit(&i.key, i.generation.get(), name, data);
    }
}

/// One cut-out, in the view's top-left coordinates. `right` anchors it to the
/// right edge, so it stays put when the view changes width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hole {
    x: Option<f64>,
    right: Option<f64>,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
}

impl Hole {
    fn rect(&self, width: f64) -> NSRect {
        let x = match (self.x, self.right) {
            (Some(x), _) => x,
            (None, Some(r)) => width - r - self.w,
            (None, None) => 0.0,
        };
        NSRect::new(NSPoint::new(x, self.y), NSSize::new(self.w, self.h))
    }
}

/// `{ holes: [{ x | right, y, w, h, radius? }] }`, at most 8; `[]` clears.
pub(crate) fn parse_holes(args: &Value) -> Result<Vec<Hole>, String> {
    let list = args.get("holes").and_then(Value::as_array).ok_or("setHoles needs { holes: [...] }")?;
    if list.len() > 8 {
        return Err("at most 8 holes".into());
    }
    let num = |h: &Value, k: &str| -> Result<Option<f64>, String> {
        match h.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0 && *n < 100_000.0)
                .map(Some)
                .ok_or_else(|| format!("hole `{k}` must be a number ≥ 0")),
        }
    };
    list.iter()
        .map(|h| {
            Ok(Hole {
                x: num(h, "x")?,
                right: num(h, "right")?,
                y: num(h, "y")?.unwrap_or(0.0),
                w: num(h, "w")?.ok_or("hole needs `w`")?,
                h: num(h, "h")?.ok_or("hole needs `h`")?,
                radius: num(h, "radius")?.unwrap_or(0.0),
            })
        })
        .collect()
}

#[repr(C)]
pub struct CGPath {
    _private: [u8; 0],
}

// SAFETY: CGPathRef is a pointer to the opaque struct CGPath.
unsafe impl RefEncode for CGPath {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[]));
}

#[repr(C)]
pub struct CGColor {
    _private: [u8; 0],
}

// SAFETY: CGColorRef is a pointer to the opaque struct CGColor.
unsafe impl RefEncode for CGColor {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorCreateGenericGray(gray: f64, alpha: f64) -> *mut CGColor;
    fn CGColorRelease(color: *mut CGColor);
    fn CGPathCreateMutable() -> *mut CGPath;
    fn CGPathAddRect(path: *mut CGPath, m: *const c_void, rect: NSRect);
    fn CGPathAddRoundedRect(path: *mut CGPath, m: *const c_void, rect: NSRect, w: f64, h: f64);
    fn CGPathRelease(path: *mut CGPath);
}

pub struct HostIvars {
    holes: RefCell<Vec<Hole>>,
    /// Soft shadow around the holes, drawn on the page (the hole itself is
    /// masked away, so the shadow only shows outside the card).
    shadow: RefCell<Option<Retained<AnyObject>>>,
}

define_class!(
    /// Holds one native view. Where the mod draws a card over it, a hole lets
    /// Herdr's drawing (below) show and receive the clicks.
    // SAFETY: NSView subclass; overrides keep NSView's contracts (super is called).
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostIvars]
    pub struct HoleHost;

    unsafe impl NSObjectProtocol for HoleHost {}

    impl HoleHost {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            // `point` is in the superview's coordinates.
            if let Some(sup) = unsafe { self.superview() } {
                let local = self.convertPoint_fromView(point, Some(&sup));
                if self.in_hole(local) {
                    return std::ptr::null_mut();
                }
            }
            unsafe { msg_send![super(self), hitTest: point] }
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.update_mask();
        }
    }
);

impl HoleHost {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HostIvars { holes: RefCell::new(Vec::new()), shadow: RefCell::new(None) });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        this.setWantsLayer(true);
        this
    }

    fn in_hole(&self, p: NSPoint) -> bool {
        let width = self.bounds().size.width;
        self.ivars().holes.borrow().iter().any(|h| {
            let r = h.rect(width);
            p.x >= r.origin.x && p.y >= r.origin.y && p.x < r.origin.x + r.size.width && p.y < r.origin.y + r.size.height
        })
    }

    fn set_holes(&self, holes: Vec<Hole>) {
        if *self.ivars().holes.borrow() == holes {
            return;
        }
        *self.ivars().holes.borrow_mut() = holes;
        self.update_mask();
    }

    /// A shape mask: the whole view minus the holes (even-odd fill).
    fn update_mask(&self) {
        let holes = self.ivars().holes.borrow();
        let layer: Option<Retained<AnyObject>> = unsafe { msg_send![self, layer] };
        let Some(layer) = layer else { return };
        if holes.is_empty() {
            let none: Option<&AnyObject> = None;
            let _: () = unsafe { msg_send![&*layer, setMask: none] };
            if let Some(sh) = self.ivars().shadow.borrow().as_ref() {
                let _: () = unsafe { msg_send![&**sh, setHidden: true] };
            }
            return;
        }
        let Some(cls) = AnyClass::get(c"CAShapeLayer") else { return };
        let b = self.bounds();
        // The view is flipped (top-left); its layer may or may not be.
        let layer_flipped: bool = unsafe { msg_send![&*layer, isGeometryFlipped] };
        unsafe {
            let path = CGPathCreateMutable();
            CGPathAddRect(path, std::ptr::null(), b);
            for h in holes.iter() {
                let mut r = h.rect(b.size.width);
                if !layer_flipped {
                    r.origin.y = b.size.height - r.origin.y - r.size.height;
                }
                let rad = h.radius.min(r.size.width / 2.0).min(r.size.height / 2.0);
                CGPathAddRoundedRect(path, std::ptr::null(), r, rad, rad);
            }
            let shape: Retained<AnyObject> = msg_send![cls, layer];
            let _: () = msg_send![&*shape, setFrame: b];
            let _: () = msg_send![&*shape, setPath: path.cast_const()];
            let rule = NSString::from_str("even-odd");
            let _: () = msg_send![&*shape, setFillRule: &*rule];
            let _: () = msg_send![&*layer, setMask: &*shape];
            CGPathRelease(path);
            self.update_shadow(&layer, &holes, b, layer_flipped);
        }
    }

    unsafe fn update_shadow(&self, layer: &AnyObject, holes: &[Hole], b: NSRect, layer_flipped: bool) {
        let Some(cls) = AnyClass::get(c"CALayer") else { return };
        let mut slot = self.ivars().shadow.borrow_mut();
        if slot.is_none() {
            let sh: Retained<AnyObject> = unsafe { msg_send![cls, layer] };
            unsafe {
                let color = CGColorCreateGenericGray(0.0, 1.0);
                let _: () = msg_send![&*sh, setShadowColor: color.cast_const()];
                CGColorRelease(color);
                let _: () = msg_send![&*sh, setShadowOpacity: 0.18f32];
                let _: () = msg_send![&*sh, setShadowRadius: 14.0f64];
                let _: () = msg_send![&*sh, setZPosition: 1000.0f64];
                let _: () = msg_send![layer, addSublayer: &*sh];
            }
            *slot = Some(sh);
        }
        let sh = slot.as_ref().unwrap();
        unsafe {
            let path = CGPathCreateMutable();
            for h in holes {
                let mut r = h.rect(b.size.width);
                if !layer_flipped {
                    r.origin.y = b.size.height - r.origin.y - r.size.height;
                }
                let rad = h.radius.min(r.size.width / 2.0).min(r.size.height / 2.0);
                CGPathAddRoundedRect(path, std::ptr::null(), r, rad, rad);
            }
            // Down the screen: +y when the layer is flipped, -y when it isn't.
            let _: () = msg_send![&**sh, setShadowOffset: NSSize::new(0.0, if layer_flipped { 6.0 } else { -6.0 })];
            let _: () = msg_send![&**sh, setFrame: b];
            let _: () = msg_send![&**sh, setShadowPath: path.cast_const()];
            let _: () = msg_send![&**sh, setHidden: false];
            CGPathRelease(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holes_parse_and_follow_the_right_edge() {
        let h = parse_holes(&json!({ "holes": [{ "right": 10, "y": 6, "w": 300, "h": 200, "radius": 12 }] })).unwrap();
        let r = h[0].rect(800.0);
        assert_eq!((r.origin.x, r.origin.y, r.size.width, r.size.height), (490.0, 6.0, 300.0, 200.0));
        assert!(parse_holes(&json!({ "holes": [] })).unwrap().is_empty());
        assert!(parse_holes(&json!({ "holes": [{ "w": -1, "h": 2 }] })).is_err());
        assert!(parse_holes(&json!({ "holes": [{ "w": 1 }] })).is_err());
        assert!(parse_holes(&json!({})).is_err());
    }
}
