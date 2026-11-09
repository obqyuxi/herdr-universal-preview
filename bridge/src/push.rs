//! Huawei Push Kit V3 (HarmonyOS NEXT and later): a service-account JWT
//! (PS256) and `messages:send`. docs/harmony/DESIGN.md §6.

use crate::state::PushConfig;
use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand_core::OsRng;
use rsa::RsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pss::SigningKey;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const AUD: &str = "https://oauth-login.cloud.huawei.com/oauth2/v3/token";
const OK: &str = "80000000";

/// The service account key file from the Huawei developer API Console.
#[derive(Deserialize)]
pub struct ServiceKey {
    pub project_id: String,
    pub key_id: String,
    pub private_key: String,
    pub sub_account: String,
}

pub struct Pusher {
    key: ServiceKey,
    signer: SigningKey<Sha256>,
    /// A JWT and when it stops being reused (seconds).
    cached: Mutex<Option<(String, u64)>>,
    endpoint: String,
}

pub struct Alert<'a> {
    /// Show it even while the app is in front (test pushes, asked for from the app).
    pub foreground: bool,
    pub title: &'a str,
    pub body: &'a str,
    pub notify_id: i64,
    pub data: Value,
}

impl Pusher {
    pub fn load(path: &Path) -> Result<Option<Pusher>> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let key: ServiceKey = serde_json::from_slice(&bytes).with_context(|| format!("parse {} (a Push Kit service account key file)", path.display()))?;
        Ok(Some(Pusher::new(key)?))
    }

    pub fn new(key: ServiceKey) -> Result<Pusher> {
        let rsa = RsaPrivateKey::from_pkcs8_pem(&normalize_pem(&key.private_key)).context("service account private_key is not a PKCS#8 RSA key")?;
        let endpoint = format!("https://push-api.cloud.huawei.com/v3/{}/messages:send", key.project_id);
        Ok(Pusher { key, signer: SigningKey::<Sha256>::new(rsa), cached: Mutex::new(None), endpoint })
    }

    /// The JWT as the docs describe: PS256, iss = sub_account, exp = iat + 3600.
    pub fn jwt_at(&self, iat: u64) -> String {
        // Field order as in Huawei's examples (kid first); serde_json would sort the keys.
        let q = |s: &str| Value::String(s.to_owned()).to_string();
        let header = format!(r#"{{"kid":{},"typ":"JWT","alg":"PS256"}}"#, q(&self.key.key_id));
        let payload = format!(r#"{{"aud":{},"iss":{},"exp":{},"iat":{}}}"#, q(AUD), q(&self.key.sub_account), iat + 3600, iat);
        let signing_input = format!("{}.{}", B64.encode(header), B64.encode(payload));
        let sig = self.signer.sign_with_rng(&mut OsRng, signing_input.as_bytes());
        format!("{signing_input}.{}", B64.encode(sig.to_bytes()))
    }

    fn jwt(&self) -> String {
        let now = crate::state::now_ms() / 1000;
        let mut cached = self.cached.lock().unwrap();
        if let Some((jwt, until)) = cached.as_ref()
            && now < *until
        {
            return jwt.clone();
        }
        let jwt = self.jwt_at(now);
        *cached = Some((jwt.clone(), now + 3000));
        jwt
    }

    pub fn body(cfg: &PushConfig, tokens: &[String], alert: &Alert) -> Value {
        json!({
            "payload": {"notification": {
                "category": cfg.category,
                "title": alert.title,
                "body": alert.body,
                "clickAction": {"actionType": 0, "data": alert.data},
                "notifyId": alert.notify_id,
                "foregroundShow": alert.foreground,
            }},
            "target": {"token": tokens},
            "pushOptions": {"testMessage": cfg.test_message, "ttl": cfg.ttl},
        })
    }

    /// Blocking; call it off the async runtime. Returns Push Kit's message on success.
    pub fn send(&self, cfg: &PushConfig, tokens: &[String], alert: &Alert) -> Result<String> {
        if tokens.is_empty() {
            bail!("no device has registered for push");
        }
        self.post("0", Self::body(cfg, tokens, alert))
    }

    /// A card-refresh message (push-type 1) for one home-screen card. `version` must grow.
    pub fn form_body(cfg: &PushConfig, token: &str, form_id: &str, version: u64, data: &Value) -> Value {
        let id = form_id.parse::<i64>().map_or_else(|_| json!(form_id), |n| json!(n));
        json!({
            "payload": {
                "moduleName": "entry",
                "abilityName": "EntryFormAbility",
                "formName": "widget",
                "formId": id,
                "version": version,
                "formData": data,
            },
            "target": {"token": [token]},
            "pushOptions": {"testMessage": cfg.test_message},
        })
    }

    pub fn send_form(&self, cfg: &PushConfig, token: &str, form_id: &str, version: u64, data: &Value) -> Result<String> {
        self.post("1", Self::form_body(cfg, token, form_id, version, data))
    }

    fn post(&self, push_type: &str, body: Value) -> Result<String> {
        let resp = ureq::post(&self.endpoint)
            .timeout(Duration::from_secs(15))
            .set("Authorization", &format!("Bearer {}", self.jwt()))
            .set("push-type", push_type)
            .send_json(body);
        let (status, text) = match resp {
            Ok(r) => (r.status(), r.into_string().unwrap_or_default()),
            Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap_or_default()),
            Err(e) => return Err(anyhow!("push request failed: {e}")),
        };
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status == 200 && v["code"] == OK {
            Ok(v["msg"].as_str().unwrap_or("Success").to_owned())
        } else {
            bail!("Push Kit answered HTTP {status}: {}", if text.is_empty() { "(empty)".into() } else { text })
        }
    }
}

/// Huawei's key files put the whole base64 body on one line; strict PEM wants lines of 64.
fn normalize_pem(pem: &str) -> String {
    let body: String = pem
        .replace("-----BEGIN PRIVATE KEY-----", "")
        .replace("-----END PRIVATE KEY-----", "")
        .replace("\\n", "")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let lines: Vec<&str> = body.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap_or_default()).collect();
    format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", lines.join("\n"))
}

/// Card messages need a growing `version` that fits in an i32: tenths of a second since 2025.
pub fn form_version(now_ms: u64) -> u64 {
    now_ms.saturating_sub(1_735_689_600_000) / 100
}

/// A stable notify id per terminal, so a newer alert replaces the older one.
pub fn notify_id(address: &str) -> i64 {
    let h = crate::crypto::sha256_hex(address.as_bytes());
    (u32::from_str_radix(&h[..8], 16).unwrap_or(0) & 0x7fff_ffff) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::pss::{Signature, VerifyingKey};
    use rsa::signature::Verifier;

    #[test]
    fn jwt_is_ps256_and_verifies() {
        let rsa = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let pem = rsa.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let p = Pusher::new(ServiceKey { project_id: "p1".into(), key_id: "k1".into(), private_key: pem, sub_account: "s1".into() }).unwrap();
        let jwt = p.jwt_at(1_000);
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: Value = serde_json::from_slice(&B64.decode(parts[0]).unwrap()).unwrap();
        let payload: Value = serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(header, json!({"kid": "k1", "typ": "JWT", "alg": "PS256"}));
        assert_eq!(payload, json!({"aud": AUD, "iss": "s1", "iat": 1000, "exp": 4600}));
        let sig = Signature::try_from(B64.decode(parts[2]).unwrap().as_slice()).unwrap();
        VerifyingKey::<Sha256>::new(rsa.to_public_key()).verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).unwrap();
        assert_eq!(p.endpoint, "https://push-api.cloud.huawei.com/v3/p1/messages:send");
    }

    /// PS256 as another implementation reads it (salt length = 32, MGF1-SHA256).
    #[test]
    fn jwt_verifies_with_openssl() {
        use rsa::pkcs8::EncodePublicKey;
        let rsa = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let pem = rsa.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let p = Pusher::new(ServiceKey { project_id: "p".into(), key_id: "k".into(), private_key: pem, sub_account: "s".into() }).unwrap();
        let jwt = p.jwt_at(1_000);
        let (input, sig) = jwt.rsplit_once('.').unwrap();
        let dir = std::env::temp_dir().join(format!("ddup-jwt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pub.pem"), rsa.to_public_key().to_public_key_pem(LineEnding::LF).unwrap()).unwrap();
        std::fs::write(dir.join("in"), input).unwrap();
        std::fs::write(dir.join("sig"), B64.decode(sig).unwrap()).unwrap();
        let out = std::process::Command::new("openssl")
            .current_dir(&dir)
            .args([
                "dgst",
                "-sha256",
                "-sigopt",
                "rsa_padding_mode:pss",
                "-sigopt",
                "rsa_pss_saltlen:32",
                "-sigopt",
                "rsa_mgf1_md:sha256",
                "-verify",
                "pub.pem",
                "-signature",
                "sig",
                "in",
            ])
            .output();
        let _ = std::fs::remove_dir_all(&dir);
        if let Ok(o) = out {
            assert!(o.status.success(), "openssl: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        }
    }

    #[test]
    fn one_line_pem_bodies_load() {
        let rsa = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let pem = rsa.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let one_line = format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n");
        assert!(Pusher::new(ServiceKey { project_id: "p".into(), key_id: "k".into(), private_key: one_line, sub_account: "s".into() }).is_ok());
    }

    #[test]
    fn body_matches_the_v3_shape() {
        let cfg = PushConfig::default();
        let b = Pusher::body(&cfg, &["tok".into()], &Alert { foreground: false, title: "t", body: "b", notify_id: 7, data: json!({"terminal": "t3"}) });
        assert_eq!(b["payload"]["notification"]["category"], "MARKETING");
        assert_eq!(b["payload"]["notification"]["clickAction"]["actionType"], 0);
        assert_eq!(b["target"]["token"][0], "tok");
        assert_eq!(b["pushOptions"]["testMessage"], true);
    }

    #[test]
    fn card_body_matches_the_form_refresh_shape() {
        let b = Pusher::form_body(&PushConfig::default(), "tok", "4234342620", 17, &json!({"n": "1"}));
        assert_eq!(b["payload"]["formId"], 4234342620i64);
        assert_eq!(b["payload"]["formName"], "widget");
        assert_eq!(b["payload"]["abilityName"], "EntryFormAbility");
        assert_eq!(b["payload"]["formData"]["n"], "1");
        assert_eq!(b["target"]["token"][0], "tok");
        let v = form_version(1_794_200_000_000);
        assert!(v > 0 && v < i32::MAX as u64 && form_version(1_794_200_000_100) == v + 1);
    }

    #[test]
    fn notify_ids_are_stable_and_in_range() {
        assert_eq!(notify_id("t7"), notify_id("t7"));
        assert_ne!(notify_id("t7"), notify_id("t8"));
        assert!((0..=i32::MAX as i64).contains(&notify_id("mac/t12")));
    }
}
