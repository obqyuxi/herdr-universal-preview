// The conversation behind a Claude Code / Codex terminal, read from the
// agent's own session log on the Mac (bridge/src/transcript.rs): what you
// asked, what it answered, and each tool call as one line you can open
// (command, diff, output). Polled while it's on screen; the terminal stays
// attached underneath, so switching back is instant.
import { Session } from '../core/Session';
import { ChatItem, ControlMsg } from '../core/Secure';

class Part {
  code: boolean;
  text: string;

  constructor(code: boolean, text: string) {
    this.code = code;
    this.text = text;
  }
}

class ChatRow {
  key: string = '';
  k: string = '';
  text: string = '';
  name: string = '';
  detail: string = '';
  result: string = '';
  done: boolean = false;
  err: boolean = false;
  parts: Part[] = [];
  callId: string = '';
  /// Byte offset of its log line (where to read earlier parts from).
  off: number = 0;
  /// Local web addresses it mentions (opened in the web preview).
  urls: string[] = [];

  copy(): ChatRow {
    const r = new ChatRow();
    r.key = this.key;
    r.k = this.k;
    r.text = this.text;
    r.name = this.name;
    r.detail = this.detail;
    r.result = this.result;
    r.done = this.done;
    r.err = this.err;
    r.parts = this.parts;
    r.callId = this.callId;
    r.urls = this.urls;
    r.off = this.off;
    return r;
  }
}

const LABEL: Record<string, string> = {
  'Bash': '运行', 'exec': '运行', 'exec_command': '运行', 'shell': '运行', 'local_shell': '运行',
  'Edit': '编辑', 'MultiEdit': '编辑', 'apply_patch': '改文件', 'Write': '写入', 'NotebookEdit': '编辑',
  'Read': '读取', 'Grep': '搜索', 'Glob': '找文件', 'WebFetch': '打开网页', 'WebSearch': '搜网页',
  'Task': '子任务', 'Agent': '子任务', 'TodoWrite': '待办', 'send_message': '发消息', 'wait_agent': '等子任务'
};
const FILE_TOOLS = ['Edit', 'MultiEdit', 'Write', 'Read', 'NotebookEdit'];
const POLL_MS = 2000;
const IDLE_POLL_MS = 5000;
/// Rows kept while following new output (loading earlier parts may go past it).
const MAX_ROWS = 600;
/// A run of tool calls longer than this folds; the last few stay in view.
const FOLD_OVER = 3;
const KEEP_LAST = 2;

/// Plain text with fenced code blocks pulled out; light markdown cleanup.
function splitParts(text: string): Part[] {
  const out: Part[] = [];
  const chunks = text.split('```');
  for (let i = 0; i < chunks.length; i++) {
    let c = chunks[i];
    if (i % 2 === 1) {
      // Drop the language tag on the fence line.
      const nl = c.indexOf('\n');
      c = nl >= 0 && !c.substring(0, nl).includes(' ') ? c.substring(nl + 1) : c;
      if (c.trim()) {
        out.push(new Part(true, c.replace(/\n$/, '')));
      }
    } else {
      const t = c.replace(/\*\*(.+?)\*\*/g, '$1').replace(/^#{1,6}\s+/gm, '').replace(/`([^`\n]+)`/g, '$1').trim();
      if (t) {
        out.push(new Part(false, t));
      }
    }
  }
  return out;
}

function lines(s: string, max: number): string[] {
  const l = s.split('\n');
  return l.length > max ? l.slice(0, max).concat([`… 还有 ${l.length - max} 行`]) : l;
}

@Component
export struct ChatView {
  @Prop @Watch('onActive') active: boolean = false;
  @Prop app: string = '';
  @Prop cwd: string = '';
  @Prop pid: number = 0;
  @Prop working: boolean = false;
  /// Bumped after you send something: look again soon.
  @Prop @Watch('onNudge') nudge: number = 0;
  @State rows: ChatRow[] = [];
  @State openKeys: string[] = [];
  @State openGroups: string[] = [];
  @State error: string = '';
  @State loading: boolean = true;
  @State truncated: boolean = false;
  @State loadingOlder: boolean = false;
  /// Byte offset where what's shown begins (0: the session's start).
  private earliest: number = 0;
  onOpenFile: (path: string) => void = (path: string) => {};
  private scroller: Scroller = new Scroller();
  private file: string = '';
  private readAt: number = 0;
  private timer: number = -1;
  private busy: boolean = false;
  private seq: number = 0;
  private byCall: Map<string, ChatRow> = new Map<string, ChatRow>();

  aboutToAppear(): void {
    this.onActive();
  }

  aboutToDisappear(): void {
    this.stop();
  }

  onActive(): void {
    if (this.active) {
      this.poll();
    } else {
      this.stop();
    }
  }

  onNudge(): void {
    if (this.active && !this.busy) {
      this.schedule(500);
    }
  }

  private stop(): void {
    if (this.timer >= 0) {
      clearTimeout(this.timer);
      this.timer = -1;
    }
  }

  private schedule(ms: number): void {
    this.stop();
    if (this.active) {
      this.timer = setTimeout(() => this.poll(), ms);
    }
  }

  private poll(): void {
    if (this.busy || !this.cwd) {
      return;
    }
    this.busy = true;
    Session.shared.transcript(this.app, this.cwd, this.pid, this.file, this.readAt, 0).then((m: ControlMsg) => {
      this.busy = false;
      this.loading = false;
      this.error = '';
      this.apply(m);
      this.schedule(POLL_MS);
    }).catch((e: Error) => {
      this.busy = false;
      this.loading = false;
      this.error = e.message;
      this.schedule(IDLE_POLL_MS);
    });
  }

  private apply(m: ControlMsg): void {
    const items = m.items ?? [];
    const atEnd = this.rows.length === 0 || this.scroller.isAtEnd();
    let rows = m.reset ? [] : this.rows.slice();
    if (m.reset) {
      this.byCall.clear();
      this.openKeys = [];
      this.openGroups = [];
      this.earliest = m.start ?? 0;
      this.truncated = this.earliest > 0;
    }
    this.file = m.file ?? '';
    this.readAt = m.offset ?? 0;
    if (items.length === 0 && !m.reset) {
      return;
    }
    for (const it of items) {
      this.add(rows, it);
    }
    if (rows.length > MAX_ROWS) {
      rows = rows.slice(rows.length - MAX_ROWS);
      // Start at a whole log line, so loading earlier parts doesn't repeat any.
      const first = rows[0].off;
      while (rows.length > 0 && rows[0].off === first) {
        rows.shift();
      }
      this.earliest = first;
      this.truncated = true;
    }
    this.rows = rows;
    if (atEnd) {
      setTimeout((): void => this.scroller.scrollEdge(Edge.Bottom), 50);
    }
  }

  /// The part of the session before what's shown, put on top; the view stays where it was.
  private loadOlder(): void {
    if (this.loadingOlder || this.earliest <= 0 || !this.file) {
      return;
    }
    this.loadingOlder = true;
    const firstKey = this.rows.length > 0 ? this.rows[0].key : '';
    Session.shared.transcript(this.app, this.cwd, this.pid, this.file, this.readAt, this.earliest).then((m: ControlMsg) => {
      this.loadingOlder = false;
      if (!m.older || m.file !== this.file) {
        return;
      }
      const older: ChatRow[] = [];
      for (const it of m.items ?? []) {
        this.add(older, it);
      }
      for (let i = 0; i < older.length; i++) {
        // Their results were in the part already left out: don't spin forever.
        if (older[i].k === 'tool' && !older[i].done) {
          const r = older[i].copy();
          r.done = true;
          older[i] = r;
        }
      }
      this.earliest = m.start ?? 0;
      this.truncated = this.earliest > 0;
      this.rows = older.concat(this.rows);
      const idx = this.view().findIndex((r: ChatRow) => r.key === firstKey);
      if (idx >= 0) {
        setTimeout((): void => this.scroller.scrollToIndex(idx + (this.truncated ? 1 : 0)), 50);
      }
    }).catch((e: Error) => {
      this.loadingOlder = false;
      this.error = e.message;
    });
  }

  private add(rows: ChatRow[], it: ChatItem): void {
    if (it.k === 'result') {
      const call = this.byCall.get(it.id ?? '');
      if (call === undefined) {
        return;
      }
      // A new object, so the row's key (and so its look) changes.
      const r = call.copy();
      r.result = it.text;
      r.err = it.err ?? false;
      r.done = true;
      r.key = `${call.key.split('|')[0]}|${r.err ? 'e' : 'd'}`;
      const i = rows.indexOf(call);
      if (i >= 0) {
        rows[i] = r;
      }
      this.byCall.set(it.id ?? '', r);
      return;
    }
    const r = new ChatRow();
    r.key = `${this.seq++}|p`;
    r.k = it.k;
    r.text = it.text;
    r.name = it.name ?? '';
    r.detail = it.detail ?? '';
    r.callId = it.id ?? '';
    r.off = it.o ?? 0;
    if (it.k !== 'tool') {
      r.parts = splitParts(it.text);
      if (r.parts.length === 0) {
        return;
      }
      const urls = it.text.match(/https?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0)(?::\d+)?[^\s'"<>)\]`*]*/g) ?? [];
      r.urls = Array.from(new Set(urls.map((u: string) => u.replace(/[.,;:]+$/, '')))).slice(0, 3);
    } else if (r.callId) {
      this.byCall.set(r.callId, r);
    }
    rows.push(r);
  }

  /// Rows as shown: long runs of tool calls fold into one "N 个操作" row.
  private view(): ChatRow[] {
    const out: ChatRow[] = [];
    let i = 0;
    while (i < this.rows.length) {
      if (this.rows[i].k !== 'tool') {
        out.push(this.rows[i]);
        i++;
        continue;
      }
      let j = i;
      while (j < this.rows.length && this.rows[j].k === 'tool') {
        j++;
      }
      const run = this.rows.slice(i, j);
      const id = run[0].key.split('|')[0];
      if (run.length > FOLD_OVER && !this.openGroups.includes(id)) {
        const g = new ChatRow();
        g.k = 'group';
        g.key = `g${id}|${run.length - KEEP_LAST}`;
        g.text = `已执行 ${run.length - KEEP_LAST} 个操作`;
        g.callId = id;
        g.err = run.slice(0, run.length - KEEP_LAST).some((r: ChatRow) => r.err);
        out.push(g);
        for (const r of run.slice(run.length - KEEP_LAST)) {
          out.push(r);
        }
      } else {
        for (const r of run) {
          out.push(r);
        }
      }
      i = j;
    }
    return out;
  }

  private toggle(r: ChatRow): void {
    const base = r.key.split('|')[0];
    this.openKeys = this.openKeys.includes(base) ? this.openKeys.filter((k: string) => k !== base) : this.openKeys.concat([base]);
  }

  private isOpen(r: ChatRow): boolean {
    return this.openKeys.includes(r.key.split('|')[0]);
  }

  @Builder
  textParts(parts: Part[], color: string) {
    Column({ space: 6 }) {
      ForEach(parts, (p: Part) => {
        if (p.code) {
          Text(p.text).fontSize(12).fontFamily('monospace').fontColor('#C9CDD2')
            .padding(8).borderRadius(6).backgroundColor('#0B0C0E').width('100%')
            .copyOption(CopyOptions.LocalDevice)
        } else {
          Text(p.text).fontSize(15).lineHeight(22).fontColor(color).copyOption(CopyOptions.LocalDevice)
        }
      }, (p: Part, i: number) => `${i}`)
    }.alignItems(HorizontalAlign.Start)
  }

  @Builder
  toolRow(r: ChatRow) {
    Column({ space: 6 }) {
      Row({ space: 8 }) {
        if (!r.done) {
          LoadingProgress().width(14).height(14).color('#8A8F98')
        } else {
          Text(r.err ? '✕' : '✓').fontSize(12).fontColor(r.err ? '#FF5D5D' : '#36C275').width(14).textAlign(TextAlign.Center)
        }
        Text(LABEL[r.name] ?? r.name).fontSize(12).fontColor('#9AA0A6')
        // A Bash call's description is prose; commands and paths are code.
        Text(r.text).fontSize(r.name === 'Bash' && r.detail ? 13 : 12)
          .fontFamily(r.name === 'Bash' && r.detail ? 'HarmonyOS Sans' : 'monospace').fontColor('#C9CDD2').layoutWeight(1)
          .maxLines(this.isOpen(r) ? 6 : 1).textOverflow({ overflow: TextOverflow.Ellipsis })
      }.width('100%')
      if (this.isOpen(r)) {
        if (FILE_TOOLS.includes(r.name) && r.text.startsWith('/')) {
          Text('打开文件 ›').fontSize(12).fontColor('#4C9AFF').onClick(() => this.onOpenFile(r.text))
        }
        if (r.detail) {
          Column() {
            ForEach(lines(r.detail, 60), (l: string, i: number) => {
              Text(l).fontSize(11).fontFamily('monospace').width('100%')
                .fontColor(l.startsWith('+ ') ? '#36C275' : l.startsWith('- ') ? '#FF7B7B' : '#9AA0A6')
            }, (l: string, i: number) => `${i}`)
          }.padding(8).borderRadius(6).backgroundColor('#0B0C0E').width('100%')
        }
        if (r.result) {
          Text(lines(r.result, 40).join('\n')).fontSize(11).fontFamily('monospace')
            .fontColor(r.err ? '#FF9A9A' : '#8A8F98').padding(8).borderRadius(6).backgroundColor('#0B0C0E').width('100%')
            .copyOption(CopyOptions.LocalDevice)
        }
      }
    }
    .width('100%').padding({ left: 10, right: 10, top: 7, bottom: 7 }).borderRadius(8)
    .backgroundColor(this.isOpen(r) ? '#17191D' : Color.Transparent)
    .onClick(() => this.toggle(r))
  }

  build() {
    Stack({ alignContent: Alignment.Center }) {
      List({ scroller: this.scroller, space: 6 }) {
        if (this.truncated) {
          ListItem() {
            Row({ space: 8 }) {
              if (this.loadingOlder) {
                LoadingProgress().width(16).height(16).color('#9AA0A6')
              }
              Text(this.loadingOlder ? '正在加载…' : '加载更早的内容').fontSize(13).fontColor('#4C9AFF')
            }
            .padding({ left: 14, right: 14, top: 7, bottom: 7 }).borderRadius(16).backgroundColor('#1A2638')
            .onClick(() => this.loadOlder())
          }.width('100%').padding(6).align(Alignment.Center)
        }
        ForEach(this.view(), (r: ChatRow) => {
          ListItem() {
            if (r.k === 'group') {
              Row({ space: 8 }) {
                Text('›').fontSize(14).fontColor('#8A8F98').width(14).textAlign(TextAlign.Center)
                Text(r.text).fontSize(12).fontColor('#9AA0A6')
                if (r.err) {
                  Text('有失败').fontSize(11).fontColor('#FF7B7B')
                }
              }
              .width('100%').padding({ left: 10, right: 10, top: 7, bottom: 7 })
              .onClick(() => {
                this.openGroups = this.openGroups.concat([r.callId]);
              })
            } else if (r.k === 'user') {
              Row() {
                Blank()
                Column() {
                  this.textParts(r.parts, '#F2F4F7')
                }
                .padding({ left: 12, right: 12, top: 8, bottom: 8 }).borderRadius(14)
                .backgroundColor('#24426E').constraintSize({ maxWidth: '85%' })
              }.width('100%').padding({ top: 6 })
            } else if (r.k === 'assistant') {
              Column({ space: 6 }) {
                this.textParts(r.parts, '#E6E6E6')
                ForEach(r.urls, (u: string) => {
                  Text(`预览 ${u.replace(/^https?:\/\//, '')} ›`).fontSize(13).fontColor('#4C9AFF')
                    .padding({ left: 10, right: 10, top: 5, bottom: 5 }).borderRadius(12).backgroundColor('#1A2638')
                    .onClick(() => this.onOpenFile(u))
                }, (u: string) => u)
              }.alignItems(HorizontalAlign.Start)
            } else {
              this.toolRow(r)
            }
          }.padding({ left: r.k === 'tool' || r.k === 'group' ? 4 : 14, right: r.k === 'tool' || r.k === 'group' ? 4 : 14 })
        }, (r: ChatRow) => `${r.key}|${this.isOpen(r) ? 'o' : 'c'}`)
        if (this.working) {
          ListItem() {
            Row({ space: 8 }) {
              LoadingProgress().width(16).height(16).color('#9AA0A6')
              Text('正在工作…').fontSize(13).fontColor('#9AA0A6')
            }.padding({ left: 14, top: 6, bottom: 6 })
          }
        }
      }
      .width('100%').height('100%').padding({ top: 8, bottom: 8 }).scrollBar(BarState.Auto)
      .edgeEffect(EdgeEffect.Spring)

      if (this.loading) {
        LoadingProgress().width(32).height(32).color('#9AA0A6')
      } else if (this.rows.length === 0) {
        Text(this.error ? `${this.error}\n\n切到「终端」可以直接看屏幕` : '这个会话还没有内容')
          .fontSize(14).fontColor('#9AA0A6').textAlign(TextAlign.Center).padding(24)
      }
    }
    .width('100%').height('100%').backgroundColor('#101114')
  }
}
