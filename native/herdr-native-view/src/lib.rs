//! Native content views hosted inside a Herdr window (interface `native-view/0`).
//!
//! This crate knows nothing about GPUI. The host glue (patches/herdr-app) gives
//! it, per view: a key, the window's GPUI `NSView`, a JSON spec and, each frame,
//! the rectangle the mod element was painted at (window points, top-left
//! origin). Everything platform-neutral — spec validation, URL normalization,
//! the action whitelist and coordinate math — lives here so it can be tested
//! without a window.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub mod macos;

/// The interface version mods check before using any of this.
pub const INTERFACE: &str = "native-view/0";

/// A rectangle in window points, origin at the top-left (GPUI's convention).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        if x1 <= x0 || y1 <= y0 {
            Rect::ZERO
        } else {
            Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.w < 1.0 || self.h < 1.0
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// The same rectangle in a bottom-left-origin space of the given height
    /// (an unflipped AppKit view): returns (x, y, w, h).
    pub fn flipped(&self, height: f64) -> (f64, f64, f64, f64) {
        (self.x, height - self.y - self.h, self.w, self.h)
    }
}

/// What a view shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Web,
    Pdf,
    Media,
    QuickLook,
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind, String> {
        match s {
            "web" => Ok(Kind::Web),
            "pdf" => Ok(Kind::Pdf),
            "media" => Ok(Kind::Media),
            "quicklook" => Ok(Kind::QuickLook),
            other => Err(format!(
                "unknown native view kind `{other}` (web, pdf, media, quicklook)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Web => "web",
            Kind::Pdf => "pdf",
            Kind::Media => "media",
            Kind::QuickLook => "quicklook",
        }
    }

    /// The only actions a mod may perform on a view of this kind. No action
    /// runs a shell, evaluates script text or reads arbitrary files.
    pub fn actions(self) -> &'static [&'static str] {
        const COMMON: [&str; 5] = ["focus", "blur", "setVisible", "state", "dispose"];
        match self {
            Kind::Web => &[
                "focus",
                "blur",
                "setVisible",
                "state",
                "dispose",
                "browser.navigate",
                "browser.back",
                "browser.forward",
                "browser.reload",
                "browser.stop",
            ],
            Kind::Pdf => &[
                "focus",
                "blur",
                "setVisible",
                "state",
                "dispose",
                "pdf.goToPage",
                "pdf.next",
                "pdf.previous",
                "pdf.zoomIn",
                "pdf.zoomOut",
                "pdf.fitWidth",
            ],
            Kind::Media => &[
                "focus",
                "blur",
                "setVisible",
                "state",
                "dispose",
                "media.play",
                "media.pause",
                "media.seek",
                "media.setMuted",
                "media.setVolume",
            ],
            Kind::QuickLook => &COMMON,
        }
    }

    pub fn allows(self, action: &str) -> bool {
        self.actions().contains(&action)
    }
}

/// How much a local HTML file is trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Static preview: no page scripts. (Network blocking: see SUPPORT_MATRIX.)
    Static,
    /// The person enabled scripts for this file.
    Interactive,
}

/// Where a view's content comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A normalized http(s) URL.
    Url(String),
    /// A canonical local file, and (for HTML) the folder it may read from.
    File {
        path: PathBuf,
        read_root: PathBuf,
        trust: Trust,
    },
}

/// A validated request for one view.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub kind: Kind,
    pub source: Source,
    /// Web only: keep cookies and logins in this app's own persistent WebKit
    /// store (separate from Safari/Chrome). Default: a throwaway store.
    pub persist: bool,
}

impl Spec {
    /// Validates the props a mod passed: `{ kind, url }` or `{ kind, path, trust?, root? }`.
    pub fn parse(v: &Value) -> Result<Spec, String> {
        let persist = v.get("persist").and_then(Value::as_bool).unwrap_or(false);
        let kind = Kind::parse(
            v.get("kind")
                .and_then(Value::as_str)
                .ok_or("missing `kind`")?,
        )?;
        if let Some(url) = v.get("url").and_then(Value::as_str) {
            if kind != Kind::Web {
                return Err("`url` is only for kind web; local content uses `path`".into());
            }
            return Ok(Spec {
                kind,
                source: Source::Url(normalize_url(url)?),
                persist,
            });
        }
        let raw = v
            .get("path")
            .and_then(Value::as_str)
            .ok_or("missing `path` or `url`")?;
        let path = canonical_file(raw)?;
        let trust = match v.get("trust").and_then(Value::as_str) {
            None | Some("static") => Trust::Static,
            Some("interactive") => Trust::Interactive,
            Some(o) => return Err(format!("unknown trust `{o}`")),
        };
        // Sibling resources (CSS, images) may be read from `root`, which must
        // contain the file; default: the file's own folder.
        let parent = path
            .parent()
            .map(Path::to_path_buf)
            .ok_or("file has no folder")?;
        let read_root = match v.get("root").and_then(Value::as_str) {
            None => parent,
            Some(r) => {
                let r = std::fs::canonicalize(r).map_err(|e| format!("root: {e}"))?;
                if !path.starts_with(&r) {
                    return Err("`root` must contain the file".into());
                }
                r
            }
        };
        Ok(Spec {
            kind,
            source: Source::File {
                path,
                read_root,
                trust,
            },
            persist,
        })
    }

    /// A short description for logs: never a full URL query or file content.
    pub fn redacted(&self) -> String {
        match &self.source {
            Source::Url(u) => redact_url(u),
            Source::File { path, .. } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// An absolute path to an existing regular file, with symlinks and `..`
/// resolved. Relative paths are refused: they would be resolved against
/// whatever folder the app happens to run in.
pub fn canonical_file(raw: &str) -> Result<PathBuf, String> {
    let p = Path::new(raw);
    if !p.is_absolute() {
        return Err(format!("path must be absolute: {raw}"));
    }
    let c = std::fs::canonicalize(p).map_err(|e| format!("cannot open {}: {e}", p.display()))?;
    let meta = std::fs::metadata(&c).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("not a file: {}", c.display()));
    }
    Ok(c)
}

/// Turns what a person types in an address bar into a URL, without ever
/// turning it into a search. Loopback hosts default to http; everything else
/// to https. Only http and https are accepted.
pub fn normalize_url(input: &str) -> Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty address".into());
    }
    if s.chars().any(char::is_whitespace) {
        return Err("an address cannot contain spaces (search is not supported)".into());
    }
    if let Some((scheme, rest)) = s.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(format!("`{scheme}:` addresses are not opened here"));
        }
        if rest.is_empty() {
            return Err("missing host".into());
        }
        return Ok(format!("{scheme}://{rest}"));
    }
    // `mailto:x`, `javascript:...`, `file:/...`, `data:...` — a scheme with no
    // `//`. A bare `localhost:3000` also has a colon, so only refuse when what
    // precedes it is not a host.
    if let Some((head, _)) = s.split_once(':')
        && !head.is_empty()
        && !head.starts_with('[')
        && !looks_like_host(head)
    {
        return Err(format!("`{head}:` addresses are not opened here"));
    }
    let host = host_of(s);
    if host.is_empty() {
        return Err("missing host".into());
    }
    let scheme = if is_loopback(&host) { "http" } else { "https" };
    if !is_loopback(&host) && !host.contains('.') {
        return Err(format!("`{s}` is not an address (search is not supported)"));
    }
    Ok(format!("{scheme}://{s}"))
}

fn looks_like_host(head: &str) -> bool {
    head == "localhost" || head.contains('.') || head.parse::<std::net::Ipv4Addr>().is_ok()
}

fn host_of(s: &str) -> String {
    let authority = s.split(['/', '?', '#']).next().unwrap_or("");
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_ascii_lowercase();
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(h, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) {
                h
            } else {
                authority
            }
        })
        .to_ascii_lowercase()
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host == "::1" || host.starts_with("127.")
}

/// Scheme, host and path only: query strings and fragments can carry tokens.
pub fn redact_url(u: &str) -> String {
    u.split(['?', '#']).next().unwrap_or("").to_owned()
}

/// What the host can do, for `nativeViewCapabilities()`.
pub fn capabilities() -> Value {
    let kinds: Vec<Value> = [Kind::Web, Kind::Pdf, Kind::Media, Kind::QuickLook]
        .iter()
        .map(|k| json!({ "kind": k.name(), "actions": k.actions() }))
        .collect();
    json!({ "interface": INTERFACE, "platform": std::env::consts::OS, "available": cfg!(target_os = "macos"), "kinds": kinds })
}

/// One event a view reports to its mod. `generation` counts loads of the view,
/// so a late event from an earlier load can be told apart and dropped.
#[derive(Clone, Debug)]
pub struct Event {
    pub key: String,
    pub generation: u64,
    pub name: &'static str,
    pub data: Value,
}

impl Event {
    pub fn to_json(&self) -> Value {
        let id = self
            .key
            .split_once(':')
            .map_or(self.key.as_str(), |(_, id)| id);
        json!({ "id": id, "generation": self.generation, "event": self.name, "data": self.data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_never_become_searches() {
        assert_eq!(
            normalize_url("localhost:3000").unwrap(),
            "http://localhost:3000"
        );
        assert_eq!(
            normalize_url("127.0.0.1:5173/app").unwrap(),
            "http://127.0.0.1:5173/app"
        );
        assert_eq!(normalize_url("[::1]:8080").unwrap(), "http://[::1]:8080");
        assert_eq!(normalize_url("example.com").unwrap(), "https://example.com");
        assert_eq!(
            normalize_url("HTTP://example.com/a?b").unwrap(),
            "http://example.com/a?b"
        );
        assert_eq!(
            normalize_url("app.localhost:4000").unwrap(),
            "http://app.localhost:4000"
        );
        assert!(normalize_url("how to cook rice").is_err());
        assert!(normalize_url("rice").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn dangerous_schemes_are_refused() {
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hi",
            "mailto:a@b.c",
            "ftp://x.y",
            "vscode://x",
            "about:blank",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn redaction_drops_query_and_fragment() {
        assert_eq!(
            redact_url("https://x.y/p?token=s3cret#frag"),
            "https://x.y/p"
        );
    }

    #[test]
    fn rect_math() {
        let a = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 50.0,
        };
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            w: 60.0,
            h: 40.0,
        };
        assert_eq!(
            a.intersect(&clip),
            Rect {
                x: 10.0,
                y: 20.0,
                w: 50.0,
                h: 20.0
            }
        );
        assert!(
            a.intersect(&Rect {
                x: 500.0,
                y: 0.0,
                w: 1.0,
                h: 1.0
            })
            .is_empty()
        );
        // In an 800pt-high unflipped view the top edge at y=20 becomes 800-20-50.
        assert_eq!(a.flipped(800.0), (10.0, 730.0, 100.0, 50.0));
        assert!(a.contains(10.0, 20.0) && !a.contains(110.0, 20.0));
    }

    #[test]
    fn actions_are_whitelisted_per_kind() {
        assert!(Kind::Web.allows("browser.back"));
        assert!(!Kind::Web.allows("media.play"));
        assert!(!Kind::Pdf.allows("exec"));
        assert!(!Kind::QuickLook.allows("browser.navigate"));
    }

    #[test]
    fn specs_validate_paths() {
        let dir = std::env::temp_dir().join(format!("hup-spec-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let f = dir.join("sub/中文 #1 'q' %.html");
        std::fs::write(&f, "<p>x</p>").unwrap();
        let s = Spec::parse(&json!({ "kind": "web", "path": f.to_str().unwrap() })).unwrap();
        match &s.source {
            Source::File {
                path,
                read_root,
                trust,
            } => {
                assert_eq!(path, &std::fs::canonicalize(&f).unwrap());
                assert_eq!(read_root, &std::fs::canonicalize(dir.join("sub")).unwrap());
                assert_eq!(*trust, Trust::Static);
            }
            _ => panic!(),
        }
        assert!(Spec::parse(&json!({ "kind": "pdf", "path": "relative.pdf" })).is_err());
        assert!(
            Spec::parse(&json!({ "kind": "pdf", "path": dir.to_str().unwrap() })).is_err(),
            "a folder is not a file"
        );
        assert!(Spec::parse(&json!({ "kind": "pdf", "url": "https://x.y" })).is_err());
        assert!(Spec::parse(&json!({ "kind": "web", "path": f.to_str().unwrap(), "root": dir.join("sub/..").join("elsewhere").to_str().unwrap() })).is_err());
        let traversal = format!("{}/sub/../sub/中文 #1 'q' %.html", dir.display());
        assert!(Spec::parse(&json!({ "kind": "web", "path": traversal })).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn events_carry_id_and_generation() {
        let e = Event {
            key: "7:left".into(),
            generation: 3,
            name: "load.finish",
            data: json!({}),
        };
        assert_eq!(e.to_json()["id"], "left");
        assert_eq!(e.to_json()["generation"], 3);
    }
}
