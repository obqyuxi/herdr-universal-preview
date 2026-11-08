//! DDB1: the handshake and message encryption between the phone app and the
//! bridge (docs/harmony/DESIGN.md §3). The QR code pins the bridge's static
//! X25519 key; both sides add an ephemeral key for forward secrecy.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Result, anyhow, bail};
use hkdf::Hkdf;
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub const MAGIC: &[u8; 4] = b"DDB1";
const INFO: &[u8] = b"ddup-bridge v1 keys";

/// One direction of an encrypted connection: AES-256-GCM, nonce = 4 zero bytes ‖ u64 BE counter.
pub struct Half {
    cipher: Aes256Gcm,
    counter: u64,
    /// The highest counter opened from a record (records may skip one: a send can fail).
    last: Option<u64>,
}

impl Half {
    fn new(key: &[u8]) -> Self {
        Half { cipher: Aes256Gcm::new_from_slice(key).expect("32-byte key"), counter: 0, last: None }
    }

    fn nonce(&mut self) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter += 1;
        n
    }

    pub fn seal(&mut self, plain: &[u8]) -> Vec<u8> {
        let n = self.nonce();
        self.cipher.encrypt(Nonce::from_slice(&n), plain).expect("aes-gcm encrypt")
    }

    /// A record: u32 BE length ‖ u64 BE counter ‖ sealed. Carrying the counter lets
    /// the other side open later records when one was lost on the way.
    pub fn seal_record(&mut self, plain: &[u8]) -> Vec<u8> {
        let counter = self.counter;
        let sealed = self.seal(plain);
        let mut r = ((sealed.len() + 8) as u32).to_be_bytes().to_vec();
        r.extend(counter.to_be_bytes());
        r.extend(sealed);
        r
    }

    /// Open a record body (counter ‖ sealed): counters must only go up (no replay, no reordering).
    pub fn open_record(&mut self, body: &[u8]) -> Result<Vec<u8>> {
        if body.len() < 8 + 16 {
            bail!("record too short");
        }
        let counter = u64::from_be_bytes(body[..8].try_into().unwrap());
        if self.last.is_some_and(|l| counter <= l) {
            bail!("replayed or reordered record");
        }
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&counter.to_be_bytes());
        let plain = self.cipher.decrypt(Nonce::from_slice(&n), &body[8..]).map_err(|_| anyhow!("message failed authentication"))?;
        self.last = Some(counter);
        Ok(plain)
    }

    /// Sequential counterpart of `seal` (the vectors and tests use it; the wire uses records).
    #[cfg(test)]
    pub fn open(&mut self, sealed: &[u8]) -> Result<Vec<u8>> {
        let n = self.nonce();
        self.cipher.decrypt(Nonce::from_slice(&n), sealed).map_err(|_| anyhow!("message failed authentication"))
    }
}

pub const MAX_RECORD: usize = 4 << 20;

/// Sealed records out of a byte stream. A WebSocket client may hand one
/// message over in parts or several at once, so records carry their length.
#[derive(Default)]
pub struct Records {
    buf: Vec<u8>,
}

impl Records {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn pop(&mut self) -> Result<Option<Vec<u8>>> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes(self.buf[..4].try_into().unwrap()) as usize;
        if len > MAX_RECORD {
            bail!("record too large ({len} bytes)");
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let r = self.buf[4..4 + len].to_vec();
        self.buf.drain(..4 + len);
        Ok(Some(r))
    }
}

/// Both directions, from the side that owns them.
pub struct Keys {
    pub send: Half,
    pub recv: Half,
}

/// `(k_c2s, k_s2c)` from the two shared secrets and the transcript.
pub fn derive(dh1: &[u8; 32], dh2: &[u8; 32], bs: &[u8; 32], ce: &[u8; 32], se: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut th = Sha256::new();
    th.update(MAGIC);
    th.update(bs);
    th.update(ce);
    th.update(se);
    let salt = th.finalize();
    let mut ikm = [0u8; 64];
    ikm[..32].copy_from_slice(dh1);
    ikm[32..].copy_from_slice(dh2);
    let mut okm = [0u8; 64];
    Hkdf::<Sha256>::new(Some(&salt), &ikm).expand(INFO, &mut okm).expect("64 bytes is a valid HKDF length");
    let (mut c2s, mut s2c) = ([0u8; 32], [0u8; 32]);
    c2s.copy_from_slice(&okm[..32]);
    s2c.copy_from_slice(&okm[32..]);
    (c2s, s2c)
}

pub fn public_of(secret: &StaticSecret) -> [u8; 32] {
    PublicKey::from(secret).to_bytes()
}

pub fn random_secret() -> StaticSecret {
    StaticSecret::random_from_rng(OsRng)
}

/// Bridge side: answer the phone's first message; returns the reply and the keys.
pub fn server_handshake(bridge: &StaticSecret, msg1: &[u8]) -> Result<(Vec<u8>, Keys)> {
    server_handshake_with(bridge, msg1, random_secret())
}

fn server_handshake_with(bridge: &StaticSecret, msg1: &[u8], se: StaticSecret) -> Result<(Vec<u8>, Keys)> {
    if msg1.len() != 36 || &msg1[..4] != MAGIC {
        bail!("not a DDB1 hello");
    }
    let ce: [u8; 32] = msg1[4..].try_into().unwrap();
    let ce_pub = PublicKey::from(ce);
    let dh1 = bridge.diffie_hellman(&ce_pub).to_bytes();
    let dh2 = se.diffie_hellman(&ce_pub).to_bytes();
    let se_pub = public_of(&se);
    let (c2s, s2c) = derive(&dh1, &dh2, &public_of(bridge), &ce, &se_pub);
    Ok((se_pub.to_vec(), Keys { send: Half::new(&s2c), recv: Half::new(&c2s) }))
}

/// Phone side (used by `probe` and the tests): the first message and the secret to finish with.
pub fn client_start() -> (StaticSecret, Vec<u8>) {
    let ce = random_secret();
    let mut msg1 = MAGIC.to_vec();
    msg1.extend_from_slice(&public_of(&ce));
    (ce, msg1)
}

pub fn client_finish(ce: &StaticSecret, bridge_pub: &[u8; 32], msg2: &[u8]) -> Result<Keys> {
    let se: [u8; 32] = msg2.try_into().map_err(|_| anyhow!("bad DDB1 reply"))?;
    let dh1 = ce.diffie_hellman(&PublicKey::from(*bridge_pub)).to_bytes();
    let dh2 = ce.diffie_hellman(&PublicKey::from(se)).to_bytes();
    let (c2s, s2c) = derive(&dh1, &dh2, bridge_pub, &public_of(ce), &se);
    Ok(Keys { send: Half::new(&c2s), recv: Half::new(&s2c) })
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

    #[test]
    fn both_sides_agree_and_counters_advance() {
        let bridge = random_secret();
        let (ce, msg1) = client_start();
        let (msg2, mut server) = server_handshake(&bridge, &msg1).unwrap();
        let mut client = client_finish(&ce, &public_of(&bridge), &msg2).unwrap();
        for i in 0..3u8 {
            let sealed = client.send.seal(&[1, i]);
            assert_eq!(server.recv.open(&sealed).unwrap(), vec![1, i]);
            let back = server.send.seal(b"ok");
            assert_eq!(client.recv.open(&back).unwrap(), b"ok");
        }
    }

    #[test]
    fn records_survive_splits_and_joins() {
        let bridge = random_secret();
        let (ce, msg1) = client_start();
        let (msg2, mut server) = server_handshake(&bridge, &msg1).unwrap();
        let mut client = client_finish(&ce, &public_of(&bridge), &msg2).unwrap();
        let mut stream = client.send.seal_record(b"one");
        stream.extend(client.send.seal_record(b"two"));
        let mut r = Records::default();
        let mut got = vec![];
        for chunk in stream.chunks(5) {
            r.push(chunk);
            while let Some(rec) = r.pop().unwrap() {
                got.push(server.recv.open_record(&rec).unwrap());
            }
        }
        assert_eq!(got, vec![b"one".to_vec(), b"two".to_vec()]);
    }

    #[test]
    fn a_lost_record_does_not_break_the_next_but_replays_fail() {
        let bridge = random_secret();
        let (ce, msg1) = client_start();
        let (msg2, mut server) = server_handshake(&bridge, &msg1).unwrap();
        let mut client = client_finish(&ce, &public_of(&bridge), &msg2).unwrap();
        let first = client.send.seal_record(b"a");
        let _lost = client.send.seal_record(b"b");
        let third = client.send.seal_record(b"c");
        assert_eq!(server.recv.open_record(&first[4..]).unwrap(), b"a");
        assert_eq!(server.recv.open_record(&third[4..]).unwrap(), b"c");
        assert!(server.recv.open_record(&first[4..]).is_err(), "replay");
        let mut bad = client.send.seal_record(b"d");
        let at = bad.len() - 1;
        bad[at] ^= 1;
        assert!(server.recv.open_record(&bad[4..]).is_err(), "tampered");
    }

    #[test]
    fn an_impostor_bridge_cannot_finish() {
        let real = random_secret();
        let fake = random_secret();
        let (ce, msg1) = client_start();
        let (msg2, mut server) = server_handshake(&fake, &msg1).unwrap();
        let mut client = client_finish(&ce, &public_of(&real), &msg2).unwrap();
        assert!(client.recv.open(&server.send.seal(b"hi")).is_err());
    }

    #[test]
    fn replay_and_tamper_fail() {
        let bridge = random_secret();
        let (ce, msg1) = client_start();
        let (msg2, mut server) = server_handshake(&bridge, &msg1).unwrap();
        let mut client = client_finish(&ce, &public_of(&bridge), &msg2).unwrap();
        let a = client.send.seal(b"a");
        assert!(server.recv.open(&a).is_ok());
        assert!(server.recv.open(&a).is_err(), "replayed message must not open");
        let mut b = client.send.seal(b"b");
        b[0] ^= 1;
        assert!(server.recv.open(&b).is_err());
    }

    /// Fixed vectors for checking another implementation (the ArkTS client):
    /// `cargo test vectors -- --nocapture` prints them; mobile/harmony keeps a copy.
    #[test]
    fn vectors() {
        let bs = StaticSecret::from([1u8; 32]);
        let ce = StaticSecret::from([2u8; 32]);
        let se = StaticSecret::from([3u8; 32]);
        let mut msg1 = MAGIC.to_vec();
        msg1.extend_from_slice(&public_of(&ce));
        let (msg2, mut server) = server_handshake_with(&bs, &msg1, se).unwrap();
        let mut client = client_finish(&ce, &public_of(&bs), &msg2).unwrap();
        let c0 = client.send.seal(b"\x01{\"t\":\"auth\"}");
        let s0 = server.send.seal(b"\x00hello");
        assert_eq!(server.recv.open(&c0).unwrap(), b"\x01{\"t\":\"auth\"}");
        let v = serde_json::json!({
            "bridge_secret": B64.encode([1u8; 32]), "bridge_public": B64.encode(public_of(&bs)),
            "client_secret": B64.encode([2u8; 32]), "client_public": B64.encode(public_of(&ce)),
            "server_ephemeral_secret": B64.encode([3u8; 32]), "msg2": B64.encode(&msg2),
            "c2s_0_plain": B64.encode(b"\x01{\"t\":\"auth\"}"), "c2s_0_sealed": B64.encode(&c0),
            "s2c_0_plain": B64.encode(b"\x00hello"), "s2c_0_sealed": B64.encode(&s0),
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        // Pinned so a change to the scheme is noticed (and the ArkTS copy updated).
        assert_eq!(B64.encode(&c0), include_str!("../testdata/c2s_0_sealed.txt").trim());
    }
}
