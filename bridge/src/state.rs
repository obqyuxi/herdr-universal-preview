//! Files in the bridge's state directory (docs/harmony/DESIGN.md §1). Every
//! file is written whole through a temporary file and kept 0600.

use crate::crypto::{public_of, sha256_hex};
use anyhow::{Context as _, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use x25519_dalek::StaticSecret;

pub const PAIR_TTL_MS: u64 = 10 * 60 * 1000;
pub const PAIR_MAX_FAILURES: u32 = 5;
pub const DEFAULT_PORT: u16 = 7841;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    OsRng.fill_bytes(&mut b);
    B64.encode(b)
}

#[derive(Clone)]
pub struct Store {
    pub dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    secret: String,
    public: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Devices {
    pub devices: Vec<Device>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub token_sha256: String,
    pub created_ms: u64,
    #[serde(default)]
    pub last_seen_ms: Option<u64>,
    #[serde(default)]
    pub push_token: Option<String>,
    /// Home-screen cards the app has placed (Push Kit form ids), refreshed by card messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Pairing {
    code_sha256: String,
    expires_ms: u64,
    failures: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Address and port to listen on.
    pub listen: String,
    /// Accept connections from public addresses too (default: loopback, private, Tailscale only).
    pub allow_public: bool,
    /// `hc` to run; default `~/.local/bin/hc`.
    pub hc: Option<PathBuf>,
    /// `hc connect --session`; default: hc's own default.
    pub session: Option<String>,
    /// Name shown in the app; default: this Mac's host name.
    pub name: Option<String>,
    pub push: PushConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PushConfig {
    pub enabled: bool,
    /// Push Kit `category`: MARKETING until the self-classification right is granted (then e.g. WORK).
    pub category: String,
    /// Push Kit `testMessage`: 1000/day per project while developing.
    pub test_message: bool,
    pub ttl: u32,
    pub notify_blocked: bool,
    pub notify_done: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { listen: format!("0.0.0.0:{DEFAULT_PORT}"), allow_public: false, hc: None, session: None, name: None, push: PushConfig::default() }
    }
}

impl Default for PushConfig {
    fn default() -> Self {
        PushConfig { enabled: true, category: "MARKETING".into(), test_message: true, ttl: 86400, notify_blocked: true, notify_done: true }
    }
}

impl Config {
    pub fn port(&self) -> u16 {
        self.listen.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)
    }

    pub fn hc_path(&self) -> PathBuf {
        self.hc.clone().unwrap_or_else(|| home().join(".local/bin/hc"))
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            std::process::Command::new("scutil")
                .args(["--get", "ComputerName"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Mac".into())
        })
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn default_dir() -> PathBuf {
    std::env::var_os("DDUP_BRIDGE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join("Library/Application Support/DD-up/bridge"))
}

fn write_secure(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).with_context(|| format!("write {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

impl Store {
    pub fn open(dir: PathBuf) -> Result<Store> {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).with_context(|| format!("create {}", dir.display()))?;
        Ok(Store { dir })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn read<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        match fs::read(self.path(name)) {
            Ok(b) => Ok(Some(serde_json::from_slice(&b).with_context(|| format!("parse {}", self.path(name).display()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", self.path(name).display())),
        }
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        write_secure(&self.path(name), &serde_json::to_vec_pretty(value)?)
    }

    pub fn config(&self) -> Result<Config> {
        match self.read("config.json")? {
            Some(c) => Ok(c),
            None => {
                let c = Config::default();
                self.write("config.json", &c)?;
                Ok(c)
            }
        }
    }

    /// The bridge's static key, made on first use.
    pub fn identity(&self) -> Result<StaticSecret> {
        if let Some(f) = self.read::<IdentityFile>("identity.json")? {
            let bytes: [u8; 32] = B64.decode(f.secret)?.try_into().map_err(|_| anyhow::anyhow!("identity.json: bad key length"))?;
            return Ok(StaticSecret::from(bytes));
        }
        let secret = crate::crypto::random_secret();
        self.write("identity.json", &IdentityFile { secret: B64.encode(secret.to_bytes()), public: B64.encode(public_of(&secret)) })?;
        Ok(secret)
    }

    pub fn devices(&self) -> Result<Devices> {
        Ok(self.read("devices.json")?.unwrap_or_default())
    }

    pub fn save_devices(&self, d: &Devices) -> Result<()> {
        self.write("devices.json", d)
    }

    /// A fresh one-time pairing code (replacing any earlier one).
    pub fn new_pairing(&self) -> Result<String> {
        let code = random_b64(16);
        self.write("pairing.json", &Pairing { code_sha256: sha256_hex(code.as_bytes()), expires_ms: now_ms() + PAIR_TTL_MS, failures: 0 })?;
        Ok(code)
    }

    /// Spend the pairing code: a new device and its token, or why not.
    pub fn pair(&self, code: &str, name: &str) -> Result<(Device, String), PairError> {
        let Some(mut p) = self.read::<Pairing>("pairing.json").map_err(PairError::Io)? else { return Err(PairError::Invalid) };
        if now_ms() > p.expires_ms {
            let _ = fs::remove_file(self.path("pairing.json"));
            return Err(PairError::Expired);
        }
        if !bool::from(sha256_hex(code.as_bytes()).as_bytes().ct_eq(p.code_sha256.as_bytes())) {
            p.failures += 1;
            if p.failures >= PAIR_MAX_FAILURES {
                let _ = fs::remove_file(self.path("pairing.json"));
            } else {
                self.write("pairing.json", &p).map_err(PairError::Io)?;
            }
            return Err(PairError::Invalid);
        }
        fs::remove_file(self.path("pairing.json")).map_err(|e| PairError::Io(e.into()))?;
        let token = random_b64(32);
        let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
        let device = Device {
            id: random_b64(9),
            name: if name.is_empty() { "phone".into() } else { name },
            token_sha256: sha256_hex(token.as_bytes()),
            created_ms: now_ms(),
            last_seen_ms: Some(now_ms()),
            push_token: None,
            forms: Vec::new(),
        };
        let mut all = self.devices().map_err(PairError::Io)?;
        all.devices.push(device.clone());
        self.save_devices(&all).map_err(PairError::Io)?;
        Ok((device, token))
    }

    /// The device these credentials belong to, if they are still valid.
    pub fn auth(&self, id: &str, token: &str) -> Result<Option<Device>> {
        let mut all = self.devices()?;
        let want = sha256_hex(token.as_bytes());
        let Some(d) = all.devices.iter_mut().find(|d| d.id == id && bool::from(d.token_sha256.as_bytes().ct_eq(want.as_bytes()))) else { return Ok(None) };
        d.last_seen_ms = Some(now_ms());
        let found = d.clone();
        self.save_devices(&all)?;
        Ok(Some(found))
    }

    pub fn set_push_token(&self, id: &str, token: Option<String>) -> Result<()> {
        let mut all = self.devices()?;
        match all.devices.iter_mut().find(|d| d.id == id) {
            Some(d) => d.push_token = token.filter(|t| !t.is_empty()),
            None => bail!("device {id} was revoked"),
        }
        self.save_devices(&all)
    }

    pub fn set_forms(&self, id: &str, forms: Vec<String>) -> Result<()> {
        let mut all = self.devices()?;
        match all.devices.iter_mut().find(|d| d.id == id) {
            Some(d) => d.forms = forms.into_iter().filter(|f| !f.is_empty() && f.len() <= 32).take(8).collect(),
            None => bail!("device {id} was revoked"),
        }
        self.save_devices(&all)
    }

    /// Remove devices by id or name; returns how many.
    pub fn revoke(&self, who: &str) -> Result<usize> {
        let mut all = self.devices()?;
        let before = all.devices.len();
        all.devices.retain(|d| d.id != who && d.name != who);
        self.save_devices(&all)?;
        Ok(before - all.devices.len())
    }

    pub fn push_key_path(&self) -> PathBuf {
        self.path("push-key.json")
    }
}

#[derive(Debug)]
pub enum PairError {
    Invalid,
    Expired,
    Io(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> Store {
        let dir = std::env::temp_dir().join(format!("ddup-bridge-test-{}-{}", std::process::id(), random_b64(6)));
        Store::open(dir).unwrap()
    }

    #[test]
    fn a_code_pairs_once_and_its_token_authenticates() {
        let s = temp();
        let code = s.new_pairing().unwrap();
        let (d, token) = s.pair(&code, "Mate").unwrap();
        assert!(matches!(s.pair(&code, "again"), Err(PairError::Invalid)));
        assert_eq!(s.auth(&d.id, &token).unwrap().unwrap().name, "Mate");
        assert!(s.auth(&d.id, "wrong").unwrap().is_none());
        assert!(!fs::read_to_string(s.path("devices.json")).unwrap().contains(&token), "only the hash is stored");
        assert_eq!(s.revoke(&d.id).unwrap(), 1);
        assert!(s.auth(&d.id, &token).unwrap().is_none());
        let _ = fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn five_wrong_codes_spend_it() {
        let s = temp();
        let code = s.new_pairing().unwrap();
        for _ in 0..PAIR_MAX_FAILURES {
            assert!(matches!(s.pair("nope", "x"), Err(PairError::Invalid)));
        }
        assert!(matches!(s.pair(&code, "x"), Err(PairError::Invalid)));
        let _ = fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn files_are_private() {
        let s = temp();
        s.identity().unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(s.path("identity.json")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(s.identity().unwrap().to_bytes(), s.identity().unwrap().to_bytes(), "made once");
        let _ = fs::remove_dir_all(&s.dir);
    }
}
