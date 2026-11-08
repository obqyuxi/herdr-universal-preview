// Byte helpers: concatenation, UTF-8, base64url (no padding), little/big-endian integers.
import { util } from '@kit.ArkTS';

const encoder: util.TextEncoder = new util.TextEncoder();
const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';

export function concat(parts: Uint8Array[]): Uint8Array {
  let n = 0;
  for (const p of parts) {
    n += p.length;
  }
  const out = new Uint8Array(n);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

export function utf8(s: string): Uint8Array {
  return encoder.encodeInto(s);
}

export function fromUtf8(b: Uint8Array): string {
  const d = util.TextDecoder.create('utf-8', { ignoreBOM: true });
  return d.decodeToString(b, { stream: false });
}

export function b64urlEncode(b: Uint8Array): string {
  let out = '';
  let i = 0;
  for (; i + 2 < b.length; i += 3) {
    const n = (b[i] << 16) | (b[i + 1] << 8) | b[i + 2];
    out += B64.charAt((n >> 18) & 63) + B64.charAt((n >> 12) & 63) + B64.charAt((n >> 6) & 63) + B64.charAt(n & 63);
  }
  if (b.length - i === 1) {
    const n = b[i] << 16;
    out += B64.charAt((n >> 18) & 63) + B64.charAt((n >> 12) & 63);
  } else if (b.length - i === 2) {
    const n = (b[i] << 16) | (b[i + 1] << 8);
    out += B64.charAt((n >> 18) & 63) + B64.charAt((n >> 12) & 63) + B64.charAt((n >> 6) & 63);
  }
  return out;
}

export function b64urlDecode(s: string): Uint8Array {
  const clean = s.replace(/=+$/, '').replace(/\+/g, '-').replace(/\//g, '_');
  const out: number[] = [];
  let acc = 0;
  let bits = 0;
  for (let i = 0; i < clean.length; i++) {
    const v = B64.indexOf(clean.charAt(i));
    if (v < 0) {
      throw new Error('bad base64');
    }
    acc = (acc << 6) | v;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out.push((acc >> bits) & 0xff);
    }
  }
  return new Uint8Array(out);
}

export function u32le(n: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n >>> 0, true);
  return b;
}

export function u32be(n: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n >>> 0, false);
  return b;
}

export function readU32le(b: Uint8Array, at: number): number {
  return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(at, true);
}

export function readU32be(b: Uint8Array, at: number): number {
  return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(at, false);
}

export function readU16le(b: Uint8Array, at: number): number {
  return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint16(at, true);
}

export function equal(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) {
    return false;
  }
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) {
      return false;
    }
  }
  return true;
}

/// A growable byte queue for parsing streams.
export class ByteQueue {
  private buf: Uint8Array = new Uint8Array(0);

  get length(): number {
    return this.buf.length;
  }

  push(b: Uint8Array): void {
    this.buf = this.buf.length === 0 ? b.slice() : concat([this.buf, b]);
  }

  peek(): Uint8Array {
    return this.buf;
  }

  take(n: number): Uint8Array {
    const out = this.buf.slice(0, n);
    this.buf = this.buf.slice(n);
    return out;
  }

  indexOf(byte: number): number {
    return this.buf.indexOf(byte);
  }
}
