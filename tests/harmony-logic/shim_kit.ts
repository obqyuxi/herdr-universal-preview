// Node stand-ins for the HarmonyOS kits the app's core modules use, so the
// .ets logic (crypto, framing, protocol, session) runs unchanged against the
// real bridge and test core. Semantics follow the HarmonyOS docs:
// AES-GCM encrypt -> update = ciphertext, doFinal = 16-byte tag; decrypt
// takes the tag in GcmParamsSpec. The WebSocket shim splits incoming
// messages at random points, as a client may deliver them in parts.
import * as nc from 'node:crypto';

export class BusinessError extends Error {
  code: number = 0;
}

// ---------- @kit.ArkTS util ----------
class TextEncoderShim {
  encodeInto(s: string): Uint8Array {
    return new Uint8Array(Buffer.from(s, 'utf8'));
  }
}
class TextDecoderShim {
  static create(enc: string, opts: object): TextDecoderShim {
    return new TextDecoderShim();
  }
  decodeToString(b: Uint8Array, opts: object): string {
    return Buffer.from(b).toString('utf8');
  }
}
class Base64HelperShim {
  encodeToStringSync(b: Uint8Array): string { return Buffer.from(b).toString('base64'); }
  decodeSync(s: string): Uint8Array { return new Uint8Array(Buffer.from(s, 'base64')); }
}
export const util = { TextEncoder: TextEncoderShim, TextDecoder: TextDecoderShim, Base64Helper: Base64HelperShim };

// ---------- @kit.CryptoArchitectureKit ----------
type Blob = { data: Uint8Array };
const blob = (b: Uint8Array | Buffer): Blob => ({ data: new Uint8Array(b) });

class PubKey {
  k: nc.KeyObject;
  constructor(k: nc.KeyObject) { this.k = k; }
  getEncoded(): Blob {
    return blob(this.k.export({ format: 'der', type: 'spki' }) as Buffer);
  }
}
class PriKey {
  k: nc.KeyObject;
  constructor(k: nc.KeyObject) { this.k = k; }
}

export const cryptoFramework = {
  CryptoMode: { ENCRYPT_MODE: 0, DECRYPT_MODE: 1 },
  createAsyKeyGenerator(alg: string) {
    if (alg !== 'X25519') throw new Error('shim: only X25519');
    return {
      generateKeyPairSync() {
        const kp = nc.generateKeyPairSync('x25519');
        return { pubKey: new PubKey(kp.publicKey), priKey: new PriKey(kp.privateKey) };
      },
      convertKeySync(pub: Blob | null, pri: Blob | null) {
        return {
          pubKey: pub ? new PubKey(nc.createPublicKey({ key: Buffer.from(pub.data), format: 'der', type: 'spki' })) : null,
          priKey: pri ? new PriKey(nc.createPrivateKey({ key: Buffer.from(pri.data), format: 'der', type: 'pkcs8' })) : null,
        };
      },
    };
  },
  createKeyAgreement(alg: string) {
    return {
      generateSecretSync(pri: PriKey, pub: PubKey): Blob {
        return blob(nc.diffieHellman({ privateKey: pri.k, publicKey: pub.k }));
      },
    };
  },
  createMd(alg: string) {
    const h = nc.createHash('sha256');
    return {
      updateSync(b: Blob) { h.update(b.data); },
      digestSync(): Blob { return blob(h.digest()); },
    };
  },
  createKdf(alg: string) {
    if (alg !== 'HKDF|SHA256|EXTRACT_AND_EXPAND') throw new Error('shim: ' + alg);
    return {
      generateSecretSync(s: { key: Uint8Array; salt: Uint8Array; info: Uint8Array; keySize: number }): Blob {
        return blob(Buffer.from(nc.hkdfSync('sha256', s.key, s.salt, s.info, s.keySize)));
      },
    };
  },
  createSymKeyGenerator(alg: string) {
    return { convertKeySync(b: Blob) { return { raw: Buffer.from(b.data) }; } };
  },
  createCipher(alg: string) {
    if (alg !== 'AES256|GCM|NoPadding') throw new Error('shim: ' + alg);
    let c: nc.CipherGCM | nc.DecipherGCM | null = null;
    let enc = true;
    return {
      initSync(mode: number, key: { raw: Buffer }, spec: { iv: Blob; aad: Blob; authTag: Blob }) {
        enc = mode === 0;
        if (enc) {
          c = nc.createCipheriv('aes-256-gcm', key.raw, Buffer.from(spec.iv.data));
        } else {
          const d = nc.createDecipheriv('aes-256-gcm', key.raw, Buffer.from(spec.iv.data));
          d.setAuthTag(Buffer.from(spec.authTag.data));
          c = d;
        }
        if (spec.aad.data.length) c.setAAD(Buffer.from(spec.aad.data));
      },
      updateSync(b: Blob): Blob { return blob(c!.update(b.data)); },
      doFinalSync(b: null): Blob {
        const tail = c!.final();
        return enc ? blob(Buffer.concat([tail, (c as nc.CipherGCM).getAuthTag()])) : blob(tail);
      },
    };
  },
};

// ---------- @kit.NetworkKit webSocket ----------
class WsShim {
  private ws: WebSocket | null = null;
  private handlers: Record<string, (err: unknown, v?: unknown) => void> = {};
  on(type: string, cb: (err: unknown, v?: unknown) => void) { this.handlers[type] = cb; }
  connect(url: string): Promise<boolean> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      ws.binaryType = 'arraybuffer';
      this.ws = ws;
      ws.onopen = () => { resolve(true); this.handlers.open?.(undefined, {}); };
      ws.onerror = () => { const e = new BusinessError('ws error'); reject(e); this.handlers.error?.(e); };
      ws.onclose = (ev) => this.handlers.close?.(undefined, { code: ev.code, reason: ev.reason });
      ws.onmessage = (ev) => {
        const all = new Uint8Array(ev.data as ArrayBuffer);
        // Deliver in random parts, like a client that hands a message over piecewise.
        let at = 0;
        while (at < all.length) {
          const n = Math.max(1, Math.floor(Math.random() * Math.min(all.length - at, 700)) + 1);
          this.handlers.message?.(undefined, all.slice(at, at + n).buffer);
          at += n;
        }
      };
    });
  }
  // Like HarmonyOS: a send completes later, after a random delay, so two sends in
  // flight can go out swapped (the app must wait for one before the next).
  send(data: ArrayBuffer | string): Promise<boolean> {
    return new Promise((resolve) => setTimeout(() => { this.ws?.send(data); resolve(true); }, Math.random() * 15));
  }
  close(): Promise<boolean> { this.ws?.close(); return Promise.resolve(true); }
}
export const webSocket = { createWebSocket: () => new WsShim() };

// ---------- @kit.AssetStoreKit ----------
const assets = new Map<string, Uint8Array>();
export const asset = {
  Tag: { ALIAS: 'alias', SECRET: 'secret', RETURN_TYPE: 'rt', ACCESSIBILITY: 'acc' },
  ReturnType: { ALL: 1 },
  Accessibility: { DEVICE_FIRST_UNLOCKED: 1 },
  async add(m: Map<string, unknown>) {
    const k = Buffer.from(m.get('alias') as Uint8Array).toString();
    if (assets.has(k)) throw new BusinessError('duplicate');
    assets.set(k, m.get('secret') as Uint8Array);
  },
  async query(m: Map<string, unknown>) {
    const k = Buffer.from(m.get('alias') as Uint8Array).toString();
    if (!assets.has(k)) throw new BusinessError('not found');
    return [new Map<string, unknown>([['secret', assets.get(k)]])];
  },
  async remove(m: Map<string, unknown>) {
    const k = Buffer.from(m.get('alias') as Uint8Array).toString();
    if (!assets.delete(k)) throw new BusinessError('not found');
  },
};

// ---------- @kit.BasicServicesKit ----------
export const deviceInfo = { marketName: 'harness', productModel: 'node' };

// ---------- @kit.PerformanceAnalysisKit ----------
export const hilog = {
  error: (_d: number, _t: string, fmt: string, ...a: unknown[]) => console.error('[hilog]', fmt, ...a),
  warn: (_d: number, _t: string, fmt: string, ...a: unknown[]) => console.error('[hilog]', fmt, ...a),
  info: () => {},
};

// ---------- @kit.FormKit / @kit.ArkData / @kit.AbilityKit (home-screen cards) ----------
export const formBindingData = { createFormBindingData: (o: object) => ({ data: o }) };
export const formProvider = { updateForm: async (id: string, b: object) => {} };
const prefs = new Map<string, unknown>();
export const preferences = {
  removePreferencesFromCacheSync: (...a: unknown[]) => {},
  getPreferencesSync: (...a: unknown[]) => ({
    getSync: (k: string, d: unknown) => (prefs.has(k) ? prefs.get(k) : d),
    putSync: (k: string, v: unknown) => { prefs.set(k, v); },
    flushSync: () => {},
  }),
};
export const common = {};

// ---------- @kit.NetworkKit (network changes: none in the harness) ----------
export const connection = {
  createNetConnection: () => ({
    on: (_t: string, _cb: (h: object) => void) => {},
    register: (cb: (e?: BusinessError) => void) => cb(undefined),
  }),
};
