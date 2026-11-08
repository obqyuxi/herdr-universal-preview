//! `ddup-bridge serve`: accept the phone, run the DDB1 handshake, check its
//! device, then relay `hc connect` both ways (docs/harmony/DESIGN.md §3–4).
//! Also runs the push watcher.

use crate::crypto::{self, Half, Records};
use crate::ports::Forwards;
use crate::push::{self, Pusher};
use crate::state::{Config, PairError, Store};
use crate::watch::{Alert, Tracker};
use anyhow::{Context as _, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use x25519_dalek::StaticSecret;

pub const HC: u8 = 0x00;
pub const CONTROL: u8 = 0x01;
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const PING_EVERY: Duration = Duration::from_secs(25);

pub fn log(msg: impl AsRef<str>) {
    eprintln!("[{}] {}", crate::state::now_ms() / 1000, msg.as_ref());
}

pub struct Ctx {
    pub store: Store,
    pub config: Config,
    pub identity: StaticSecret,
    pub pusher: Option<Pusher>,
    /// Device ids whose app is in the foreground, by connection.
    foreground: Mutex<HashMap<u64, String>>,
    /// Serialises read-modify-write of devices.json within this process.
    files: Mutex<()>,
    next_conn: AtomicU64,
    forwards: Arc<Forwards>,
}

impl Ctx {
    pub fn new(store: Store) -> Result<Ctx> {
        let config = store.config()?;
        let identity = store.identity()?;
        let pusher = Pusher::load(&store.push_key_path())?;
        Ok(Ctx {
            store,
            config,
            identity,
            pusher,
            foreground: Mutex::default(),
            files: Mutex::default(),
            next_conn: AtomicU64::new(1),
            forwards: Arc::default(),
        })
    }

    pub fn hc_command(&self) -> Command {
        let mut c = Command::new(self.config.hc_path());
        c.arg("connect");
        if let Some(s) = &self.config.session {
            c.arg("--session").arg(s);
        }
        // Started from a Herdr terminal, these would make every call look like it came from that terminal.
        for v in ["HC_TERMINAL_ID", "HC_TERMINAL_SOCKET", "HC_SCOPE"] {
            c.env_remove(v);
        }
        c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).kill_on_drop(true);
        c
    }

    fn is_foreground(&self, device: &str) -> bool {
        self.foreground.lock().unwrap().values().any(|d| d == device)
    }

    /// Send an alert to every device that registered for push and isn't looking. Blocking.
    pub fn push(&self, alert: &Alert) -> Result<String> {
        let devices = self.store.devices()?;
        let registered: Vec<_> = devices.devices.iter().filter(|d| d.push_token.is_some()).collect();
        let tokens: Vec<String> = registered.iter().filter(|d| !self.is_foreground(&d.id)).filter_map(|d| d.push_token.clone()).collect();
        if tokens.is_empty() && !registered.is_empty() {
            bail!("the app is open on every registered phone (no push while it's in front)");
        }
        self.push_to(&tokens, alert)
    }

    pub fn push_to(&self, tokens: &[String], alert: &Alert) -> Result<String> {
        let Some(p) = &self.pusher else { bail!("no push key: put the Push Kit service account key at {}", self.store.push_key_path().display()) };
        if !self.config.push.enabled {
            bail!("push is turned off in config.json");
        }
        let a = push::Alert {
            foreground: alert.terminal == "test",
            title: &alert.title,
            body: &alert.body,
            notify_id: push::notify_id(&alert.terminal),
            data: json!({"terminal": alert.terminal}),
        };
        p.send(&self.config.push, tokens, &a)
    }
}

/// Loopback, private, link-local and Tailscale (100.64/10, fd7a:115c:a1e0::/48 ⊂ fc00::/7) addresses.
pub fn is_near(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_loopback() || v.is_private() || v.is_link_local() || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v) => {
            if let Some(m) = v.to_ipv4_mapped() {
                return is_near(IpAddr::V4(m));
            }
            let s = v.segments();
            v.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

pub async fn serve(ctx: Arc<Ctx>) -> Result<()> {
    let listener = TcpListener::bind(&ctx.config.listen).await.with_context(|| format!("listen on {}", ctx.config.listen))?;
    log(format!(
        "listening on {} (hc: {}{}); push: {}",
        listener.local_addr()?,
        ctx.config.hc_path().display(),
        ctx.config.session.as_ref().map(|s| format!(" --session {s}")).unwrap_or_default(),
        if ctx.pusher.is_some() { "ready" } else { "no key" }
    ));
    tokio::spawn(watch(ctx.clone()));
    loop {
        let (stream, peer) = listener.accept().await?;
        if !ctx.config.allow_public && !is_near(peer.ip()) {
            log(format!("refused {peer}: not a private or Tailscale address"));
            continue;
        }
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = connection(ctx, stream, peer).await {
                log(format!("{peer}: {e:#}"));
            }
        });
    }
}

async fn connection(ctx: Arc<Ctx>, stream: TcpStream, peer: SocketAddr) -> Result<()> {
    stream.set_nodelay(true)?;
    // tungstenite's callback signature fixes the error type.
    #[allow(clippy::result_large_err)]
    let check_path = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if req.uri().path() == "/v1" {
            Ok(resp)
        } else {
            let mut r = ErrorResponse::new(Some("not found".into()));
            *r.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND;
            Err(r)
        }
    };
    let ws = tokio::time::timeout(AUTH_TIMEOUT, tokio_tungstenite::accept_hdr_async(stream, check_path))
        .await
        .map_err(|_| anyhow!("websocket upgrade timed out"))??;
    let (mut sink, mut source) = ws.split();

    let msg1 = tokio::time::timeout(AUTH_TIMEOUT, next_binary(&mut source)).await.map_err(|_| anyhow!("handshake timed out"))??;
    let (msg2, keys) = crypto::server_handshake(&ctx.identity, &msg1)?;
    sink.send(Message::Binary(msg2)).await?;
    let (mut send, mut recv) = (keys.send, keys.recv);

    let mut records = Records::default();
    let first = tokio::time::timeout(AUTH_TIMEOUT, next_record(&mut source, &mut records)).await.map_err(|_| anyhow!("auth timed out"))??;
    let first = recv.open_record(&first)?;
    let hello = control(&first).ok_or_else(|| anyhow!("first message was not a control message"))?;
    let device = match hello["t"].as_str() {
        Some("pair") => {
            let code = hello["code"].as_str().unwrap_or_default();
            let name = hello["device"].as_str().unwrap_or("phone");
            let result = {
                let _g = ctx.files.lock().unwrap();
                ctx.store.pair(code, name)
            };
            match result {
                Ok((d, token)) => {
                    log(format!("{peer}: paired new device {} ({})", d.name, d.id));
                    sink.send(seal_control(&mut send, &json!({"t": "paired", "device_id": d.id, "token": token, "name": ctx.config.display_name()}))).await?;
                    d
                }
                Err(e) => {
                    let code = match e {
                        PairError::Expired => "pair_expired",
                        PairError::Invalid => "pair_invalid",
                        PairError::Io(e) => {
                            log(format!("{peer}: pairing failed: {e:#}"));
                            "internal"
                        }
                    };
                    sink.send(seal_control(&mut send, &json!({"t": "error", "code": code}))).await?;
                    bail!("pairing refused: {code}");
                }
            }
        }
        Some("auth") => {
            let found = {
                let _g = ctx.files.lock().unwrap();
                ctx.store.auth(hello["device_id"].as_str().unwrap_or_default(), hello["token"].as_str().unwrap_or_default())?
            };
            let Some(d) = found else {
                sink.send(seal_control(&mut send, &json!({"t": "error", "code": "unauthorized"}))).await?;
                bail!("unknown or revoked device");
            };
            sink.send(seal_control(&mut send, &json!({"t": "ok", "bridge": env!("CARGO_PKG_VERSION"), "name": ctx.config.display_name()}))).await?;
            d
        }
        _ => bail!("expected pair or auth"),
    };

    let conn = ctx.next_conn.fetch_add(1, Ordering::Relaxed);
    log(format!("{peer}: {} connected (#{conn})", device.name));
    let result = relay(&ctx, conn, &device.id, sink, source, send, recv, records).await;
    ctx.foreground.lock().unwrap().remove(&conn);
    log(format!("{peer}: {} disconnected (#{conn})", device.name));
    result
}

type Sink = futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;
type Source = futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>;

async fn next_binary(source: &mut Source) -> Result<Vec<u8>> {
    while let Some(m) = source.next().await {
        match m? {
            Message::Binary(b) => return Ok(b),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => bail!("closed"),
            _ => bail!("unexpected text frame"),
        }
    }
    bail!("closed")
}

async fn next_record(source: &mut Source, records: &mut Records) -> Result<Vec<u8>> {
    loop {
        if let Some(r) = records.pop()? {
            return Ok(r);
        }
        records.push(&next_binary(source).await?);
    }
}

fn control(plain: &[u8]) -> Option<Value> {
    (plain.first() == Some(&CONTROL)).then(|| serde_json::from_slice(&plain[1..]).ok()).flatten()
}

fn seal_control(send: &mut Half, v: &Value) -> Message {
    let mut p = vec![CONTROL];
    p.extend(serde_json::to_vec(v).unwrap());
    Message::Binary(send.seal_record(&p))
}

#[allow(clippy::too_many_arguments)]
async fn relay(
    ctx: &Arc<Ctx>,
    conn: u64,
    device: &str,
    mut sink: Sink,
    mut source: Source,
    mut send: Half,
    mut recv: Half,
    mut records: Records,
) -> Result<()> {
    let mut child: Child = ctx.hc_command().spawn().with_context(|| format!("run {} connect", ctx.config.hc_path().display()))?;
    let mut hc_in = child.stdin.take().unwrap();
    let mut hc_out = child.stdout.take().unwrap();

    // Everything to the phone goes through one writer so the nonce counter matches the order.
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            tokio::select! {
                m = out_rx.recv() => match m {
                    Some(plain) => if sink.send(Message::Binary(send.seal_record(&plain))).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if sink.send(Message::Ping(vec![])).await.is_err() { break },
            }
        }
        let _ = sink.close().await;
    });

    let tx = out_tx.clone();
    let reader = tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match hc_out.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut p = Vec::with_capacity(n + 1);
                    p.push(HC);
                    p.extend_from_slice(&buf[..n]);
                    if tx.send(p).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let result: Result<()> = async {
        loop {
            tokio::select! {
                m = source.next() => {
                    let Some(m) = m else { return Ok(()) };
                    let bytes = match m? {
                        Message::Binary(b) => b,
                        Message::Ping(_) | Message::Pong(_) => continue,
                        Message::Close(_) => return Ok(()),
                        _ => bail!("unexpected text frame"),
                    };
                    records.push(&bytes);
                    while let Some(rec) = records.pop()? {
                        let plain = recv.open_record(&rec)?;
                        match plain.first() {
                            Some(&HC) => hc_in.write_all(&plain[1..]).await.context("hc connect stopped reading")?,
                            Some(&CONTROL) => {
                                let v: Value = serde_json::from_slice(&plain[1..])?;
                                // Requests that read files or the network answer later (matched by id), so the terminal keeps flowing.
                                let (ctx, device, tx) = (ctx.clone(), device.to_owned(), out_tx.clone());
                                let task = async move {
                                    if let Some(reply) = on_control(&ctx, conn, &device, &v).await {
                                        let mut p = vec![CONTROL];
                                        p.extend(serde_json::to_vec(&reply).unwrap_or_default());
                                        let _ = tx.send(p).await;
                                    }
                                };
                                if matches!(serde_json::from_slice::<Value>(&plain[1..])?["t"].as_str(), Some("transcript" | "ports" | "forward")) {
                                    tokio::spawn(task);
                                } else {
                                    task.await;
                                }
                            }
                            _ => bail!("unknown message type"),
                        }
                    }
                }
                status = child.wait() => {
                    let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
                    let mut p = vec![CONTROL];
                    p.extend(serde_json::to_vec(&json!({"t": "hc_closed", "code": code}))?);
                    let _ = out_tx.send(p).await;
                    // Let the reader drain what hc wrote last, then stop.
                    let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
                    return Ok(());
                }
            }
        }
    }
    .await;
    drop(out_tx);
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
    result
}

async fn on_control(ctx: &Arc<Ctx>, conn: u64, device: &str, v: &Value) -> Option<Value> {
    match v["t"].as_str()? {
        "foreground" => {
            let mut fg = ctx.foreground.lock().unwrap();
            if v["on"].as_bool().unwrap_or(false) {
                fg.insert(conn, device.to_owned());
            } else {
                fg.remove(&conn);
            }
            None
        }
        "push_token" => {
            let token = v["token"].as_str().map(str::to_owned);
            let r = {
                let _g = ctx.files.lock().unwrap();
                ctx.store.set_push_token(device, token)
            };
            Some(match r {
                Ok(()) => {
                    json!({"t": "push_result", "re": "push_token", "ok": true, "detail": if ctx.pusher.is_some() { "registered" } else { "registered; the bridge has no push key yet" }})
                }
                Err(e) => json!({"t": "push_result", "re": "push_token", "ok": false, "detail": e.to_string()}),
            })
        }
        "push_test" => {
            let ctx = ctx.clone();
            let device = device.to_owned();
            let r = tokio::task::spawn_blocking(move || {
                let token = ctx
                    .store
                    .devices()?
                    .devices
                    .into_iter()
                    .find(|d| d.id == device)
                    .and_then(|d| d.push_token)
                    .ok_or_else(|| anyhow!("this device has not registered a push token"))?;
                let alert =
                    Alert { terminal: "test".into(), why: crate::watch::Why::Done, title: "码上 · 测试通知".into(), body: "推送通道正常".into() };
                ctx.push_to(&[token], &alert)
            })
            .await
            .map_err(|e| anyhow!(e))
            .and_then(|r| r);
            Some(match r {
                Ok(m) => json!({"t": "push_result", "re": "push_test", "ok": true, "detail": m}),
                Err(e) => json!({"t": "push_result", "re": "push_test", "ok": false, "detail": format!("{e:#}")}),
            })
        }
        "transcript" => {
            let (app, cwd) = (v["app"].as_str().unwrap_or_default().to_owned(), v["cwd"].as_str().unwrap_or_default().to_owned());
            let (file, offset) = (v["file"].as_str().map(str::to_owned), v["offset"].as_u64());
            let pid = v["pid"].as_u64().and_then(|p| u32::try_from(p).ok()).filter(|p| *p > 0);
            let before = v["before"].as_u64();
            let r = tokio::task::spawn_blocking(move || crate::transcript::read(&app, &cwd, pid, file.as_deref(), offset, before)).await;
            Some(match r.map_err(|e| anyhow!(e)).and_then(|r| r) {
                Ok(page) => {
                    let mut j = serde_json::to_value(page).unwrap_or_default();
                    j["t"] = json!("transcript");
                    j["id"] = v["id"].clone();
                    j
                }
                Err(e) => json!({"t": "transcript", "id": v["id"], "error": format!("{e:#}")}),
            })
        }
        "ports" => {
            let mut skip = ctx.forwards.listen_ports();
            skip.extend(ctx.config.listen.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()));
            let sites = crate::ports::list(&skip).await;
            Some(json!({"t": "ports", "id": v["id"], "sites": sites}))
        }
        "forward" => {
            let port = v["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(0);
            let ip = ctx.config.listen.rsplit_once(':').map_or("0.0.0.0", |(h, _)| h).trim_matches(['[', ']']).to_owned();
            Some(match ctx.forwards.open(&ip, port, ctx.config.allow_public).await {
                Ok(f) => json!({"t": "forward", "id": v["id"], "port": port, "listen_port": f.listen_port, "token": f.token}),
                Err(e) => json!({"t": "forward", "id": v["id"], "port": port, "error": format!("{e:#}")}),
            })
        }
        _ => None,
    }
}

/// Follow the core's events and push alerts; reconnects when hc goes away.
async fn watch(ctx: Arc<Ctx>) {
    let mut wait = Duration::from_secs(1);
    loop {
        match watch_once(&ctx).await {
            Ok(()) => wait = Duration::from_secs(1),
            Err(e) => log(format!("watcher: {e:#}")),
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(30));
    }
}

async fn watch_once(ctx: &Arc<Ctx>) -> Result<()> {
    let mut child = ctx.hc_command().spawn().with_context(|| format!("run {} connect", ctx.config.hc_path().display()))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"events.subscribe\",\"params\":{}}\n").await?;
    let mut tracker = Tracker::default();
    let mut subscribed = false;
    while let Some(line) = lines.next_line().await? {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if !subscribed {
            if v["id"] == 1 {
                if let Some(err) = v.get("error") {
                    bail!("events.subscribe failed: {err}");
                }
                tracker.snapshot(v["result"]["seq"].as_u64().unwrap_or(0), &v["result"]["terminals"]);
                subscribed = true;
                log(format!("watcher: following {} terminal(s)", v["result"]["terminals"].as_array().map_or(0, Vec::len)));
            }
            continue;
        }
        for alert in tracker.line(&v) {
            let wanted = match alert.why {
                crate::watch::Why::NeedsYou => ctx.config.push.notify_blocked,
                crate::watch::Why::Done => ctx.config.push.notify_done,
            };
            if !wanted {
                continue;
            }
            log(format!("alert {}: {} — {}", alert.terminal, alert.title, alert.body));
            let ctx = ctx.clone();
            tokio::task::spawn_blocking(move || match ctx.push(&alert) {
                Ok(m) => log(format!("pushed {}: {m}", alert.terminal)),
                Err(e) => log(format!("push {} not sent: {e:#}", alert.terminal)),
            });
        }
    }
    let _ = child.wait().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_addresses() {
        for ip in ["127.0.0.1", "192.168.1.5", "10.0.0.2", "172.20.1.1", "100.101.2.3", "::1", "fd7a:115c:a1e0::1", "fe80::1", "::ffff:192.168.1.2"] {
            assert!(is_near(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "2001:4860::8888", "::ffff:8.8.8.8"] {
            assert!(!is_near(ip.parse().unwrap()), "{ip}");
        }
    }
}
