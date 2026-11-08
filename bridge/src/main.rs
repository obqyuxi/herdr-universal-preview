//! ddup-bridge: lets the DD-up HarmonyOS app reach herdr-core on this Mac (D27).
//! Design and wire protocol: docs/harmony/DESIGN.md.

mod crypto;
mod ports;
mod push;
mod server;
mod state;
mod transcript;
mod watch;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use state::Store;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser)]
#[command(
    name = "ddup-bridge",
    version,
    about = "Let the DD-up HarmonyOS app reach herdr-core on this Mac (encrypted relay of `hc connect`, QR pairing, Push Kit alerts)"
)]
struct Cli {
    /// State directory (default: $DDUP_BRIDGE_HOME or ~/Library/Application Support/DD-up/bridge)
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the bridge (WebSocket listener and push watcher)
    Serve {
        /// Override config.json `listen`, e.g. 127.0.0.1:7841
        #[arg(long)]
        listen: Option<String>,
        /// Override config.json `hc`
        #[arg(long)]
        hc: Option<PathBuf>,
        /// Override config.json `session`
        #[arg(long)]
        session: Option<String>,
    },
    /// Make a one-time pairing code (10 minutes) and print its QR code and link
    Pair {
        /// Address the phone should dial (repeatable); default: this Mac's private and Tailscale addresses
        #[arg(long = "host")]
        hosts: Vec<String>,
        /// Print only the link (no QR code)
        #[arg(long)]
        link_only: bool,
        /// Print JSON (link, QR modules, addresses, expiry, whether `serve` is running) for the DD-up dialog
        #[arg(long)]
        json: bool,
    },
    /// List paired devices
    Devices {
        #[arg(long)]
        json: bool,
    },
    /// Remove a paired device by id or name
    Revoke { device: String },
    /// Send a test push to every device that registered for push
    PushTest {
        /// Send to this Push Token instead (a made-up one checks the key: Push Kit then rejects only the token)
        #[arg(long)]
        token: Option<String>,
    },
    /// Show the state directory, config and push readiness
    Status,
    /// Print a fresh Push Kit JWT (1 hour) for diagnosing the service account; pipe it, don't log it
    #[command(hide = true)]
    PushJwt,
    /// Act as a phone: pair with a link (or reuse saved credentials), then call the core. For testing.
    Probe {
        /// A ddup://pair link (needed the first time)
        #[arg(long)]
        link: Option<String>,
        /// Where the probe keeps its credentials
        #[arg(long)]
        creds: PathBuf,
        /// JSON-RPC method to call after hello
        #[arg(long, default_value = "terminal.list")]
        method: String,
        /// Register this push token for the probe device
        #[arg(long)]
        push_token: Option<String>,
        /// Send this control message (JSON with an "id") instead of calling the core; print the reply
        #[arg(long)]
        control: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open(cli.state_dir.unwrap_or_else(state::default_dir))?;
    match cli.cmd {
        Cmd::Serve { listen, hc, session } => {
            let mut ctx = server::Ctx::new(store)?;
            if let Some(l) = listen {
                ctx.config.listen = l;
            }
            if hc.is_some() {
                ctx.config.hc = hc;
            }
            if session.is_some() {
                ctx.config.session = session;
            }
            tokio::runtime::Runtime::new()?.block_on(server::serve(Arc::new(ctx)))
        }
        Cmd::Pair { hosts, link_only, json } => pair(&store, hosts, link_only, json),
        Cmd::Devices { json } => {
            let d = store.devices()?;
            if json {
                let list: Vec<Value> = d
                    .devices
                    .iter()
                    .map(|d| json!({"id": d.id, "name": d.name, "created_ms": d.created_ms, "last_seen_ms": d.last_seen_ms, "push": d.push_token.is_some()}))
                    .collect();
                println!("{}", json!({"devices": list}));
                return Ok(());
            }
            if d.devices.is_empty() {
                println!("no paired devices");
            }
            for d in d.devices {
                println!(
                    "{}  {}  paired {}  last seen {}  push {}",
                    d.id,
                    d.name,
                    ago(d.created_ms),
                    d.last_seen_ms.map(ago).unwrap_or("never".into()),
                    if d.push_token.is_some() { "on" } else { "off" }
                );
            }
            Ok(())
        }
        Cmd::Revoke { device } => {
            let n = store.revoke(&device)?;
            if n == 0 {
                bail!("no device with id or name {device:?}");
            }
            println!("revoked {n} device(s); open connections end when they next reconnect");
            Ok(())
        }
        Cmd::PushTest { token } => {
            let ctx = server::Ctx::new(store)?;
            let alert = watch::Alert { terminal: "test".into(), why: watch::Why::Done, title: "DD-up 测试通知".into(), body: "推送通道正常".into() };
            let sent = match token {
                Some(t) => ctx.push_to(&[t], &alert)?,
                None => ctx.push(&alert)?,
            };
            println!("{sent}");
            Ok(())
        }
        Cmd::PushJwt => {
            let p = push::Pusher::load(&store.push_key_path())?.ok_or_else(|| anyhow!("no push-key.json"))?;
            println!("{}", p.jwt_at(state::now_ms() / 1000));
            Ok(())
        }
        Cmd::Status => {
            let c = store.config()?;
            let id = store.identity()?;
            println!("state dir   {}", store.dir.display());
            println!("listen      {}{}", c.listen, if c.allow_public { " (public allowed)" } else { " (private and Tailscale peers only)" });
            println!("hc          {}{}", c.hc_path().display(), c.session.as_ref().map(|s| format!(" --session {s}")).unwrap_or_default());
            println!("bridge key  {}", B64.encode(crypto::public_of(&id)));
            println!("devices     {}", store.devices()?.devices.len());
            let key = store.push_key_path();
            println!(
                "push        {}; category {}, testMessage {}",
                if key.exists() { "key present" } else { "no key (put the service account key file at push-key.json)" },
                c.push.category,
                c.push.test_message
            );
            Ok(())
        }
        Cmd::Probe { link, creds, method, push_token, control } => tokio::runtime::Runtime::new()?.block_on(probe(link, creds, method, push_token, control)),
    }
}

fn ago(ms: u64) -> String {
    let s = state::now_ms().saturating_sub(ms) / 1000;
    match s {
        0..60 => format!("{s}s ago"),
        60..3600 => format!("{}m ago", s / 60),
        3600..86400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

/// Private LAN addresses first, then Tailscale.
fn near_hosts() -> Vec<String> {
    let mut lan = vec![];
    let mut tailnet = vec![];
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        if let IpAddr::V4(v) = i.ip() {
            let o = v.octets();
            if o[0] == 100 && (o[1] & 0xc0) == 64 {
                tailnet.push(v.to_string());
            } else if v.is_private() {
                lan.push(v.to_string());
            }
        }
    }
    lan.extend(tailnet);
    lan
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Whether `serve` answers on this Mac (the port in config.json).
fn serving(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), std::time::Duration::from_millis(400)).is_ok()
}

fn pair(store: &Store, hosts: Vec<String>, link_only: bool, json_out: bool) -> Result<()> {
    let c = store.config()?;
    let id = store.identity()?;
    let hosts = if hosts.is_empty() { near_hosts() } else { hosts };
    if hosts.is_empty() {
        bail!("no private or Tailscale address found; pass --host");
    }
    let code = store.new_pairing()?;
    let mut link = format!("ddup://pair?v=1&p={}&k={}&c={}&n={}", c.port(), B64.encode(crypto::public_of(&id)), code, encode(&c.display_name()));
    for h in &hosts {
        link.push_str(&format!("&h={}", encode(h)));
    }
    if json_out {
        let qr = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M).context("make QR code")?;
        let w = qr.width();
        let modules: Vec<String> =
            qr.to_colors().chunks(w).map(|row| row.iter().map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' }).collect()).collect();
        let out = json!({
            "link": link,
            "qr": modules,
            "hosts": hosts,
            "name": c.display_name(),
            "port": c.port(),
            "expires_ms": state::now_ms() + state::PAIR_TTL_MS,
            "serving": serving(c.port()),
            "devices": store.devices()?.devices.len(),
        });
        println!("{out}");
        return Ok(());
    }
    if !link_only {
        let qr = qrcode::QrCode::new(link.as_bytes()).context("make QR code")?;
        let art = qr
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .quiet_zone(true)
            .build();
        println!("{art}");
        println!("Scan with DD-up on the phone (valid 10 minutes, once). Addresses: {}", hosts.join(", "));
    }
    println!("{link}");
    Ok(())
}

struct Link {
    hosts: Vec<String>,
    port: u16,
    key: [u8; 32],
    code: Option<String>,
}

fn parse_link(s: &str) -> Result<Link> {
    let q = s.strip_prefix("ddup://pair?").ok_or_else(|| anyhow!("not a ddup://pair link"))?;
    let (mut hosts, mut port, mut key, mut code) = (vec![], state::DEFAULT_PORT, None, None);
    for kv in q.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        let v = decode(v);
        match k {
            "h" => hosts.push(v),
            "p" => port = v.parse()?,
            "k" => key = Some(B64.decode(v)?.try_into().map_err(|_| anyhow!("bad key"))?),
            "c" => code = Some(v),
            _ => {}
        }
    }
    Ok(Link { hosts, port, key: key.ok_or_else(|| anyhow!("link has no key"))?, code })
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(x) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(x);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn frame(channel: u32, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = channel.to_le_bytes().to_vec();
    f.push(kind);
    f.extend((payload.len() as u32).to_le_bytes());
    f.extend_from_slice(payload);
    f
}

async fn probe(link: Option<String>, creds_path: PathBuf, method: String, push_token: Option<String>, control: Option<String>) -> Result<()> {
    let saved: Option<Value> = std::fs::read(&creds_path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let (hosts, port, key, code) = match (&link, &saved) {
        (Some(l), _) => {
            let l = parse_link(l)?;
            (l.hosts, l.port, l.key, l.code)
        }
        (None, Some(s)) => {
            let key: [u8; 32] = B64.decode(s["key"].as_str().unwrap_or_default())?.try_into().map_err(|_| anyhow!("bad saved key"))?;
            (vec![s["host"].as_str().unwrap_or_default().to_owned()], s["port"].as_u64().unwrap_or(7841) as u16, key, None)
        }
        _ => bail!("need --link the first time"),
    };
    let mut last = anyhow!("no address to dial");
    for host in &hosts {
        let url = format!("ws://{}:{port}/v1", if host.contains(':') { format!("[{host}]") } else { host.clone() });
        match tokio::time::timeout(std::time::Duration::from_secs(3), tokio_tungstenite::connect_async(&url)).await {
            Ok(Ok((ws, _))) => return probe_on(ws, host, port, key, code, saved, &creds_path, &method, push_token, control.clone()).await,
            Ok(Err(e)) => last = anyhow!("{url}: {e}"),
            Err(_) => last = anyhow!("{url}: timed out"),
        }
    }
    Err(last)
}

#[allow(clippy::too_many_arguments)]
async fn probe_on(
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    host: &str,
    port: u16,
    key: [u8; 32],
    code: Option<String>,
    saved: Option<Value>,
    creds_path: &PathBuf,
    method: &str,
    push_token: Option<String>,
    control: Option<String>,
) -> Result<()> {
    let (mut sink, mut source) = ws.split();
    let (ce, msg1) = crypto::client_start();
    sink.send(Message::Binary(msg1)).await?;
    let msg2 = next_bin(&mut source).await?;
    let mut keys = crypto::client_finish(&ce, &key, &msg2)?;
    let send_ctl = |keys: &mut crypto::Keys, v: Value| {
        let mut p = vec![server::CONTROL];
        p.extend(serde_json::to_vec(&v).unwrap());
        Message::Binary(keys.send.seal_record(&p))
    };
    let hello = match (&code, &saved) {
        (Some(c), _) => json!({"t": "pair", "code": c, "device": "probe"}),
        (None, Some(s)) => json!({"t": "auth", "device_id": s["device_id"], "token": s["token"]}),
        _ => bail!("no pairing code and no saved credentials"),
    };
    sink.send(send_ctl(&mut keys, hello)).await?;
    let mut records = crypto::Records::default();
    let reply = keys.recv.open_record(&next_rec(&mut source, &mut records).await?)?;
    let reply: Value = serde_json::from_slice(&reply[1..])?;
    println!("bridge: {reply}");
    match reply["t"].as_str() {
        Some("paired") => {
            let c = json!({"device_id": reply["device_id"], "token": reply["token"], "key": B64.encode(key), "host": host, "port": port});
            std::fs::write(creds_path, serde_json::to_vec_pretty(&c)?)?;
        }
        Some("ok") => {}
        _ => bail!("bridge refused: {reply}"),
    }
    if let Some(t) = push_token {
        sink.send(send_ctl(&mut keys, json!({"t": "push_token", "token": t}))).await?;
    }
    if let Some(c) = control {
        let req: Value = serde_json::from_str(&c).context("--control is not JSON")?;
        sink.send(send_ctl(&mut keys, req.clone())).await?;
        loop {
            let plain = keys.recv.open_record(&next_rec(&mut source, &mut records).await?)?;
            if plain[0] != server::CONTROL {
                continue;
            }
            let v: Value = serde_json::from_slice(&plain[1..])?;
            if v["t"] == req["t"] && v["id"] == req["id"] {
                println!("{}", serde_json::to_string_pretty(&v)?);
                let _ = sink.close().await;
                return if v.get("error").is_some() { Err(anyhow!("bridge error")) } else { Ok(()) };
            }
        }
    }

    let send_hc = |keys: &mut crypto::Keys, bytes: &[u8]| {
        let mut p = vec![server::HC];
        p.extend_from_slice(bytes);
        Message::Binary(keys.send.seal_record(&p))
    };
    sink.send(send_hc(&mut keys, b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hello\",\"params\":{\"framed\":true}}\n")).await?;
    let mut buf: Vec<u8> = vec![];
    let mut hello_done = false;
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}}).to_string() + "\n";
    loop {
        let plain = keys.recv.open_record(&next_rec(&mut source, &mut records).await?)?;
        if plain[0] == server::CONTROL {
            println!("bridge: {}", String::from_utf8_lossy(&plain[1..]));
            continue;
        }
        buf.extend_from_slice(&plain[1..]);
        if !hello_done {
            let Some(nl) = buf.iter().position(|b| *b == b'\n') else { continue };
            let line: Value = serde_json::from_slice(&buf[..nl])?;
            buf.drain(..=nl);
            println!("core hello: version {} generation {}", line["result"]["version"], line["result"]["generation"]);
            hello_done = true;
            sink.send(send_hc(&mut keys, &frame(0, 0, call.as_bytes()))).await?;
        }
        while buf.len() >= 9 {
            let ch = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            let kind = buf[4];
            let len = u32::from_le_bytes(buf[5..9].try_into().unwrap()) as usize;
            if buf.len() < 9 + len {
                break;
            }
            let payload: Vec<u8> = buf[9..9 + len].to_vec();
            buf.drain(..9 + len);
            if ch == 0 && kind == 0 {
                for line in payload.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                    let v: Value = serde_json::from_slice(line)?;
                    if v["id"] == 1 {
                        println!("{}", serde_json::to_string_pretty(&v)?);
                        let _ = sink.close().await;
                        return if v.get("error").is_some() { Err(anyhow!("core error")) } else { Ok(()) };
                    }
                }
            }
        }
    }
}

async fn next_rec<S>(source: &mut S, records: &mut crypto::Records) -> Result<Vec<u8>>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        if let Some(r) = records.pop()? {
            return Ok(r);
        }
        records.push(&next_bin(source).await?);
    }
}

async fn next_bin<S>(source: &mut S) -> Result<Vec<u8>>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(m) =
        tokio::time::timeout(std::time::Duration::from_secs(10), source.next()).await.map_err(|_| anyhow!("timed out waiting for the bridge"))?
    {
        match m? {
            Message::Binary(b) => return Ok(b),
            Message::Close(_) => bail!("bridge closed the connection"),
            _ => continue,
        }
    }
    bail!("bridge closed the connection")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip() {
        let l = parse_link("ddup://pair?v=1&p=7841&k=AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE&c=abc&n=My%20Mac&h=192.168.1.2&h=100.64.0.9").unwrap();
        assert_eq!(l.hosts, vec!["192.168.1.2", "100.64.0.9"]);
        assert_eq!(l.port, 7841);
        assert_eq!(l.key, [1u8; 32]);
        assert_eq!(l.code.as_deref(), Some("abc"));
        assert_eq!(decode(&encode("小米 Mac")), "小米 Mac");
    }
}
