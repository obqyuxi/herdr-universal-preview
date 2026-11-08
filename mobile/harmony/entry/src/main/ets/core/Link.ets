// The pairing link in the bridge's QR code (docs/harmony/DESIGN.md §2):
// ddup://pair?v=1&h=<host>&h=<host2>&p=<port>&k=<key>&c=<code>&n=<name>
import { b64urlDecode } from './Bytes';

export class PairLink {
  hosts: string[] = [];
  port: number = 7841;
  key: Uint8Array = new Uint8Array(0);
  code: string = '';
  name: string = 'Mac';
}

function decode(s: string): string {
  try {
    return decodeURIComponent(s.replace(/\+/g, ' '));
  } catch (e) {
    return s;
  }
}

export function parsePairLink(text: string): PairLink {
  const s = text.trim();
  const prefix = 'ddup://pair?';
  if (!s.startsWith(prefix)) {
    throw new Error('不是配对链接（应以 ddup://pair? 开头）');
  }
  const link = new PairLink();
  for (const kv of s.substring(prefix.length).split('&')) {
    const at = kv.indexOf('=');
    const k = at < 0 ? kv : kv.substring(0, at);
    const v = at < 0 ? '' : decode(kv.substring(at + 1));
    if (k === 'h' && v.length > 0) {
      link.hosts.push(v);
    } else if (k === 'p') {
      link.port = Number.parseInt(v);
    } else if (k === 'k') {
      link.key = b64urlDecode(v);
    } else if (k === 'c') {
      link.code = v;
    } else if (k === 'n') {
      link.name = v;
    }
  }
  if (link.key.length !== 32) {
    throw new Error('配对链接缺少桥接服务公钥');
  }
  if (link.hosts.length === 0 || !link.code) {
    throw new Error('配对链接缺少地址或配对码');
  }
  return link;
}

export function wsUrl(host: string, port: number): string {
  const h = host.includes(':') ? `[${host}]` : host;
  return `ws://${h}:${port}/v1`;
}
