import { PromptInfo } from './Prompt';

// Terminals and agents from events.subscribe, grouped the way every Herdr
// client shows them (.work/herdr-app/docs/labels.md): blocked/error needs
// you, working, done (turn > herdr.seen), idle; terminals that never
// reported an OSC 7501 status are plain shells.

export interface StatusRec {
  state?: string;
  app?: string;
  msg?: string;
  title?: string;
  kind?: string | null;
  progress?: number | null;
}

/// What runs in front in a terminal (the agent, not the shell that started it).
export interface FgRec {
  pid?: number | null;
  name?: string;
  cwd?: string | null;
}

export interface TermRec {
  foreground?: FgRec | null;
  address?: string;
  name?: string | null;
  title?: string;
  cwd?: string;
  state?: string;
  status?: StatusRec | null;
  status_ms?: number | null;
  turn?: number;
  labels?: Record<string, string>;
  cols?: number;
  rows?: number;
  argv?: string[];
  parent?: string | null;
  created_ms?: number;
}

/// `data` of the events that patch one field (status, turn, title, labels, size).
export interface PatchData {
  pid?: number | null;
  cwd?: string | null;
  status?: StatusRec | null;
  status_ms?: number | null;
  turn?: number;
  title?: string;
  labels?: Record<string, string>;
  record?: TermRec;
  cols?: number;
  rows?: number;
}

export interface EventParams {
  seq?: number;
  type?: string;
  terminal?: string;
  data?: Object;
  terminals?: TermRec[];
}

export interface EventLine {
  method?: string;
  params?: EventParams;
}

export interface SubscribeResult {
  channel?: number | null;
  seq?: number;
  terminals?: TermRec[];
}

export enum Group {
  NeedsYou = 0,
  Working = 1,
  Done = 2,
  Idle = 3,
  Shell = 4
}

export const GROUP_TITLES: string[] = ['需要你处理', '工作中', '已完成', '空闲', '其他终端'];

export class Agent {
  address: string = '';
  name: string = '';
  title: string = '';
  cwd: string = '';
  /// The foreground process and its folder (where an agent really works).
  fgPid: number = 0;
  fgCwd: string = '';
  live: boolean = true;
  state: string = '';
  app: string = '';
  msg: string = '';
  kind: string = '';
  progress: number = -1;
  statusMs: number = 0;
  turn: number = 0;
  seen: number = -1;
  pinned: boolean = false;
  cols: number = 80;
  rows: number = 24;
  argv0: string = '';
  /// What it asks while blocked, read off its screen; null until read (or not blocked).
  prompt: PromptInfo | null = null;
  /// The statusMs the prompt was read for (a new block reads again).
  promptFor: number = -1;

  static from(r: TermRec): Agent {
    const a = new Agent();
    a.address = r.address ?? '';
    a.name = r.name ?? '';
    a.title = r.title ?? '';
    a.cwd = r.cwd ?? '';
    a.fgPid = r.foreground?.pid ?? 0;
    a.fgCwd = r.foreground?.cwd ?? '';
    a.live = (r.state ?? 'live') === 'live';
    a.turn = r.turn ?? 0;
    a.cols = r.cols ?? 80;
    a.rows = r.rows ?? 24;
    a.argv0 = r.argv && r.argv.length > 0 ? r.argv[0] : '';
    a.setStatus(r.status ?? null, r.status_ms ?? null);
    a.setLabels(r.labels);
    return a;
  }

  setStatus(s: StatusRec | null, ms: number | null): void {
    if ((s?.state ?? '') !== 'blocked') {
      this.prompt = null;
      this.promptFor = -1;
    }
    this.state = s?.state ?? '';
    this.app = s?.app ?? this.app;
    this.msg = s?.msg ?? s?.title ?? '';
    this.kind = s?.kind ?? '';
    this.progress = s?.progress ?? -1;
    this.statusMs = ms ?? Date.now();
  }

  setLabels(l: Record<string, string> | undefined): void {
    if (l === undefined) {
      return;
    }
    const seen = l['herdr.seen'];
    this.seen = seen !== undefined && seen !== '' ? Number.parseInt(seen) : -1;
    const pin = l['herdr.pinned'];
    this.pinned = pin !== undefined && pin !== '';
  }

  group(): Group {
    if (!this.live || this.state === '') {
      return Group.Shell;
    }
    if (this.state === 'blocked' || this.state === 'error') {
      return Group.NeedsYou;
    }
    if (this.state === 'working') {
      return Group.Working;
    }
    return this.seen >= 0 && this.turn > this.seen ? Group.Done : Group.Idle;
  }

  label(): string {
    if (this.app) {
      return this.app;
    }
    if (this.name) {
      return this.name;
    }
    if (this.title) {
      return this.title;
    }
    const base = this.argv0.split('/').pop();
    return base ? base : this.address;
  }

  /// Where it works: the foreground process's folder, else the shell's.
  dir(): string {
    return this.fgCwd ? this.fgCwd : this.cwd;
  }

  folder(): string {
    const parts = this.dir().split('/').filter((s: string) => s.length > 0);
    return parts.length > 0 ? parts[parts.length - 1] : '';
  }

  detail(): string {
    if (this.state === 'blocked') {
      if (this.msg) {
        return this.msg;
      }
      if (this.kind === 'permission') {
        return '要你批准权限';
      }
      if (this.kind === 'question') {
        return '在问你问题';
      }
      if (this.kind === 'auth') {
        return '要你登录';
      }
      return '在等你';
    }
    if (this.state === 'error') {
      return this.msg ? this.msg : '出错了';
    }
    if (this.state === 'working') {
      const p = this.progress >= 0 ? ` ${this.progress}%` : '';
      return (this.msg ? this.msg : '工作中') + p;
    }
    if (this.state === '') {
      return this.title ? this.title : this.dir();
    }
    return this.msg ? this.msg : this.state;
  }
}

export class AgentStore {
  private map: Map<string, Agent> = new Map();
  private seq: number = 0;

  get(address: string): Agent | undefined {
    return this.map.get(address);
  }

  snapshot(seq: number, terminals: TermRec[]): void {
    this.seq = seq;
    this.map.clear();
    for (const r of terminals) {
      const a = Agent.from(r);
      if (a.address && a.live) {
        this.map.set(a.address, a);
      }
    }
  }

  /// One line from the events channel; true when something visible changed.
  line(l: EventLine): boolean {
    const p = l.params;
    if (p === undefined) {
      return false;
    }
    if (l.method === 'missed') {
      this.snapshot(p.seq ?? this.seq, p.terminals ?? []);
      return true;
    }
    if (l.method !== 'event') {
      return false;
    }
    if (p.seq !== undefined) {
      if (p.seq <= this.seq) {
        return false;
      }
      this.seq = p.seq;
    }
    const address = p.terminal ?? '';
    const kind = p.type ?? '';
    if (kind === 'created' || kind === 'restarted') {
      const a = Agent.from(p.data as TermRec);
      if (a.address) {
        this.map.set(a.address, a);
      }
      return true;
    }
    if (kind === 'removed' || kind === 'finished') {
      return this.map.delete(address);
    }
    const a = this.map.get(address);
    if (a === undefined) {
      return false;
    }
    const d = (p.data ?? new Object()) as PatchData;
    if (kind === 'status') {
      a.setStatus(d.status ?? null, d.status_ms ?? null);
    } else if (kind === 'turn') {
      a.turn = d.turn ?? a.turn;
    } else if (kind === 'title') {
      a.title = d.title ?? a.title;
    } else if (kind === 'labels') {
      a.setLabels(d.labels ?? d.record?.labels);
    } else if (kind === 'foreground') {
      a.fgPid = d.pid ?? 0;
      a.fgCwd = d.cwd ?? '';
    } else if (kind === 'size') {
      a.cols = d.cols ?? a.cols;
      a.rows = d.rows ?? a.rows;
    } else {
      return false;
    }
    return true;
  }

  /// Needs you first, then working, done, idle, shells; pinned first within a group, newest first.
  list(): Agent[] {
    const all: Agent[] = [];
    this.map.forEach((a: Agent) => all.push(a));
    all.sort((x: Agent, y: Agent) => {
      if (x.group() !== y.group()) {
        return (x.group() as number) - (y.group() as number);
      }
      if (x.pinned !== y.pinned) {
        return x.pinned ? -1 : 1;
      }
      return y.statusMs - x.statusMs;
    });
    return all;
  }
}
