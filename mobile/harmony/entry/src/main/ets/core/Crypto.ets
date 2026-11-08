// DDB1 handshake and record encryption, matching bridge/src/crypto.rs
// (docs/harmony/DESIGN.md §3). Uses the synchronous Crypto Architecture Kit
// APIs (API 12+): X25519, SHA256, HKDF, AES-256-GCM.
import { cryptoFramework } from '@kit.CryptoArchitectureKit';
import { concat, utf8, u32be, readU32be, ByteQueue } from './Bytes';

function u64be(n: number): Uint8Array {
  const b = new Uint8Array(8);
  const v = new DataView(b.buffer);
  v.setUint32(0, Math.floor(n / 4294967296), false);
  v.setUint32(4, n >>> 0, false);
  return b;
}

function readU64be(b: Uint8Array): number {
  const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
  return v.getUint32(0, false) * 4294967296 + v.getUint32(4, false);
}

// DER prefixes: SubjectPublicKeyInfo and PKCS#8 for X25519 (OID 1.3.101.110).
const SPKI_X25519 = new Uint8Array([0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00]);
const PKCS8_X25519 = new Uint8Array([0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04, 0x20]);
export const MAGIC: Uint8Array = utf8('DDB1');
const INFO: Uint8Array = utf8('ddup-bridge v1 keys');
export const MAX_RECORD = 4 * 1024 * 1024;

export class XKey {
  priv: cryptoFramework.PriKey;
  pub: Uint8Array;

  constructor(priv: cryptoFramework.PriKey, pub: Uint8Array) {
    this.priv = priv;
    this.pub = pub;
  }
}

export function generateX25519(): XKey {
  const kp = cryptoFramework.createAsyKeyGenerator('X25519').generateKeyPairSync();
  const der = kp.pubKey.getEncoded().data;
  return new XKey(kp.priKey, der.slice(der.length - 32));
}

/// A private key from its raw 32 bytes (for the test vectors).
export function x25519Private(raw: Uint8Array): cryptoFramework.PriKey {
  const kp = cryptoFramework.createAsyKeyGenerator('X25519').convertKeySync(null, { data: concat([PKCS8_X25519, raw]) });
  return kp.priKey;
}

export function x25519(priv: cryptoFramework.PriKey, peerPub: Uint8Array): Uint8Array {
  const kp = cryptoFramework.createAsyKeyGenerator('X25519').convertKeySync({ data: concat([SPKI_X25519, peerPub]) }, null);
  return cryptoFramework.createKeyAgreement('X25519').generateSecretSync(priv, kp.pubKey).data;
}

export function sha256(data: Uint8Array): Uint8Array {
  const md = cryptoFramework.createMd('SHA256');
  md.updateSync({ data: data });
  return md.digestSync().data;
}

export function hkdfSha256(ikm: Uint8Array, salt: Uint8Array, info: Uint8Array, len: number): Uint8Array {
  const kdf = cryptoFramework.createKdf('HKDF|SHA256|EXTRACT_AND_EXPAND');
  const spec: cryptoFramework.HKDFSpec = { algName: 'HKDF', key: ikm, salt: salt, info: info, keySize: len };
  return kdf.generateSecretSync(spec).data;
}

/// One direction: AES-256-GCM, nonce = 4 zero bytes ‖ u64 BE counter, empty AAD.
export class Half {
  private key: cryptoFramework.SymKey;
  private counter: number = 0;
  /// The highest counter opened from a record (-1: none yet). Records may skip one.
  private last: number = -1;

  constructor(raw: Uint8Array) {
    this.key = cryptoFramework.createSymKeyGenerator('AES256').convertKeySync({ data: raw });
  }

  private nonce(): Uint8Array {
    const n = Half.nonceFor(this.counter);
    this.counter++;
    return n;
  }

  private static nonceFor(counter: number): Uint8Array {
    return concat([new Uint8Array(4), u64be(counter)]);
  }

  private spec(tag: Uint8Array, nonce?: Uint8Array): cryptoFramework.GcmParamsSpec {
    const s: cryptoFramework.GcmParamsSpec = {
      algName: 'GcmParamsSpec',
      iv: { data: nonce ?? this.nonce() },
      aad: { data: new Uint8Array(0) },
      authTag: { data: tag }
    };
    return s;
  }

  /// ciphertext ‖ 16-byte tag
  seal(plain: Uint8Array): Uint8Array {
    const c = cryptoFramework.createCipher('AES256|GCM|NoPadding');
    c.initSync(cryptoFramework.CryptoMode.ENCRYPT_MODE, this.key, this.spec(new Uint8Array(16)));
    const head = c.updateSync({ data: plain }).data;
    const tail = c.doFinalSync(null);
    return concat([head, tail ? tail.data : new Uint8Array(0)]);
  }

  /// u32 BE length ‖ u64 BE counter ‖ seal(plain): the counter travels with the
  /// record, so a send that fails doesn't break the records after it.
  sealRecord(plain: Uint8Array): Uint8Array {
    const counter = this.counter;
    const s = this.seal(plain);
    return concat([u32be(s.length + 8), u64be(counter), s]);
  }

  /// A record body (counter ‖ sealed); counters must only go up. Throws on failure.
  openRecord(body: Uint8Array): Uint8Array {
    if (body.length < 24) {
      throw new Error('record too short');
    }
    const counter = readU64be(body.slice(0, 8));
    if (counter <= this.last) {
      throw new Error('replayed or reordered record');
    }
    const sealed = body.slice(8);
    const c = cryptoFramework.createCipher('AES256|GCM|NoPadding');
    c.initSync(cryptoFramework.CryptoMode.DECRYPT_MODE, this.key, this.spec(sealed.slice(sealed.length - 16), Half.nonceFor(counter)));
    const head = c.updateSync({ data: sealed.slice(0, sealed.length - 16) }).data;
    const tail = c.doFinalSync(null);
    this.last = counter;
    return concat([head, tail ? tail.data : new Uint8Array(0)]);
  }

  /// Throws when the message fails authentication.
  open(sealed: Uint8Array): Uint8Array {
    if (sealed.length < 16) {
      throw new Error('record too short');
    }
    const c = cryptoFramework.createCipher('AES256|GCM|NoPadding');
    c.initSync(cryptoFramework.CryptoMode.DECRYPT_MODE, this.key, this.spec(sealed.slice(sealed.length - 16)));
    const head = c.updateSync({ data: sealed.slice(0, sealed.length - 16) }).data;
    const tail = c.doFinalSync(null);
    return concat([head, tail ? tail.data : new Uint8Array(0)]);
  }
}

export class Keys {
  send: Half;
  recv: Half;

  constructor(send: Half, recv: Half) {
    this.send = send;
    this.recv = recv;
  }
}

/// Phone side: finish the handshake from our ephemeral key, the bridge's pinned key and its reply.
export function clientKeys(ce: XKey, bridgePub: Uint8Array, msg2: Uint8Array): Keys {
  if (msg2.length !== 32) {
    throw new Error('bad DDB1 reply');
  }
  const dh1 = x25519(ce.priv, bridgePub);
  const dh2 = x25519(ce.priv, msg2);
  const salt = sha256(concat([MAGIC, bridgePub, ce.pub, msg2]));
  const okm = hkdfSha256(concat([dh1, dh2]), salt, INFO, 64);
  return new Keys(new Half(okm.slice(0, 32)), new Half(okm.slice(32, 64)));
}

export function clientHello(ce: XKey): Uint8Array {
  return concat([MAGIC, ce.pub]);
}

/// Sealed records out of a byte stream (WebSocket messages may arrive split or joined).
export class Records {
  private q: ByteQueue = new ByteQueue();

  push(b: Uint8Array): void {
    this.q.push(b);
  }

  pop(): Uint8Array | null {
    if (this.q.length < 4) {
      return null;
    }
    const len = readU32be(this.q.peek(), 0);
    if (len > MAX_RECORD) {
      throw new Error('record too large');
    }
    if (this.q.length < 4 + len) {
      return null;
    }
    this.q.take(4);
    return this.q.take(len);
  }
}
