//! Chromium (CEF 156) browser views inside the Herdr window (kind `chromium`).
//!
//! CEF runs in this process with an external message pump: a main-thread
//! timer calls `do_message_loop_work`, so GPUI keeps its own run loop. Each
//! browser is created as a child of the window's content view; the returned
//! NSView is then managed like every other native view (frame sync, hiding
//! under overlays, disposal). Helper processes live in
//! `Contents/Frameworks/<exe> Helper*.app` (see scripts/build).
//!
//! Profile: `$HUP_CHROMIUM_PROFILE`, else `~/Library/Application Support/
//! <executable>/Chromium`. Never the person's Chrome profile.

use cef::*;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::NSTimer;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What the handlers learn about one browser, read by `state`.
#[derive(Default)]
pub struct Info {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub find: Option<(i32, i32)>,
    /// Finished or running downloads: id -> full path chosen by the person.
    pub downloads: std::collections::HashMap<u32, String>,
}

pub struct ChromiumView {
    pub key: String,
    pub browser: Browser,
    pub info: Arc<Mutex<Info>>,
    pub generation: Arc<std::sync::atomic::AtomicU64>,
}

thread_local! {
    static STARTED: Cell<Option<bool>> = const { Cell::new(None) };
    static LOADER: RefCell<Option<library_loader::LibraryLoader>> = const { RefCell::new(None) };
    static PUMP: RefCell<Option<Retained<NSTimer>>> = const { RefCell::new(None) };
    static IN_PUMP: Cell<bool> = const { Cell::new(false) };
    /// A running "import Chrome cookies": who asked, and where the result arrives.
    static IMPORT: RefCell<Option<ImportJob>> = const { RefCell::new(None) };
}

type ImportJob = (
    String,
    Arc<std::sync::atomic::AtomicU64>,
    std::sync::mpsc::Receiver<Result<crate::chrome_import::ImportResult, String>>,
);

/// Applies a finished cookie import (main thread, from the pump).
fn finish_import() {
    let done = IMPORT.with(|i| {
        let mut i = i.borrow_mut();
        let r = i.as_ref().and_then(|(_, _, rx)| rx.try_recv().ok())?;
        let (key, generation, _) = i.take()?;
        Some((key, generation, r))
    });
    let Some((key, generation, r)) = done else { return };
    match r {
        Err(message) => report(&key, &generation, "import.error", json!({ "message": message })),
        Ok(res) => {
            let Some(manager) = cookie_manager_get_global_manager(None) else {
                report(&key, &generation, "import.error", json!({ "message": "浏览器的 Cookie 存储不可用" }));
                return;
            };
            let mut sites = std::collections::HashSet::new();
            let mut set = 0;
            for c in &res.cookies {
                let host = c.host.trim_start_matches('.');
                let path = if c.path.is_empty() { "/" } else { c.path.as_str() };
                let url = format!("{}://{}{}", if c.secure { "https" } else { "http" }, host, path);
                let cookie = Cookie {
                    name: CefString::from(c.name.as_str()),
                    value: CefString::from(c.value.as_str()),
                    domain: CefString::from(c.host.as_str()),
                    path: CefString::from(c.path.as_str()),
                    secure: c.secure as i32,
                    httponly: c.httponly as i32,
                    creation: Basetime { val: c.creation },
                    last_access: Basetime { val: c.last_access },
                    has_expires: c.has_expires as i32,
                    expires: Basetime { val: c.expires },
                    same_site: match c.samesite {
                        0 => CookieSameSite::NO_RESTRICTION,
                        1 => CookieSameSite::LAX_MODE,
                        2 => CookieSameSite::STRICT_MODE,
                        _ => CookieSameSite::UNSPECIFIED,
                    },
                    priority: CookiePriority::MEDIUM,
                    ..Default::default()
                };
                if manager.set_cookie(Some(&CefString::from(url.as_str())), Some(&cookie), None) != 0 {
                    set += 1;
                    sites.insert(host.trim_start_matches("www.").to_owned());
                }
            }
            manager.flush_store(None);
            let skipped = res.skipped + res.cookies.len() - set;
            report(&key, &generation, "import.done", json!({ "profile": res.profile, "cookies": set, "sites": sites.len(), "skipped": skipped }));
        }
    }
}

fn pump() {
    if IN_PUMP.with(|p| p.replace(true)) {
        return;
    }
    do_message_loop_work();
    finish_import();
    IN_PUMP.with(|p| p.set(false));
}

wrap_app! {
    struct HupApp;

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(HupBrowserProcess::new())
        }
    }
}

wrap_browser_process_handler! {
    struct HupBrowserProcess;

    impl BrowserProcessHandler {
        // The 60 Hz timer already pumps; nothing extra to schedule.
        fn on_schedule_message_pump_work(&self, _delay_ms: i64) {}
    }
}

fn bundle_paths() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = exe
        .file_name()
        .ok_or("no executable name")?
        .to_string_lossy()
        .into_owned();
    let contents = exe
        .parent()
        .and_then(|p| p.parent())
        .ok_or("not inside an app bundle")?
        .to_path_buf();
    let bundle = contents
        .parent()
        .ok_or("not inside an app bundle")?
        .to_path_buf();
    let frameworks = contents.join("Frameworks");
    let helper = frameworks.join(format!("{name} Helper.app/Contents/MacOS/{name} Helper"));
    if !frameworks
        .join("Chromium Embedded Framework.framework")
        .exists()
    {
        return Err("this build has no Chromium framework (scripts/build --chromium)".into());
    }
    if !helper.exists() {
        return Err(format!("missing Chromium helper: {}", helper.display()));
    }
    Ok((bundle, frameworks, helper))
}

fn profile_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("HUP_CHROMIUM_PROFILE") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    // Per app: the daily edition (e.g. "DD-up") and "HerdrPreviewDev" never share a profile.
    let app = std::env::current_exe().ok().and_then(|e| e.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "Herdr".into());
    home.join("Library/Application Support").join(app).join("Chromium")
}

/// Loads and initializes CEF once (main thread). Later calls return the first result.
pub fn ensure_started(_mtm: MainThreadMarker) -> Result<(), String> {
    match STARTED.with(Cell::get) {
        Some(true) => return Ok(()),
        Some(false) => return Err("Chromium failed to start earlier in this run".into()),
        None => {}
    }
    let r = start();
    STARTED.with(|s| s.set(Some(r.is_ok())));
    r
}

fn start() -> Result<(), String> {
    let (bundle, frameworks, helper) = bundle_paths()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let loader = library_loader::LibraryLoader::new(&exe, false);
    if !loader.load() {
        return Err("cannot load the Chromium framework".into());
    }
    LOADER.with(|l| *l.borrow_mut() = Some(loader));
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let profile = profile_dir();
    std::fs::create_dir_all(&profile).map_err(|e| format!("profile folder: {e}"))?;
    let args = args::Args::new();
    let settings = Settings {
        no_sandbox: 1,
        external_message_pump: 1,
        multi_threaded_message_loop: 0,
        main_bundle_path: CefString::from(bundle.to_string_lossy().as_ref()),
        framework_dir_path: CefString::from(
            frameworks
                .join("Chromium Embedded Framework.framework")
                .to_string_lossy()
                .as_ref(),
        ),
        browser_subprocess_path: CefString::from(helper.to_string_lossy().as_ref()),
        root_cache_path: CefString::from(profile.to_string_lossy().as_ref()),
        cache_path: CefString::from(profile.join("Default").to_string_lossy().as_ref()),
        persist_session_cookies: 1,
        log_severity: LogSeverity::WARNING,
        log_file: CefString::from(profile.join("chromium.log").to_string_lossy().as_ref()),
        ..Default::default()
    };
    let mut app = HupApp::new();
    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) != 1
    {
        return Err("Chromium did not initialize (see chromium.log in the profile folder)".into());
    }
    let block = block2::RcBlock::new(|_t: std::ptr::NonNull<NSTimer>| pump());
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0 / 60.0, true, &block) };
    PUMP.with(|p| *p.borrow_mut() = Some(timer));
    Ok(())
}

/// Creates a browser as a child of `parent` (the window's content view).
pub fn create(
    mtm: MainThreadMarker,
    key: &str,
    parent: &NSView,
    url: &str,
    generation: u64,
) -> Result<(Retained<NSView>, ChromiumView), String> {
    ensure_started(mtm)?;
    let info = Arc::new(Mutex::new(Info::default()));
    let gen_cell = Arc::new(std::sync::atomic::AtomicU64::new(generation));
    let mut window_info = WindowInfo::default().set_as_child(
        parent as *const NSView as *mut c_void as _,
        &Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
    );
    // Alloy style: on macOS, CEF refuses Chrome style for a browser with a
    // native parent ("Chrome style is not supported for this browser"; it
    // needs its own CEF Views window). So no chrome:// pages or extensions here.
    window_info.runtime_style = RuntimeStyle::ALLOY;
    let mut client = HupClient::new(key.to_owned(), info.clone(), gen_cell.clone());
    let browser = browser_host_create_browser_sync(
        Some(&window_info),
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&BrowserSettings::default()),
        None,
        None,
    )
    .ok_or("Chromium could not create a browser")?;
    let host = browser.host().ok_or("browser has no host")?;
    let handle = host.window_handle();
    // SAFETY: CEF returns the browser's NSView (a subview of `parent`); we retain it.
    let view: Retained<NSView> =
        unsafe { Retained::retain(handle as *mut NSView) }.ok_or("browser has no view")?;
    Ok((
        view,
        ChromiumView {
            key: key.to_owned(),
            browser,
            info,
            generation: gen_cell,
        },
    ))
}

impl ChromiumView {
    pub fn load(&self, url: &str, generation: u64) {
        self.generation
            .store(generation, std::sync::atomic::Ordering::Relaxed);
        if let Some(f) = self.browser.main_frame() {
            f.load_url(Some(&CefString::from(url)));
        }
    }

    pub fn state(&self) -> Value {
        let i = self.info.lock().unwrap();
        let zoom = self.browser.host().map(|h| h.zoom_level()).unwrap_or(0.0);
        json!({
            "url": crate::redact_url(&i.url),
            "title": i.title,
            "loading": i.loading,
            "canGoBack": i.can_back,
            "canGoForward": i.can_forward,
            "zoomLevel": zoom,
            "find": i.find.map(|(n, a)| json!({ "matches": n, "active": a })),
            "engine": "chromium",
        })
    }

    pub fn action(&self, action: &str, args: &Value) -> Result<Value, String> {
        let b = &self.browser;
        let host = b.host().ok_or("browser has no host")?;
        match action {
            "browser.back" => b.go_back(),
            "browser.forward" => b.go_forward(),
            "browser.reload" => b.reload(),
            "browser.stop" => b.stop_load(),
            "browser.zoomIn" => host.set_zoom_level(host.zoom_level() + 0.5),
            "browser.zoomOut" => host.set_zoom_level(host.zoom_level() - 0.5),
            "browser.zoomReset" => host.set_zoom_level(0.0),
            "browser.print" => host.print(),
            "browser.find" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("browser.find needs { text }")?;
                let forward = args.get("forward").and_then(Value::as_bool).unwrap_or(true);
                let next = args.get("next").and_then(Value::as_bool).unwrap_or(false);
                host.find(Some(&CefString::from(text)), forward as i32, 0, next as i32);
            }
            "browser.stopFind" => host.stop_finding(1),
            "browser.clearData" => {
                // Cookies (logins) and the HTTP cache of this app's own profile.
                if let Some(m) = cookie_manager_get_global_manager(None) {
                    m.delete_cookies(None, None, None);
                    m.flush_store(None);
                }
                host.execute_dev_tools_method(
                    0,
                    Some(&CefString::from("Network.clearBrowserCache")),
                    None,
                );
                host.execute_dev_tools_method(
                    0,
                    Some(&CefString::from("Network.clearBrowserCookies")),
                    None,
                );
            }
            "browser.importChromeCookies" => {
                if IMPORT.with(|i| i.borrow().is_some()) {
                    return Err("正在导入，请稍候".into());
                }
                let (tx, rx) = std::sync::mpsc::channel();
                // Off the main thread: the Keychain prompt waits for the person.
                std::thread::spawn(move || {
                    let _ = tx.send(crate::chrome_import::read());
                });
                IMPORT.with(|i| *i.borrow_mut() = Some((self.key.clone(), self.generation.clone(), rx)));
                return Ok(json!({ "started": true }));
            }
            "browser.revealDownload" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_u64)
                    .ok_or("browser.revealDownload needs { id }")? as u32;
                let path = self
                    .info
                    .lock()
                    .unwrap()
                    .downloads
                    .get(&id)
                    .cloned()
                    .ok_or("no such download")?;
                // Only reveals the file the person saved; never opens or runs it.
                let url = objc2_foundation::NSURL::fileURLWithPath(
                    &objc2_foundation::NSString::from_str(&path),
                );
                let list = objc2_foundation::NSArray::from_retained_slice(&[url]);
                objc2_app_kit::NSWorkspace::sharedWorkspace()
                    .activateFileViewerSelectingURLs(&list);
            }
            "focus" => host.set_focus(1),
            _ => return Err(format!("`{action}` is not available for Chromium views")),
        }
        Ok(self.state())
    }

    pub fn close(&self) {
        if let Some(h) = self.browser.host() {
            h.close_browser(1);
        }
    }
}

fn report(key: &str, generation: &std::sync::atomic::AtomicU64, name: &'static str, data: Value) {
    crate::macos::emit_event(
        key,
        generation.load(std::sync::atomic::Ordering::Relaxed),
        name,
        data,
    );
}

wrap_client! {
    struct HupClient {
        key: String,
        info: Arc<Mutex<Info>>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(HupDisplay::new(self.key.clone(), self.info.clone(), self.generation.clone()))
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(HupLoad::new(self.key.clone(), self.info.clone(), self.generation.clone()))
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(HupLifeSpan::new(self.key.clone(), self.info.clone(), self.generation.clone()))
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(HupDownload::new(self.key.clone(), self.info.clone(), self.generation.clone()))
        }
        fn find_handler(&self) -> Option<FindHandler> {
            Some(HupFind::new(self.key.clone(), self.info.clone(), self.generation.clone()))
        }
    }
}

wrap_display_handler! {
    struct HupDisplay {
        key: String,
        info: Arc<Mutex<Info>>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    }

    impl DisplayHandler {
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_some_and(|f| f.is_main() == 0) {
                return;
            }
            let url = url.map(|u| u.to_string()).unwrap_or_default();
            self.info.lock().unwrap().url = url.clone();
            report(&self.key, &self.generation, "load.commit", json!({ "url": url }));
        }
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let title = title.map(|t| t.to_string()).unwrap_or_default();
            self.info.lock().unwrap().title = title.clone();
            report(&self.key, &self.generation, "title", json!({ "title": title }));
        }
    }
}

wrap_load_handler! {
    struct HupLoad {
        key: String,
        info: Arc<Mutex<Info>>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    }

    impl LoadHandler {
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, is_loading: i32, can_go_back: i32, can_go_forward: i32) {
            let (url, title) = {
                let mut i = self.info.lock().unwrap();
                i.loading = is_loading != 0;
                i.can_back = can_go_back != 0;
                i.can_forward = can_go_forward != 0;
                (i.url.clone(), i.title.clone())
            };
            let data = json!({ "url": url, "title": title, "canGoBack": can_go_back != 0, "canGoForward": can_go_forward != 0 });
            report(&self.key, &self.generation, if is_loading != 0 { "load.start" } else { "load.finish" }, data);
        }
        fn on_load_error(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, error_code: Errorcode, error_text: Option<&CefString>, failed_url: Option<&CefString>) {
            if frame.is_some_and(|f| f.is_main() == 0) {
                return;
            }
            // ERR_ABORTED (-3): a navigation replaced by another; not a failure to show.
            let code: i32 = unsafe { std::mem::transmute_copy(&error_code) };
            if code == -3 {
                return;
            }
            let text = error_text.map(|t| t.to_string()).unwrap_or_default();
            let url = failed_url.map(|u| crate::redact_url(&u.to_string())).unwrap_or_default();
            report(&self.key, &self.generation, "load.error", json!({ "message": text, "code": code, "url": url }));
        }
    }
}

wrap_life_span_handler! {
    struct HupLifeSpan {
        key: String,
        info: Arc<Mutex<Info>>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    }

    impl LifeSpanHandler {
        // New windows and target=_blank open in this same view.
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            if user_gesture != 0
                && let (Some(b), Some(url)) = (browser, target_url)
                && let Some(f) = b.main_frame()
            {
                f.load_url(Some(url));
                report(&self.key, &self.generation, "web.newWindowInPlace", json!({}));
            }
            1
        }
    }
}

wrap_find_handler! {
    struct HupFind {
        key: String,
        info: Arc<Mutex<Info>>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    }

    impl FindHandler {
        fn on_find_result(&self, _browser: Option<&mut Browser>, _identifier: i32, count: i32, _selection_rect: Option<&Rect>, active_match_ordinal: i32, final_update: i32) {
            self.info.lock().unwrap().find = Some((count, active_match_ordinal));
            if final_update != 0 {
                report(&self.key, &self.generation, "find.result", json!({ "matches": count, "active": active_match_ordinal }));
            }
        }
    }
}

wrap_download_handler! {
    struct HupDownload {
        key: String,
        info: Arc<Mutex<Info>>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    }

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _method: Option<&CefString>) -> i32 {
            1
        }
        // The person always chooses where a download is saved (a save panel).
        fn on_before_download(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>, suggested_name: Option<&CefString>, callback: Option<&mut BeforeDownloadCallback>) -> i32 {
            let name = suggested_name.map(|n| n.to_string()).unwrap_or_default();
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            let suggested = home.join("Downloads").join(&name);
            if let Some(cb) = callback {
                cb.cont(Some(&CefString::from(suggested.to_string_lossy().as_ref())), 1);
            }
            let id = item.map(|i| i.id()).unwrap_or(0);
            report(&self.key, &self.generation, "download.start", json!({ "id": id, "name": name }));
            1
        }
        fn on_download_updated(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>, _callback: Option<&mut DownloadItemCallback>) {
            let Some(item) = item else { return };
            let id = item.id();
            let path = CefString::from(&item.full_path()).to_string();
            if !path.is_empty() {
                self.info.lock().unwrap().downloads.insert(id, path.clone());
            }
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| CefString::from(&item.suggested_file_name()).to_string());
            report(&self.key, &self.generation, "download.update", json!({
                "id": id,
                "name": name,
                "percent": item.percent_complete(),
                "received": item.received_bytes(),
                "total": item.total_bytes(),
                "complete": item.is_complete() != 0,
                "canceled": item.is_canceled() != 0,
            }));
        }
    }
}
