// One encrypted connection to ddup-bridge: WebSocket, DDB1 handshake, then
// records of type 0x00 (hc connect bytes) or 0x01 (control JSON).
// docs/harmony/DESIGN.md §3–4.
import { webSocket } from '@kit.NetworkKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { concat, fromUtf8, utf8, ByteQueue } from './Bytes';
import { clientHello, clientKeys, generateX25519, Keys, Records, XKey } from './Crypto';

export const HC: number = 0x00;
export const CONTROL: number = 0x01;

/// Every control message, both directions (fields used depend on `t`).
/// One entry of an agent's session log, as the bridge summarises it (bridge/src/transcript.rs).
export interface ChatItem {
  /// user | assistant | tool | result
  k: string;
  text: string;
  name?: string;
  detail?: string;
  id?: string;
  err?: boolean;
  /// Byte offset of the log line it came from.
  o?: number;
}

/// A local web server on the Mac (bridge/src/ports.rs).
export interface SiteInfo {
  port: number;
  process: string;
  pid: number;
  cwd?: string;
  title?: string;
  status: number;
}

export interface ControlMsg {
  t: string;
  /// Matches a request to its reply (transcript, ports, forward).
  id?: number;
  error?: string;
  app?: string;
  cwd?: string;
  pid?: number;
  file?: string;
  offset?: number;
  reset?: boolean;
  truncated?: boolean;
  start?: number;
  older?: boolean;
  before?: number;
  items?: ChatItem[];
  sites?: SiteInfo[];
  port?: number;
  listen_port?: number;
  code?: string;
  msg?: string;
  device?: string;
  device_id?: string;
  token?: string;
  name?: string;
  bridge?: string;
  on?: boolean;
  ok?: boolean;
  re?: string;
  detail?: string;
}

function toArrayBuffer(b: Uint8Array): ArrayBuffer {
  return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer;
}

export class SecureSocket {
  private ws: webSocket.WebSocket = webSocket.createWebSocket();
  private keys: Keys | null = null;
  private records: Records = new Records();
  private pre: ByteQueue = new ByteQueue();
  private ce: XKey = generateX25519();
  private settled: boolean = false;
  private queue: ArrayBuffer[] = [];
  private sending: boolean = false;
  private closed: boolean = false;
  onHc: (b: Uint8Array) => void = (b: Uint8Array) => {};
  onControl: (m: ControlMsg) => void = (m: ControlMsg) => {};
  onClose: (why: string) => void = (why: string) => {};

  /// Connect, shake hands, send `first` (pair or auth) and resolve with the bridge's answer.
  open(url: string, bridgeKey: Uint8Array, first: ControlMsg, timeoutMs: number): Promise<ControlMsg> {
    return new Promise<ControlMsg>((resolve, reject) => {
      const timer = setTimeout(() => {
        if (!this.settled) {
          this.settled = true;
          this.close();
          reject(new Error('连接超时'));
        }
      }, timeoutMs);
      const fail = (why: string) => {
        if (!this.settled) {
          this.settled = true;
          clearTimeout(timer);
          reject(new Error(why));
        } else if (!this.closed) {
          this.closed = true;
          this.onClose(why);
        }
      };
      this.ws.on('open', (err: BusinessError, value: Object) => {
        this.ws.send(toArrayBuffer(clientHello(this.ce)));
      });
      this.ws.on('message', (err: BusinessError, value: string | ArrayBuffer) => {
        if (err || typeof value === 'string') {
          return;
        }
        try {
          this.receive(new Uint8Array(value as ArrayBuffer), bridgeKey, first, (m: ControlMsg) => {
            if (this.settled) {
              this.onControl(m);
              return;
            }
            this.settled = true;
            clearTimeout(timer);
            if (m.t === 'error') {
              this.close();
              reject(new Error(m.code ?? 'error'));
            } else {
              resolve(m);
            }
          });
        } catch (e) {
          this.close();
          fail(`加密通道出错：${(e as Error).message}`);
        }
      });
      this.ws.on('close', (err: BusinessError, value: webSocket.CloseResult): void => {
        fail('连接已断开');
      });
      this.ws.on('error', (err: BusinessError): void => {
        fail(err.message ? `连接失败：${err.message}` : '连接失败');
      });
      this.ws.connect(url).catch((e: BusinessError): void => {
        fail(`连接失败：${e.message}`);
      });
    });
  }

  private receive(bytes: Uint8Array, bridgeKey: Uint8Array, first: ControlMsg, control: (m: ControlMsg) => void): void {
    let keys = this.keys;
    if (keys === null) {
      this.pre.push(bytes);
      if (this.pre.length < 32) {
        return;
      }
      keys = clientKeys(this.ce, bridgeKey, this.pre.take(32));
      this.keys = keys;
      this.sendControl(first);
      if (this.pre.length > 0) {
        this.records.push(this.pre.take(this.pre.length));
      }
    } else {
      this.records.push(bytes);
    }
    let r = this.records.pop();
    while (r !== null) {
      const plain = keys.recv.openRecord(r);
      if (plain[0] === HC) {
        this.onHc(plain.slice(1));
      } else if (plain[0] === CONTROL) {
        control(JSON.parse(fromUtf8(plain.slice(1))) as ControlMsg);
      }
      r = this.records.pop();
    }
  }

  private sendPlain(plain: Uint8Array): void {
    const keys = this.keys;
    if (keys === null || this.closed) {
      return;
    }
    // Sealed now, in order; sent one at a time: ws.send is asynchronous and two sends
    // in flight can reach the bridge swapped, which it rejects as a reordered record.
    this.queue.push(toArrayBuffer(keys.send.sealRecord(plain)));
    this.pump();
  }

  private pump(): void {
    if (this.sending || this.queue.length === 0 || this.closed) {
      return;
    }
    const next = this.queue.shift() as ArrayBuffer;
    this.sending = true;
    this.ws.send(next).then((ok: boolean) => {
      if (!ok) {
        hilog.error(0xd0d0, 'ddup', 'ws.send returned false (%{public}d bytes)', next.byteLength);
      }
      this.sending = false;
      this.pump();
    }).catch((e: BusinessError) => {
      hilog.error(0xd0d0, 'ddup', 'ws.send failed: %{public}d %{public}s', e.code, e.message);
      this.sending = false;
      this.pump();
    });
  }

  sendControl(m: ControlMsg): void {
    this.sendPlain(concat([new Uint8Array([CONTROL]), utf8(JSON.stringify(m))]));
  }

  sendHc(b: Uint8Array): void {
    this.sendPlain(concat([new Uint8Array([HC]), b]));
  }

  close(): void {
    this.closed = true;
    this.ws.close().catch((): void => {});
  }
}
