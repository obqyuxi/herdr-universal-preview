//! Reads the cookies of the person's own Google Chrome (its last-used
//! profile) so the browser panel can stay signed in. Only on an explicit
//! request from the person (the "导入 Chrome 登录" button).
//!
//! What it touches, read-only: Chrome's `Local State` and the profile's
//! `Cookies` database (copied to a temporary file first, removed after), and
//! the "Chrome Safe Storage" Keychain item (macOS asks the person to allow
//! it). The key and cookie values stay in memory; nothing is logged or sent.
//! Passwords are not imported.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// One cookie as Chrome stored it, decrypted.
#[derive(Clone, Debug)]
pub struct ImportedCookie {
    pub host: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub secure: bool,
    pub httponly: bool,
    pub has_expires: bool,
    /// Microseconds since 1601-01-01 (Chrome's and CEF's base time).
    pub expires: i64,
    pub creation: i64,
    pub last_access: i64,
    /// Chrome's column: -1 unspecified, 0 none, 1 lax, 2 strict.
    pub samesite: i64,
}

pub struct ImportResult {
    pub profile: String,
    pub cookies: Vec<ImportedCookie>,
    pub skipped: usize,
}

fn chrome_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/Google/Chrome")
}

/// Microseconds between 1601-01-01 and now.
fn chrome_now() -> i64 {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0);
    unix + 11_644_473_600_000_000
}

pub fn read() -> Result<ImportResult, String> {
    let dir = chrome_dir();
    let state: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("Local State")).map_err(|_| "没有找到 Google Chrome 的资料（是否安装过 Chrome？）")?)
        .map_err(|_| "Chrome 的 Local State 无法读取")?;
    let profile = state.pointer("/profile/last_used").and_then(Value::as_str).unwrap_or("Default").to_owned();
    if profile.contains('/') || profile.contains("..") {
        return Err("Chrome 资料名称异常".into());
    }
    let db = dir.join(&profile).join("Cookies");
    if !db.is_file() {
        return Err(format!("Chrome 资料「{profile}」里没有 Cookie 数据库"));
    }
    // A private copy: Chrome may hold the database open; never write to it.
    let tmp = std::env::temp_dir().join(format!("hup-chrome-cookies-{}-{}", std::process::id(), chrome_now()));
    std::fs::create_dir(&tmp).map_err(|e| e.to_string())?;
    let copy = tmp.join("Cookies");
    let result = (|| {
        std::fs::copy(&db, &copy).map_err(|e| format!("复制 Cookie 数据库失败：{e}"))?;
        let version: i64 = sqlite(&copy, "select value from meta where key='version'")?
            .first().and_then(|r| r.get("value")).and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_i64())).unwrap_or(0);
        let rows = sqlite(&copy, "select host_key, name, value, hex(encrypted_value) as ev, path, expires_utc, is_secure, is_httponly, has_expires, samesite, creation_utc, last_access_utc from cookies")?;
        let key = derive_key(&keychain_password()?)?;
        let now = chrome_now();
        let mut cookies = Vec::new();
        let mut skipped = 0;
        for r in rows {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let n = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(0);
            let has_expires = n("has_expires") != 0;
            if has_expires && n("expires_utc") < now {
                skipped += 1;
                continue;
            }
            let host = s("host_key");
            let value = if !s("value").is_empty() {
                s("value")
            } else {
                match decrypt_value(&key, &hex(&s("ev")), &host, version) {
                    Some(v) => v,
                    None => {
                        skipped += 1;
                        continue;
                    }
                }
            };
            cookies.push(ImportedCookie {
                host,
                name: s("name"),
                value,
                path: s("path"),
                secure: n("is_secure") != 0,
                httponly: n("is_httponly") != 0,
                has_expires,
                expires: n("expires_utc"),
                creation: n("creation_utc"),
                last_access: n("last_access_utc"),
                samesite: n("samesite"),
            });
        }
        Ok(ImportResult { profile: profile.clone(), cookies, skipped })
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn sqlite(db: &std::path::Path, sql: &str) -> Result<Vec<serde_json::Map<String, Value>>, String> {
    let out = Command::new("/usr/bin/sqlite3").arg("-readonly").arg("-json").arg(db).arg(sql).output().map_err(|e| format!("sqlite3: {e}"))?;
    if !out.status.success() {
        return Err("读取 Cookie 数据库失败".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&text).map_err(|_| "Cookie 数据库格式无法识别".to_string())
}

/// Asks the Keychain (macOS shows the person an Allow/Deny prompt).
fn keychain_password() -> Result<Vec<u8>, String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-w", "-s", "Chrome Safe Storage", "-a", "Chrome"])
        .output()
        .map_err(|e| format!("security: {e}"))?;
    if !out.status.success() {
        return Err("没有得到钥匙串授权（或者点了拒绝），未导入任何内容".into());
    }
    let mut p = out.stdout;
    while p.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        p.pop();
    }
    Ok(p)
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn CCKeyDerivationPBKDF(algorithm: u32, password: *const u8, password_len: usize, salt: *const u8, salt_len: usize, prf: u32, rounds: u32, derived: *mut u8, derived_len: usize) -> i32;
    #[allow(clippy::too_many_arguments)]
    fn CCCrypt(op: u32, alg: u32, options: u32, key: *const u8, key_len: usize, iv: *const u8, data_in: *const u8, data_in_len: usize, data_out: *mut u8, data_out_avail: usize, data_out_moved: *mut usize) -> i32;
}

/// Chrome on macOS: PBKDF2-HMAC-SHA1(password, "saltysalt", 1003) -> 16 bytes.
pub fn derive_key(password: &[u8]) -> Result<[u8; 16], String> {
    let mut key = [0u8; 16];
    let salt = b"saltysalt";
    let r = unsafe { CCKeyDerivationPBKDF(2, password.as_ptr(), password.len(), salt.as_ptr(), salt.len(), 1, 1003, key.as_mut_ptr(), 16) };
    if r != 0 { Err("密钥派生失败".into()) } else { Ok(key) }
}

fn aes_cbc(decrypt: bool, key: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    let iv = [b' '; 16];
    let mut out = vec![0u8; data.len() + 16];
    let mut moved = 0usize;
    let r = unsafe { CCCrypt(if decrypt { 1 } else { 0 }, 0, 1, key.as_ptr(), 16, iv.as_ptr(), data.as_ptr(), data.len(), out.as_mut_ptr(), out.len(), &mut moved) };
    if r != 0 {
        return None;
    }
    out.truncate(moved);
    Some(out)
}

/// `v10` + AES-128-CBC; from database version 24 the plaintext starts with
/// SHA-256(host) (32 bytes), which is dropped.
pub fn decrypt_value(key: &[u8; 16], encrypted: &[u8], _host: &str, db_version: i64) -> Option<String> {
    let body = encrypted.strip_prefix(b"v10")?;
    let plain = aes_cbc(true, key, body)?;
    let plain = if db_version >= 24 && plain.len() >= 32 { &plain[32..] } else { &plain[..] };
    String::from_utf8(plain.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_what_chrome_encrypts() {
        let key = derive_key(b"test-password").unwrap();
        // Version 24 layout: 32-byte hash prefix, then the value.
        let mut plain = vec![7u8; 32];
        plain.extend_from_slice("session=中文✓".as_bytes());
        let mut enc = b"v10".to_vec();
        enc.extend(aes_cbc(false, &key, &plain).unwrap());
        assert_eq!(decrypt_value(&key, &enc, "x.y", 24).as_deref(), Some("session=中文✓"));
        assert_eq!(decrypt_value(&key, b"v11garbage", "x.y", 24), None);
        assert_ne!(decrypt_value(&derive_key(b"other").unwrap(), &enc, "x.y", 24).as_deref(), Some("session=中文✓"));
    }

    #[test]
    fn hex_parses() {
        assert_eq!(hex("00ff10"), vec![0, 255, 16]);
    }
}
