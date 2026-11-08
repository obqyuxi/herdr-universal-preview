//! Web preview: which local dev servers are up, and a forward so the phone can
//! open one (docs/harmony/DESIGN.md §4.3).
//!
//! A forward is a plain-HTTP listener on a random port, reachable only from
//! near addresses like the bridge itself. The first request must carry
//! `?__ddup=<token>`; it is answered with a cookie and a redirect, and every
//! later request needs that cookie. Requests go to the dev server with `Host`
//! and `Origin` set to `localhost:<port>` (dev servers check them); WebSocket
//! upgrades (hot reload) are piped through as they are.

use crate::server::{is_near, log};
use anyhow::{Context as _, Result, bail};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PROBE_TIMEOUT: Duration = Duration::from_millis(600);
const HEAD_MAX: usize = 64 * 1024;
const PARAM: &str = "__ddup";

#[derive(Serialize, Debug, Clone)]
pub struct Site {
    pub port: u16,
    pub process: String,
    pub pid: u32,
    /// The process's working directory (which project it serves).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub status: u16,
}

/// `lsof -F pcn` output → (pid, command, port) for every listening TCP socket.
pub fn parse_lsof(out: &str) -> Vec<(u32, String, u16)> {
    let mut v = vec![];
    let (mut pid, mut cmd) = (0u32, String::new());
    for line in out.lines() {
        let (tag, rest) = line.split_at(line.len().min(1));
        match tag {
            "p" => pid = rest.parse().unwrap_or(0),
            "c" => cmd = rest.to_owned(),
            "n" => {
                if let Some(port) = rest.rsplit(':').next().and_then(|p| p.parse::<u16>().ok())
                    && !v.iter().any(|(_, _, p)| *p == port)
                {
                    v.push((pid, cmd.clone(), port));
                }
            }
            _ => {}
        }
    }
    v
}

fn title_of(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(a) = lower.find("<title") else { return String::new() };
    let Some(open_end) = lower[a..].find('>').map(|i| a + i + 1) else { return String::new() };
    let Some(close) = lower[open_end..].find("</title>").map(|i| open_end + i) else { return String::new() };
    html.get(open_end..close).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect()
}

async fn dial(port: u16) -> Option<TcpStream> {
    for addr in [format!("127.0.0.1:{port}"), format!("[::1]:{port}")] {
        if let Ok(Ok(s)) = tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(&addr)).await {
            return Some(s);
        }
    }
    None
}

/// Is it HTTP? Returns (status, page title).
async fn probe(port: u16) -> Option<(u16, String)> {
    let mut s = dial(port).await?;
    let req = format!("GET / HTTP/1.1\r\nHost: localhost:{port}\r\nAccept: text/html\r\nConnection: close\r\n\r\n");
    tokio::time::timeout(PROBE_TIMEOUT, s.write_all(req.as_bytes())).await.ok()?.ok()?;
    let mut buf = vec![0u8; 32 * 1024];
    let mut n = 0;
    let _ = tokio::time::timeout(PROBE_TIMEOUT, async {
        while n < buf.len() {
            match s.read(&mut buf[n..]).await {
                Ok(0) | Err(_) => break,
                Ok(k) => n += k,
            }
            if n >= 5 && !buf.starts_with(b"HTTP/") {
                break;
            }
        }
    })
    .await;
    let text = String::from_utf8_lossy(&buf[..n]);
    if !text.starts_with("HTTP/") {
        return None;
    }
    let status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Some((status, title_of(&text)))
}

async fn run(cmd: &str, args: &[&str]) -> String {
    let out = tokio::process::Command::new(cmd).args(args).output().await;
    out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// Listening local HTTP servers, without the bridge's own ports.
pub async fn list(skip: &[u16]) -> Vec<Site> {
    let out = run("/usr/sbin/lsof", &["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"]).await;
    let candidates: Vec<_> = parse_lsof(&out).into_iter().filter(|(_, _, p)| !skip.contains(p)).collect();
    let probes = candidates.iter().map(|(_, _, p)| probe(*p));
    let results = futures_util::future::join_all(probes).await;
    let mut sites: Vec<Site> = candidates
        .into_iter()
        .zip(results)
        .filter_map(|((pid, process, port), r)| r.map(|(status, title)| Site { port, process, pid, cwd: String::new(), title, status }))
        .collect();
    if !sites.is_empty() {
        let pids: Vec<String> = sites.iter().map(|s| s.pid.to_string()).collect();
        let out = run("/usr/sbin/lsof", &["-a", "-d", "cwd", "-p", &pids.join(","), "-Fpn"]).await;
        let mut cwd: HashMap<u32, String> = HashMap::new();
        let mut pid = 0;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix('p') {
                pid = p.parse().unwrap_or(0);
            } else if let Some(n) = line.strip_prefix('n') {
                cwd.insert(pid, n.to_owned());
            }
        }
        for s in &mut sites {
            s.cwd = cwd.get(&s.pid).cloned().unwrap_or_default();
        }
    }
    // System services (launched with cwd "/", like AirPlay's) aren't what you're building.
    sites.retain(|s| s.cwd != "/");
    // Pages with a title first, then the ones that answer at all.
    sites.sort_by_key(|s| (s.title.is_empty(), !(200..400).contains(&s.status), s.port));
    sites
}

#[derive(Clone)]
pub struct Forward {
    pub listen_port: u16,
    pub token: String,
}

/// Forwards by target port; they live as long as the bridge.
#[derive(Default)]
pub struct Forwards {
    map: Mutex<HashMap<u16, Forward>>,
}

impl Forwards {
    pub fn listen_ports(&self) -> Vec<u16> {
        self.map.lock().unwrap().values().map(|f| f.listen_port).collect()
    }

    pub async fn open(self: &Arc<Self>, bind_ip: &str, port: u16, allow_public: bool) -> Result<Forward> {
        if let Some(f) = self.map.lock().unwrap().get(&port) {
            return Ok(f.clone());
        }
        if dial(port).await.is_none() {
            bail!("端口 {port} 没有在监听");
        }
        let listener = TcpListener::bind((bind_ip, 0)).await.context("open a forward port")?;
        let mut t = [0u8; 18];
        OsRng.fill_bytes(&mut t);
        let token: String = t.iter().map(|b| format!("{b:02x}")).collect();
        let f = Forward { listen_port: listener.local_addr()?.port(), token: token.clone() };
        self.map.lock().unwrap().insert(port, f.clone());
        log(format!("forward :{} → localhost:{port}", f.listen_port));
        let lp = f.listen_port;
        tokio::spawn(async move {
            loop {
                let Ok((s, peer)) = listener.accept().await else { break };
                if !allow_public && !is_near(peer.ip()) {
                    continue;
                }
                let token = token.clone();
                tokio::spawn(async move {
                    if let Err(e) = proxy(s, peer, port, lp, &token).await {
                        log(format!("forward :{lp} {peer}: {e:#}"));
                    }
                });
            }
        });
        Ok(f)
    }
}

struct Head {
    method: String,
    target: String,
    version: String,
    headers: Vec<(String, String)>,
}

fn parse_head(raw: &[u8]) -> Option<Head> {
    let text = std::str::from_utf8(raw).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?.to_owned(), first.next()?.to_owned(), first.next()?.to_owned());
    let headers = lines.filter(|l| !l.is_empty()).filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))).collect();
    Some(Head { method, target, version, headers })
}

fn header<'a>(h: &'a Head, name: &str) -> Option<&'a str> {
    h.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// The token from `?__ddup=` and the target without it.
fn take_param(target: &str) -> (Option<String>, String) {
    let Some((path, query)) = target.split_once('?') else { return (None, target.to_owned()) };
    let mut token = None;
    let rest: Vec<&str> = query
        .split('&')
        .filter(|kv| match kv.strip_prefix(PARAM).and_then(|r| r.strip_prefix('=')) {
            Some(v) => {
                token = Some(v.to_owned());
                false
            }
            None => true,
        })
        .collect();
    let t = if rest.is_empty() { path.to_owned() } else { format!("{path}?{}", rest.join("&")) };
    (token, t)
}

fn cookie_name(lp: u16) -> String {
    format!("ddup_fwd_{lp}")
}

fn eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// The request head as sent to the dev server, or the reply for the phone.
enum Decision {
    Forward(Vec<u8>),
    Reply(String),
}

fn decide(h: &Head, port: u16, lp: u16, token: &str) -> Decision {
    let (param, target) = take_param(&h.target);
    let name = cookie_name(lp);
    if let Some(p) = param
        && eq(&p, token)
    {
        let loc = if target.is_empty() { "/".to_owned() } else { target };
        return Decision::Reply(format!(
            "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nSet-Cookie: {name}={token}; Path=/; HttpOnly; SameSite=Lax\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ));
    }
    let mut ok = false;
    let mut kept_cookies = vec![];
    for (k, v) in &h.headers {
        if k.eq_ignore_ascii_case("cookie") {
            for c in v.split(';').map(str::trim) {
                match c.split_once('=') {
                    Some((n, val)) if n == name => ok |= eq(val, token),
                    _ if !c.is_empty() => kept_cookies.push(c.to_owned()),
                    _ => {}
                }
            }
        }
    }
    if !ok {
        let body = "这个预览链接已失效，请在 DD-up 里重新打开。";
        return Decision::Reply(format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
    }
    let upgrade = header(h, "upgrade").is_some();
    let local = format!("localhost:{port}");
    let mut out = format!("{} {} {}\r\n", h.method, target, h.version);
    for (k, v) in &h.headers {
        let lk = k.to_ascii_lowercase();
        match lk.as_str() {
            "cookie" | "keep-alive" | "proxy-connection" => {}
            "connection" if !upgrade => {}
            "host" => out.push_str(&format!("Host: {local}\r\n")),
            "origin" => out.push_str(&format!("Origin: http://{local}\r\n")),
            "referer" => {
                let path = v.splitn(4, '/').nth(3).unwrap_or_default();
                out.push_str(&format!("Referer: http://{local}/{path}\r\n"));
            }
            _ => out.push_str(&format!("{k}: {v}\r\n")),
        }
    }
    if !kept_cookies.is_empty() {
        out.push_str(&format!("Cookie: {}\r\n", kept_cookies.join("; ")));
    }
    if !upgrade {
        // One request per connection keeps the proxy a plain pipe.
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    Decision::Forward(out.into_bytes())
}

async fn proxy(mut phone: TcpStream, _peer: SocketAddr, port: u16, lp: u16, token: &str) -> Result<()> {
    phone.set_nodelay(true)?;
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let end = loop {
        let n = tokio::time::timeout(Duration::from_secs(30), phone.read(&mut chunk)).await??;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > HEAD_MAX {
            bail!("request head too large");
        }
    };
    let Some(head) = parse_head(&buf[..end]) else { bail!("bad request") };
    match decide(&head, port, lp, token) {
        Decision::Reply(r) => {
            phone.write_all(r.as_bytes()).await?;
            let _ = phone.shutdown().await;
        }
        Decision::Forward(h) => {
            let Some(mut server) = dial(port).await else {
                let body = format!("localhost:{port} 没有响应（开发服务器停了？）");
                phone
                    .write_all(
                        format!(
                            "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await?;
                return Ok(());
            };
            server.set_nodelay(true)?;
            server.write_all(&h).await?;
            server.write_all(&buf[end..]).await?;
            let _ = tokio::io::copy_bidirectional(&mut phone, &mut server).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsof_output() {
        let out = "p100\ncnode\nf20\nn*:5173\nf21\nn[::1]:5173\np200\ncPython\nf3\nn127.0.0.1:8000\n";
        assert_eq!(parse_lsof(out), vec![(100, "node".into(), 5173), (200, "Python".into(), 8000)]);
    }

    #[test]
    fn titles() {
        assert_eq!(title_of("<html><head><TITLE>\n  My  App </TITLE>"), "My App");
        assert_eq!(title_of("no title"), "");
    }

    fn head(target: &str, extra: &str) -> Head {
        parse_head(format!("GET {target} HTTP/1.1\r\nHost: 100.1.2.3:5555\r\nOrigin: http://100.1.2.3:5555\r\n{extra}\r\n").as_bytes()).unwrap()
    }

    #[test]
    fn token_then_cookie() {
        let Decision::Reply(r) = decide(&head("/app?x=1&__ddup=tok", ""), 5173, 5555, "tok") else { panic!() };
        assert!(r.starts_with("HTTP/1.1 302") && r.contains("Location: /app?x=1\r\n") && r.contains("Set-Cookie: ddup_fwd_5555=tok;"), "{r}");
        let Decision::Reply(r) = decide(&head("/?__ddup=bad", ""), 5173, 5555, "tok") else { panic!() };
        assert!(r.starts_with("HTTP/1.1 403"));
        let Decision::Reply(r) = decide(&head("/", ""), 5173, 5555, "tok") else { panic!() };
        assert!(r.starts_with("HTTP/1.1 403"));
        let Decision::Forward(h) = decide(&head("/src/main.ts", "Cookie: a=1; ddup_fwd_5555=tok\r\nConnection: keep-alive\r\n"), 5173, 5555, "tok") else {
            panic!()
        };
        let h = String::from_utf8(h).unwrap();
        assert!(h.starts_with("GET /src/main.ts HTTP/1.1\r\n"), "{h}");
        assert!(h.contains("Host: localhost:5173\r\n") && h.contains("Origin: http://localhost:5173\r\n"), "{h}");
        assert!(h.contains("Cookie: a=1\r\n") && !h.contains("ddup_fwd") && h.contains("Connection: close\r\n") && !h.contains("keep-alive"), "{h}");
    }

    #[test]
    fn websocket_upgrade_kept() {
        let Decision::Forward(h) = decide(&head("/?token=x", "Cookie: ddup_fwd_5555=tok\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n"), 5173, 5555, "tok")
        else {
            panic!()
        };
        let h = String::from_utf8(h).unwrap();
        assert!(h.contains("Connection: Upgrade\r\n") && h.contains("Upgrade: websocket\r\n") && !h.contains("Connection: close"), "{h}");
    }
}
