// The app's one connection: pairing, connecting (trying each address of the
// Mac), the core's events, reconnecting when the app returns to the
// foreground, push registration, and the calls the pages make.
import { deviceInfo } from '@kit.BasicServicesKit';
import { b64urlDecode, b64urlEncode, fromUtf8 } from './Bytes';
import { ControlMsg, SecureSocket } from './Secure';
import { HcClient, HelloResult, LineSplitter, RESIZE, StreamSink } from './Hc';
import { Agent, AgentStore, EventLine, SubscribeResult } from './Agents';
import { PairLink, wsUrl } from './Link';
import { keysFor, parsePrompt, PromptInfo } from './Prompt';
import { cardValues, Cards } from './Cards';
import { util } from '@kit.ArkTS';
import { forgetMachine, loadMachine, MachineData, saveMachine } from './Vault';

export enum Conn {
  Idle = 0,
  Connecting = 1,
  Ready = 2,
  Failed = 3
}

export const CONN_TEXT: string[] = ['未连接', '连接中…', '已连接', '连接失败'];

// Generous: through a Tailscale relay (DERP) one round trip can take a second or more.
const CONNECT_TIMEOUT_MS = 12000;

class EmptyParams {
}

class TerminalParam {
  terminal: string;

  constructor(terminal: string) {
    this.terminal = terminal;
  }
}

class DisplayParam {
  bg: string = '#101114';
  fg: string = '#e6e6e6';
  cursor: string = '#e6e6e6';
}

class AttachParams {
  terminal: string;
  cols: number;
  rows: number;
  observe: boolean;
  history: number = 2000;
  resize_notifications: boolean = true;
  display: DisplayParam = new DisplayParam();

  constructor(terminal: string, cols: number, rows: number, observe: boolean) {
    this.terminal = terminal;
    this.cols = cols;
    this.rows = rows;
    this.observe = observe;
  }
}

class SendParams {
  terminal: string;
  text: string;
  force: boolean;

  constructor(terminal: string, text: string, force: boolean) {
    this.terminal = terminal;
    this.text = text;
    this.force = force;
  }
}

class KeysParams {
  terminal: string;
  keys: string[];

  constructor(terminal: string, keys: string[]) {
    this.terminal = terminal;
    this.keys = keys;
  }
}

class ReadParams {
  terminal: string;
  lines: number;

  constructor(terminal: string, lines: number) {
    this.terminal = terminal;
    this.lines = lines;
  }
}

class PutParams {
  terminal: string;
  name: string;
  data: string;

  constructor(terminal: string, name: string, data: string) {
    this.terminal = terminal;
    this.name = name;
    this.data = data;
  }
}

class ExecParams {
  line: string;
  cwd: string | null;
  timeout: number;

  constructor(line: string, cwd: string | null, timeout: number) {
    this.line = line;
    this.cwd = cwd;
    this.timeout = timeout;
  }
}

class FileReadParams {
  path: string;
  cwd: string | null;
  limit: number;

  constructor(path: string, cwd: string | null, limit: number) {
    this.path = path;
    this.cwd = cwd;
    this.limit = limit;
  }
}

class RunParams {
  line: string;
  cwd: string;
  env: Record<string, string>;
  labels: Record<string, string>;

  constructor(line: string, cwd: string, env: Record<string, string>) {
    this.line = line;
    this.cwd = cwd;
    this.env = env;
    this.labels = { 'herdr.seen': '0' };
  }
}

export interface ReadResult {
  text?: string;
}

export interface PutResult {
  path?: string;
  bytes?: number;
}

export interface ExecResult {
  code?: number | null;
  stdout?: string;
  stderr?: string;
  timed_out?: boolean;
  truncated?: boolean;
}

export interface FileReadResult {
  text?: string;
  lines?: number;
  last?: number;
}

export interface RunResult {
  address?: string;
}

/// machine.exec output, decoded.
export class ExecOutput {
  code: number = -1;
  stdout: Uint8Array = new Uint8Array(0);
  stderr: string = '';
  timedOut: boolean = false;
}

class LabelParams {
  terminal: string;
  labels: Record<string, string>;

  constructor(terminal: string, labels: Record<string, string>) {
    this.terminal = terminal;
    this.labels = labels;
  }
}

/// Router params for pages/Terminal.
export class TermParams {
  address: string = '';
}

export interface AttachResult {
  channel?: number;
}

const SUPERSEDED = 'superseded';

export class Session {
  static readonly shared: Session = new Session();

  machine: MachineData | null = null;
  loaded: boolean = false;
  conn: Conn = Conn.Idle;
  error: string = '';
  coreVersion: string = '';
  agents: AgentStore = new AgentStore();
  /// A terminal a tapped notification asked to open.
  pendingOpen: string = '';
  /// A ddup://pair link the app was opened with (scanned by the system camera).
  pendingPair: string = '';

  private sock: SecureSocket | null = null;
  private hc: HcClient | null = null;
  private foreground: boolean = true;
  private pushToken: string = '';
  private pushWaiter: ((m: ControlMsg) => void) | null = null;
  /// Requests waiting for their reply, by id.
  private waiters: Map<number, (m: ControlMsg) => void> = new Map<number, (m: ControlMsg) => void>();
  private nextId: number = 1;
  /// The address that answered (for web previews, which connect to it directly).
  host: string = '';
  private retryTimer: number = -1;
  private retryDelay: number = 2000;
  /// One dial at a time (foreground, retry timer and the page can all ask at once).
  private dialing: boolean = false;
  /// Pairing with a new Mac: the saved one must not reconnect meanwhile (a pairing link
  /// can start the app, which also reconnects on launch).
  private pairing: boolean = false;
  /// Only the newest dial may become the session.
  private dialGen: number = 0;
  private listeners: Array<() => void> = [];

  subscribe(fn: () => void): void {
    this.listeners.push(fn);
  }

  unsubscribe(fn: () => void): void {
    this.listeners = this.listeners.filter((f: () => void) => f !== fn);
  }

  /// Something arrived from outside (a link, a tapped notification): let the pages look.
  poke(): void {
    this.changed();
  }

  private changed(): void {
    for (const f of this.listeners) {
      f();
    }
    if (this.conn === Conn.Ready) {
      Cards.soon(() => this.agents.list());
    }
  }

  isForeground(): boolean {
    return this.foreground;
  }

  async start(): Promise<void> {
    if (!this.loaded) {
      this.machine = await loadMachine();
      this.loaded = true;
      this.changed();
    }
    if (this.machine !== null && this.conn !== Conn.Ready && this.conn !== Conn.Connecting) {
      await this.connect();
    }
  }

  /// Pair with a scanned or pasted link, save the credentials, stay connected.
  async pair(link: PairLink): Promise<void> {
    this.pairing = true;
    try {
      await this.pairOnce(link);
    } finally {
      this.pairing = false;
    }
  }

  private async pairOnce(link: PairLink): Promise<void> {
    const first: ControlMsg = { t: 'pair', code: link.code, device: deviceInfo.marketName || deviceInfo.productModel || 'HarmonyOS' };
    const reply = await this.dial(link.hosts, link.port, link.key, first);
    if (reply.t !== 'paired' || !reply.device_id || !reply.token) {
      throw new Error('桥接服务没有返回设备令牌');
    }
    const m: MachineData = {
      name: reply.name ?? link.name,
      hosts: link.hosts,
      port: link.port,
      key: b64urlEncode(link.key),
      deviceId: reply.device_id,
      token: reply.token
    };
    await saveMachine(m);
    this.machine = m;
    await this.afterAuth();
  }

  async connect(): Promise<void> {
    const m = this.machine;
    if (m === null || this.dialing || this.pairing) {
      return;
    }
    this.dialing = true;
    try {
      await this.connectOnce(m);
    } finally {
      this.dialing = false;
    }
  }

  private async connectOnce(m: MachineData): Promise<void> {
    this.clearRetry();
    try {
      const first: ControlMsg = { t: 'auth', device_id: m.deviceId, token: m.token };
      await this.dial(m.hosts, m.port, b64urlDecode(m.key), first);
      await this.afterAuth();
    } catch (e) {
      const msg = (e as Error).message;
      if (msg === SUPERSEDED) {
        return;
      }
      if (msg === 'unauthorized') {
        this.error = '这台手机已在 Mac 上被移除，请重新配对';
        this.conn = Conn.Failed;
        this.changed();
        return;
      }
      this.error = msg;
      this.conn = Conn.Failed;
      this.changed();
      this.scheduleRetry();
    }
  }

  /// Dial every address at once (home Wi-Fi, Tailscale, ...); the first that answers wins
  /// and the rest are closed. Away from home the LAN address simply never answers.
  private async dial(hosts: string[], port: number, key: Uint8Array, first: ControlMsg): Promise<ControlMsg> {
    this.drop();
    this.conn = Conn.Connecting;
    this.error = '';
    this.changed();
    if (hosts.length === 0) {
      throw new Error('没有可用地址');
    }
    const gen = ++this.dialGen;
    return new Promise<ControlMsg>((resolve, reject) => {
      let pending = hosts.length;
      let won = false;
      let last: Error = new Error('连接失败');
      for (const host of hosts) {
        const sock = new SecureSocket();
        sock.open(wsUrl(host, port), key, first, CONNECT_TIMEOUT_MS).then((reply: ControlMsg) => {
          if (won) {
            sock.close();
            return;
          }
          won = true;
          if (gen !== this.dialGen) {
            sock.close();
            reject(new Error(SUPERSEDED));
            return;
          }
          this.sock = sock;
          this.host = host;
          const m = this.machine;
          if (m !== null && m.hosts[0] !== host) {
            // Try the address that worked first next time.
            m.hosts = [host].concat(m.hosts.filter((h: string) => h !== host));
            saveMachine(m).catch((): void => {});
          }
          resolve(reply);
        }).catch((e: Error) => {
          // A refusal from the bridge says more than another address timing out.
          if (e.message.startsWith('pair_') || e.message === 'unauthorized' || last.message === '连接失败' || last.message === '连接超时') {
            last = e;
          }
          pending--;
          if (pending === 0 && !won) {
            reject(last);
          }
        });
      }
    });
  }

  private async afterAuth(): Promise<void> {
    const sock = this.sock;
    if (sock === null) {
      return;
    }
    const hc = new HcClient((b: Uint8Array) => sock.sendHc(b));
    this.hc = hc;
    sock.onHc = (b: Uint8Array) => hc.feed(b);
    sock.onControl = (m: ControlMsg) => this.onControl(m);
    sock.onClose = (why: string) => this.lost(why);
    const hello = (await hc.start()) as HelloResult;
    this.coreVersion = hello.version ?? '';
    const sub = (await hc.call('events.subscribe', new EmptyParams())) as SubscribeResult;
    this.agents.snapshot(sub.seq ?? 0, sub.terminals ?? []);
    this.readPrompts();
    if (sub.channel !== undefined && sub.channel !== null) {
      const ch = sub.channel;
      const lines = new LineSplitter();
      const sink: StreamSink = {
        data: (b: Uint8Array) => {
          let changed = false;
          lines.push(b, (line: string) => {
            changed = this.agents.line(JSON.parse(line) as EventLine) || changed;
          });
          hc.credit(ch, b.length);
          if (changed) {
            this.changed();
            this.readPrompts();
          }
        },
        resize: (cols: number, rows: number) => {},
        closed: (why: string) => {}
      };
      hc.openStream(ch, sink);
    }
    this.conn = Conn.Ready;
    this.retryDelay = 2000;
    this.readPrompts();
    sock.sendControl({ t: 'foreground', on: this.foreground });
    if (this.pushToken) {
      sock.sendControl({ t: 'push_token', token: this.pushToken });
    }
    this.sendForms();
    this.changed();
  }

  /// Tell the bridge which home-screen cards to refresh while the app is away.
  sendForms(): void {
    if (this.sock !== null && this.conn === Conn.Ready) {
      this.sock.sendControl({ t: 'forms', ids: Cards.ids() });
    }
  }

  private onControl(m: ControlMsg): void {
    const w = m.id !== undefined ? this.waiters.get(m.id) : undefined;
    if (w !== undefined && m.id !== undefined) {
      this.waiters.delete(m.id);
      w(m);
    } else if (m.t === 'push_result' && m.re === 'push_test' && this.pushWaiter !== null) {
      const w = this.pushWaiter;
      this.pushWaiter = null;
      w(m);
    } else if (m.t === 'hc_closed') {
      this.lost('Mac 上的 core 断开了');
    }
  }

  private lost(why: string): void {
    this.drop();
    this.conn = Conn.Failed;
    this.error = why;
    this.changed();
    this.scheduleRetry();
  }

  private drop(): void {
    const waiting = Array.from(this.waiters.values());
    this.waiters.clear();
    for (const w of waiting) {
      w({ t: 'dropped', error: '连接已断开' });
    }
    const hc = this.hc;
    this.hc = null;
    if (hc !== null) {
      hc.fail('连接已断开');
    }
    const sock = this.sock;
    this.sock = null;
    if (sock !== null) {
      sock.onClose = (why: string) => {};
      sock.close();
    }
  }

  private scheduleRetry(): void {
    this.clearRetry();
    if (!this.foreground || this.machine === null) {
      return;
    }
    this.retryTimer = setTimeout(() => {
      this.retryTimer = -1;
      this.connect();
    }, this.retryDelay);
    this.retryDelay = Math.min(this.retryDelay * 2, 30000);
  }

  private clearRetry(): void {
    if (this.retryTimer >= 0) {
      clearTimeout(this.retryTimer);
      this.retryTimer = -1;
    }
  }

  /// Background: the system suspends the app soon anyway; the bridge pushes meanwhile.
  setForeground(on: boolean): void {
    this.foreground = on;
    if (!on && this.conn === Conn.Ready) {
      // Leave the cards current: the bridge takes over from here.
      Cards.show(cardValues(this.agents.list()));
    } else if (on) {
      this.sendForms();
    }
    if (this.sock !== null) {
      this.sock.sendControl({ t: 'foreground', on: on });
    }
    if (on && this.machine !== null && this.conn !== Conn.Ready && this.conn !== Conn.Connecting) {
      this.retryDelay = 2000;
      this.connect();
    } else if (!on) {
      this.clearRetry();
    }
  }

  setPushToken(token: string): void {
    this.pushToken = token;
    if (this.sock !== null && this.conn === Conn.Ready) {
      this.sock.sendControl({ t: 'push_token', token: token });
    }
  }

  hasPushToken(): boolean {
    return this.pushToken.length > 0;
  }

  testPush(): Promise<string> {
    const sock = this.sock;
    if (sock === null || this.conn !== Conn.Ready) {
      return Promise.reject(new Error('未连接'));
    }
    return new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pushWaiter = null;
        reject(new Error('桥接服务没有回应'));
      }, 20000);
      this.pushWaiter = (m: ControlMsg) => {
        clearTimeout(timer);
        if (m.ok) {
          resolve(m.detail ?? 'ok');
        } else {
          reject(new Error(m.detail ?? '推送失败'));
        }
      };
      sock.sendControl({ t: 'push_test' });
    });
  }

  /// Ask the bridge itself (not the core); the reply carries the same id.
  request(m: ControlMsg, timeoutMs: number): Promise<ControlMsg> {
    const sock = this.sock;
    if (sock === null || this.conn !== Conn.Ready) {
      return Promise.reject(new Error('未连接'));
    }
    const id = this.nextId++;
    m.id = id;
    return new Promise<ControlMsg>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters.delete(id);
        reject(new Error('桥接服务没有回应（需要更新 Mac 上的 ddup-bridge？）'));
      }, timeoutMs);
      this.waiters.set(id, (r: ControlMsg) => {
        clearTimeout(timer);
        if (r.error) {
          reject(new Error(r.error));
        } else {
          resolve(r);
        }
      });
      sock.sendControl(m);
    });
  }

  /// `before` > 0 asks for the part of the log before that byte (older messages).
  transcript(app: string, cwd: string, pid: number, file: string, offset: number, before: number): Promise<ControlMsg> {
    const m: ControlMsg = { t: 'transcript', app: app, cwd: cwd, pid: pid };
    if (file) {
      m.file = file;
      m.offset = offset;
    }
    if (before > 0) {
      m.before = before;
    }
    return this.request(m, 15000);
  }

  sites(): Promise<ControlMsg> {
    return this.request({ t: 'ports' }, 15000);
  }

  /// A one-time link to a local web server, through a forward on the bridge.
  async forward(port: number, path: string): Promise<string> {
    const r = await this.request({ t: 'forward', port: port }, 15000);
    const h = this.host.includes(':') ? `[${this.host}]` : this.host;
    const p = path.startsWith('/') ? path : '/';
    return `http://${h}:${r.listen_port}${p}${p.includes('?') ? '&' : '?'}__ddup=${r.token}`;
  }

  async forget(): Promise<void> {
    this.clearRetry();
    this.drop();
    await forgetMachine();
    this.machine = null;
    this.conn = Conn.Idle;
    this.error = '';
    this.agents = new AgentStore();
    this.changed();
  }

  private client(): HcClient {
    const hc = this.hc;
    if (hc === null || this.conn !== Conn.Ready) {
      throw new Error('未连接到 Mac');
    }
    return hc;
  }

  /// Show a terminal. With a size (phone width) the core sizes the terminal to the
  /// phone while it is attached and gives the desktop its size back afterwards;
  /// without one (cols 0) the phone only watches at the terminal's own size.
  async attach(a: Agent, cols: number, rows: number, sink: StreamSink): Promise<number> {
    const hc = this.client();
    const observe = cols <= 0 || rows <= 0;
    const params = new AttachParams(a.address, observe ? a.cols : cols, observe ? a.rows : rows, observe);
    const r = (await hc.call('terminal.attach', params)) as AttachResult;
    const ch = r.channel ?? -1;
    if (ch < 0) {
      throw new Error('core 没有返回终端通道');
    }
    const wrapped: StreamSink = {
      data: (b: Uint8Array) => {
        sink.data(b);
        hc.credit(ch, b.length);
      },
      resize: sink.resize,
      closed: sink.closed
    };
    hc.openStream(ch, wrapped);
    return ch;
  }

  /// The phone's viewport changed (keyboard, rotation): tell the core the new size.
  resize(ch: number, cols: number, rows: number): void {
    if (this.hc !== null && ch >= 0 && cols > 0 && rows > 0) {
      const p = new Uint8Array(4);
      const v = new DataView(p.buffer);
      v.setUint16(0, cols, true);
      v.setUint16(2, rows, true);
      this.hc.send(ch, RESIZE, p);
    }
  }

  detach(ch: number): void {
    if (this.hc !== null && ch >= 0) {
      this.hc.closeStream(ch);
    }
  }

  async sendText(address: string, text: string, force: boolean): Promise<void> {
    await this.client().call('terminal.send', new SendParams(address, text, force));
  }

  async keys(address: string, keys: string[]): Promise<void> {
    await this.client().call('terminal.keys', new KeysParams(address, keys));
  }

  async interrupt(address: string): Promise<void> {
    await this.client().call('terminal.interrupt', new TerminalParam(address));
  }

  /// Read what each newly blocked agent asks (its screen), once per block.
  private readPrompts(): void {
    for (const a of this.agents.list()) {
      if (a.state === 'blocked' && a.promptFor !== a.statusMs) {
        a.promptFor = a.statusMs;
        this.refreshPrompt(a.address);
      }
    }
  }

  /// Re-read a blocked agent's screen and parse its question.
  /// An agent says "blocked" a moment before it finishes drawing the question,
  /// so an empty read is tried again a little later.
  refreshPrompt(address: string, attempt?: number): void {
    const hc = this.hc;
    const n = attempt ?? 0;
    if (hc === null || this.conn !== Conn.Ready) {
      // Not connected yet: try again once it is (readPrompts runs on Ready).
      const a = this.agents.get(address);
      if (a !== undefined) {
        a.promptFor = -1;
      }
      return;
    }
    hc.call('terminal.read', new ReadParams(address, 0)).then((r: Object) => {
      const a = this.agents.get(address);
      if (a === undefined || a.state !== 'blocked') {
        return;
      }
      const p = parsePrompt((r as ReadResult).text ?? '');
      a.prompt = p.kind === 'none' ? null : p;
      this.changed();
      if (p.kind === 'none' && n < 2) {
        setTimeout(() => this.refreshPrompt(address, n + 1), n === 0 ? 600 : 1500);
      }
    }).catch((): void => {});
  }

  /// Read a blocked agent's question now (a card button checks it is still the same).
  async readPrompt(address: string): Promise<PromptInfo | null> {
    const r = (await this.client().call('terminal.read', new ReadParams(address, 0))) as ReadResult;
    const p = parsePrompt(r.text ?? '');
    const a = this.agents.get(address);
    if (a !== undefined) {
      a.prompt = p.kind === 'none' ? null : p;
      a.promptFor = a.statusMs;
    }
    return p.kind === 'none' ? null : p;
  }

  /// Wait until connected (a card button starts the app in the background).
  async ready(timeoutMs: number): Promise<void> {
    await this.start();
    const until = Date.now() + timeoutMs;
    while (this.conn !== Conn.Ready) {
      if (Date.now() > until || this.conn === Conn.Failed && this.error === '这台手机已在 Mac 上被移除，请重新配对') {
        throw new Error(this.error || '连不上 Mac');
      }
      if (this.conn !== Conn.Connecting) {
        this.connect();
      }
      await new Promise<void>((r: () => void) => setTimeout(r, 200));
    }
  }

  /// Answer a blocked agent's question: move to the option and press enter.
  async answer(address: string, option: number): Promise<void> {
    const a = this.agents.get(address);
    const p: PromptInfo | null = a?.prompt ?? null;
    if (a === undefined || p === null) {
      throw new Error('没有待回答的问题');
    }
    await this.keys(address, keysFor(p, option));
    // The screen moves on (or asks again): read it once it settles.
    a.promptFor = -1;
    setTimeout(() => this.refreshPrompt(address), 900);
  }

  /// Put a file beside a terminal (where its agent can read it); returns its path on the Mac.
  async put(address: string, name: string, bytes: Uint8Array): Promise<string> {
    const data = new util.Base64Helper().encodeToStringSync(bytes);
    const r = (await this.client().call('terminal.put', new PutParams(address, name, data))) as PutResult;
    return r.path ?? '';
  }

  /// Run a shell line on the Mac (git, ls, cat ...).
  async exec(line: string, cwd: string | null, timeout?: number): Promise<ExecOutput> {
    const r = (await this.client().call('machine.exec', new ExecParams(line, cwd, timeout ?? 30))) as ExecResult;
    const out = new ExecOutput();
    const b64 = new util.Base64Helper();
    out.code = r.code ?? -1;
    out.stdout = r.stdout ? b64.decodeSync(r.stdout) : new Uint8Array(0);
    out.stderr = r.stderr ? fromUtf8(b64.decodeSync(r.stderr)) : '';
    out.timedOut = r.timed_out ?? false;
    return out;
  }

  async readFile(path: string, cwd: string | null, limit: number): Promise<FileReadResult> {
    return (await this.client().call('file.read', new FileReadParams(path, cwd, limit))) as FileReadResult;
  }

  /// Start a program in a new terminal through the login shell (so PATH has the user's tools).
  async run(line: string, cwd: string, env: Record<string, string>): Promise<string> {
    const r = (await this.client().call('terminal.run', new RunParams(line, cwd, env))) as RunResult;
    return r.address ?? '';
  }

  /// Folders terminals run in, the most recently active first.
  recentFolders(): string[] {
    const seen: string[] = [];
    const list = this.agents.list().slice().sort((x: Agent, y: Agent) => y.statusMs - x.statusMs);
    for (const a of list) {
      if (a.dir() && !seen.includes(a.dir())) {
        seen.push(a.dir());
      }
    }
    return seen;
  }

  /// docs/labels.md: raise herdr.seen to the turn a person is looking at.
  markSeen(a: Agent): void {
    if (this.hc === null || this.conn !== Conn.Ready || (a.seen >= 0 && a.seen >= a.turn)) {
      return;
    }
    const labels: Record<string, string> = { 'herdr.seen': `${a.turn}` };
    a.seen = a.turn;
    this.hc.call('terminal.label', new LabelParams(a.address, labels)).catch(() => {});
  }
}
