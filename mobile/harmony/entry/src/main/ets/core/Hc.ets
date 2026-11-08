// herdr-core's framed protocol, as the desktop app speaks it
// (.work/herdr-app/src/hc.rs): a hello line, then frames of
// u32 LE channel ‖ u8 type ‖ u32 LE length ‖ payload. Channel 0 carries
// JSON-RPC lines; attach and events.subscribe open their own channels.
import { concat, fromUtf8, readU16le, readU32le, u32le, utf8, ByteQueue } from './Bytes';

export const DATA: number = 0;
export const CREDIT: number = 1;
export const CLOSE: number = 2;
export const RESIZE: number = 3;
const NL: number = 10;

export interface RpcErrorData {
  kind?: string;
  hint?: string;
}

export interface RpcError {
  code?: number;
  message?: string;
  data?: RpcErrorData;
}

export interface RpcLine {
  id?: number;
  method?: string;
  result?: Object;
  error?: RpcError;
  params?: Object;
}

export interface CoreInfo {
  id?: string;
  name?: string;
}

export interface HelloResult {
  version?: string;
  generation?: number;
  core?: CoreInfo;
  methods?: string[];
}

/// A core error as an Error; its `name` carries the core's error kind (e.g. "blocked").
export function rpcError(e: RpcError): Error {
  const err = new Error(e.message ?? 'core call failed');
  err.name = e.data?.kind ?? 'HcError';
  return err;
}

/// What a stream channel delivers.
export interface StreamSink {
  data: (b: Uint8Array) => void;
  resize: (cols: number, rows: number) => void;
  closed: (why: string) => void;
}

class Pending {
  resolve: (v: Object) => void;
  reject: (e: Error) => void;

  constructor(resolve: (v: Object) => void, reject: (e: Error) => void) {
    this.resolve = resolve;
    this.reject = reject;
  }
}

class Frame {
  kind: number;
  payload: Uint8Array;

  constructor(kind: number, payload: Uint8Array) {
    this.kind = kind;
    this.payload = payload;
  }
}

class RpcRequest {
  jsonrpc: string = '2.0';
  id: number;
  method: string;
  params: Object;

  constructor(id: number, method: string, params: Object) {
    this.id = id;
    this.method = method;
    this.params = params;
  }
}

export class HcClient {
  private out: (b: Uint8Array) => void;
  private q: ByteQueue = new ByteQueue();
  private helloWaiter: Pending | null = null;
  private helloDone: boolean = false;
  private nextId: number = 1;
  private pending: Map<number, Pending> = new Map();
  private streams: Map<number, StreamSink> = new Map();
  private early: Map<number, Frame[]> = new Map();

  constructor(out: (b: Uint8Array) => void) {
    this.out = out;
  }

  /// Switch the connection to framed mode; resolves with the core's hello.
  start(): Promise<HelloResult> {
    return new Promise<HelloResult>((resolve, reject) => {
      this.helloWaiter = new Pending((v: Object) => resolve(v as HelloResult), reject);
      this.out(utf8('{"jsonrpc":"2.0","id":0,"method":"hello","params":{"framed":true}}\n'));
    });
  }

  feed(b: Uint8Array): void {
    this.q.push(b);
    if (!this.helloDone) {
      const nl = this.q.indexOf(NL);
      if (nl < 0) {
        return;
      }
      const line = JSON.parse(fromUtf8(this.q.take(nl + 1).slice(0, nl))) as RpcLine;
      this.helloDone = true;
      const w = this.helloWaiter;
      this.helloWaiter = null;
      if (w !== null) {
        if (line.error !== undefined) {
          w.reject(rpcError(line.error));
        } else {
          w.resolve(line.result ?? new Object());
        }
      }
    }
    while (this.q.length >= 9) {
      const head = this.q.peek();
      const len = readU32le(head, 5);
      if (this.q.length < 9 + len) {
        return;
      }
      const ch = readU32le(head, 0);
      const kind = head[4];
      this.q.take(9);
      this.frame(ch, kind, this.q.take(len));
    }
  }

  private frame(ch: number, kind: number, payload: Uint8Array): void {
    if (ch === 0) {
      // Each DATA frame on channel 0 is one whole JSON-RPC message (hc.rs reads it with from_slice).
      if (kind === DATA && payload.length > 0) {
        this.reply(JSON.parse(fromUtf8(payload)) as RpcLine);
      }
      return;
    }
    const sink = this.streams.get(ch);
    if (sink === undefined) {
      const list = this.early.get(ch) ?? [];
      list.push(new Frame(kind, payload));
      this.early.set(ch, list);
      return;
    }
    this.deliver(ch, sink, new Frame(kind, payload));
  }

  private deliver(ch: number, sink: StreamSink, f: Frame): void {
    if (f.kind === DATA) {
      sink.data(f.payload);
    } else if (f.kind === RESIZE && f.payload.length >= 4) {
      sink.resize(readU16le(f.payload, 0), readU16le(f.payload, 2));
    } else if (f.kind === CLOSE) {
      this.streams.delete(ch);
      sink.closed(f.payload.length > 0 ? fromUtf8(f.payload) : 'closed');
    }
  }

  private reply(line: RpcLine): void {
    if (line.id === undefined) {
      return;
    }
    const p = this.pending.get(line.id);
    if (p === undefined) {
      return;
    }
    this.pending.delete(line.id);
    if (line.error !== undefined) {
      p.reject(rpcError(line.error));
    } else {
      p.resolve(line.result ?? new Object());
    }
  }

  call(method: string, params: Object): Promise<Object> {
    const id = this.nextId++;
    return new Promise<Object>((resolve, reject) => {
      this.pending.set(id, new Pending(resolve, reject));
      this.send(0, DATA, utf8(JSON.stringify(new RpcRequest(id, method, params)) + '\n'));
    });
  }

  send(ch: number, kind: number, payload: Uint8Array): void {
    this.out(concat([u32le(ch), new Uint8Array([kind]), u32le(payload.length), payload]));
  }

  /// Start delivering a channel's frames (including any that came before its reply).
  openStream(ch: number, sink: StreamSink): void {
    this.streams.set(ch, sink);
    const early = this.early.get(ch);
    this.early.delete(ch);
    if (early !== undefined) {
      for (const f of early) {
        this.deliver(ch, sink, f);
      }
    }
  }

  /// Tell the core we consumed `n` bytes of a stream.
  credit(ch: number, n: number): void {
    if (n > 0) {
      this.send(ch, CREDIT, u32le(n));
    }
  }

  closeStream(ch: number): void {
    if (this.streams.delete(ch)) {
      this.send(ch, CLOSE, new Uint8Array(0));
    }
    this.early.delete(ch);
  }

  /// The connection is gone: fail calls, end streams.
  fail(why: string): void {
    this.pending.forEach((p: Pending) => p.reject(new Error(why)));
    this.pending.clear();
    this.streams.forEach((s: StreamSink) => s.closed(why));
    this.streams.clear();
    const w = this.helloWaiter;
    this.helloWaiter = null;
    if (w !== null) {
      w.reject(new Error(why));
    }
  }
}

/// JSON lines on a stream channel (events.subscribe); frame boundaries can fall anywhere.
export class LineSplitter {
  private q: ByteQueue = new ByteQueue();

  push(b: Uint8Array, each: (line: string) => void): void {
    this.q.push(b);
    let nl = this.q.indexOf(NL);
    while (nl >= 0) {
      const line = this.q.take(nl + 1).slice(0, nl);
      if (line.length > 0) {
        each(fromUtf8(line));
      }
      nl = this.q.indexOf(NL);
    }
  }
}
