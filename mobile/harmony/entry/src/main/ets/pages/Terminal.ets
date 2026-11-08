// One terminal: its live screen (xterm.js in ArkWeb), what the agent is asking
// as buttons, quick keys (long-press for the alternate key), a text box with
// attachments, and the way to its changes and files. Typing goes through
// terminal.keys / terminal.send, never through xterm (no soft-keyboard trouble).
import { promptAction, router } from '@kit.ArkUI';
import { webview } from '@kit.ArkWeb';
import { common } from '@kit.AbilityKit';
import { Conn, Session, TermParams } from '../core/Session';
import { Agent, Group } from '../core/Agents';
import { StreamSink } from '../core/Hc';
import { PromptInfo, PromptOption } from '../core/Prompt';
import { load, pickFiles, pickPhotos, takePhoto } from '../common/Attach';
import { PreviewParams } from './Preview';

class QuickKey {
  label: string;
  keys: string[];
  altLabel: string;
  altKeys: string[];

  constructor(label: string, keys: string[], altLabel: string, altKeys: string[]) {
    this.label = label;
    this.keys = keys;
    this.altLabel = altLabel;
    this.altKeys = altKeys;
  }
}

// Long-press gives the second key (Termux-style alternates).
const QUICK: QuickKey[] = [
  new QuickKey('Esc', ['esc'], 'Esc Esc', ['esc', 'esc']),
  new QuickKey('Enter', ['enter'], '⌫', ['backspace']),
  new QuickKey('↑', ['up'], 'PgUp', ['pageup']),
  new QuickKey('↓', ['down'], 'PgDn', ['pagedown']),
  new QuickKey('←', ['left'], 'Home', ['home']),
  new QuickKey('→', ['right'], 'End', ['end']),
  new QuickKey('Tab', ['tab'], '⇧Tab', ['shift+tab']),
  new QuickKey('^C', ['ctrl+c'], '^D', ['ctrl+d']),
  new QuickKey('1', ['1'], '4', ['4']),
  new QuickKey('2', ['2'], '5', ['5']),
  new QuickKey('3', ['3'], '6', ['6']),
  new QuickKey('y', ['y'], 'Y', ['Y']),
  new QuickKey('n', ['n'], 'N', ['N'])
];

class SizeMsg {
  t: string;
  cols: number;
  rows: number;
  mode: string;
  font: number;

  constructor(t: string, cols: number, rows: number, mode: string, font: number) {
    this.t = t;
    this.cols = cols;
    this.rows = rows;
    this.mode = mode;
    this.font = font;
  }
}

/// From the page: {"t":"size",cols,rows} (what fits), {"t":"font",size}, {"t":"open",path}.
interface PageMsg {
  t?: string;
  cols?: number;
  rows?: number;
  size?: number;
  path?: string;
}

class Attachment {
  id: number = 0;
  name: string = '';
  state: string = 'uploading';
  path: string = '';
}

/// The font size people pinched to, kept while the app runs.
let fontSize = 12;
let nextAttachment = 1;

@Entry
@Component
struct Terminal {
  @State title: string = '';
  @State detail: string = '';
  @State group: number = Group.Shell;
  @State input: string = '';
  @State notice: string = '';
  @State phoneWidth: boolean = true;
  @State prompt: PromptInfo | null = null;
  @State answering: number = 0;
  @State promptOpen: boolean = true;
  @State attachments: Attachment[] = [];
  @State altHint: string = '';
  private forceNext: boolean = false;
  private fitCols: number = 0;
  private fitRows: number = 0;
  private address: string = (router.getParams() as TermParams).address;
  private controller: webview.WebviewController = new webview.WebviewController();
  private ports: webview.WebMessagePort[] = [];
  private pageReady: boolean = false;
  private channel: number = -1;
  private attaching: boolean = false;
  private visible: boolean = true;
  private listener: () => void = () => this.refresh();

  aboutToAppear(): void {
    Session.shared.subscribe(this.listener);
    this.refresh();
  }

  aboutToDisappear(): void {
    Session.shared.unsubscribe(this.listener);
    Session.shared.detach(this.channel);
    this.channel = -1;
    for (const p of this.ports) {
      p.close();
    }
  }

  onPageShow(): void {
    this.visible = true;
    this.refresh();
  }

  onPageHide(): void {
    this.visible = false;
  }

  private agent(): Agent | undefined {
    return Session.shared.agents.get(this.address);
  }

  private refresh(): void {
    const s = Session.shared;
    const a = this.agent();
    if (s.conn !== Conn.Ready) {
      this.notice = s.conn === Conn.Connecting ? '正在连接 Mac…' : `已断开${s.error ? '：' + s.error : ''}，正在重连…`;
    } else if (a === undefined) {
      this.notice = '这个终端已经结束';
    } else {
      this.notice = '';
    }
    if (a === undefined) {
      return;
    }
    const folder = a.folder();
    this.title = folder ? `${a.label()} · ${folder}` : a.label();
    this.detail = a.detail();
    const wasBlocked = this.group === Group.NeedsYou;
    this.group = a.group();
    const p = a.group() === Group.NeedsYou ? a.prompt : null;
    if (p !== this.prompt) {
      this.prompt = p;
      this.answering = 0;
      if (p !== null && !wasBlocked) {
        this.promptOpen = true;
      }
    }
    if (this.visible) {
      s.markSeen(a);
    }
    if (s.conn === Conn.Ready && this.pageReady && this.channel < 0 && !this.attaching) {
      this.attach(a);
    }
  }

  private post(m: string | ArrayBuffer): void {
    if (this.ports.length > 1) {
      this.ports[1].postMessageEvent(m);
    }
  }

  private initPorts(): void {
    this.ports = this.controller.createWebMessagePorts();
    this.ports[1].onMessageEvent((m: webview.WebMessage) => {
      if (typeof m !== 'string') {
        return;
      }
      const msg = JSON.parse(m as string) as PageMsg;
      if (msg.t === 'font' && msg.size) {
        fontSize = msg.size;
        return;
      }
      if (msg.t === 'open' && msg.path) {
        this.openPath(msg.path);
        return;
      }
      if (msg.t !== 'size' || !msg.cols || !msg.rows) {
        return;
      }
      const first = !this.pageReady;
      const changed = msg.cols !== this.fitCols || msg.rows !== this.fitRows;
      this.fitCols = msg.cols;
      this.fitRows = msg.rows;
      this.pageReady = true;
      if (first) {
        this.refresh();
      } else if (changed && this.phoneWidth && this.channel >= 0) {
        Session.shared.resize(this.channel, this.fitCols, this.fitRows);
      }
    });
    this.controller.postMessage('__init_port__', [this.ports[0]], '*');
  }

  private attach(a: Agent): void {
    this.attaching = true;
    const phone = this.phoneWidth && this.fitCols > 0;
    const mode = phone ? 'phone' : 'full';
    this.post(JSON.stringify(phone ? new SizeMsg('init', this.fitCols, this.fitRows, mode, fontSize)
      : new SizeMsg('init', a.cols, a.rows, mode, fontSize)));
    const sink: StreamSink = {
      data: (b: Uint8Array) => {
        this.post(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer);
      },
      resize: (cols: number, rows: number) => {
        this.post(JSON.stringify(new SizeMsg('resize', cols, rows, this.phoneWidth ? 'phone' : 'full', fontSize)));
      },
      closed: (why: string) => {
        this.channel = -1;
        this.refresh();
      }
    };
    Session.shared.attach(a, phone ? this.fitCols : 0, phone ? this.fitRows : 0, sink).then((ch: number) => {
      this.channel = ch;
      this.attaching = false;
    }).catch((e: Error) => {
      this.attaching = false;
      this.notice = `打不开终端：${e.message}`;
    });
  }

  private toggleWidth(): void {
    this.phoneWidth = !this.phoneWidth;
    Session.shared.detach(this.channel);
    this.channel = -1;
    const a = this.agent();
    if (a !== undefined && Session.shared.conn === Conn.Ready && this.pageReady && !this.attaching) {
      this.attach(a);
    }
  }

  private press(keys: string[], label: string): void {
    Session.shared.keys(this.address, keys).catch((e: Error) => {
      promptAction.showToast({ message: e.message });
    });
    if (label) {
      this.altHint = label;
      setTimeout(() => {
        this.altHint = '';
      }, 700);
    }
  }

  private choose(o: PromptOption): void {
    if (this.answering !== 0) {
      return;
    }
    this.answering = o.index;
    Session.shared.answer(this.address, o.index).then(() => {
      if (o.freeText) {
        // The agent now waits for typed text: send it from the box (past the "blocked" guard).
        this.forceNext = true;
        this.answering = 0;
        focusControl.requestFocus('term-input');
        promptAction.showToast({ message: '在下面输入你的回答，然后发送' });
      }
    }).catch((e: Error) => {
      this.answering = 0;
      promptAction.showToast({ message: e.message });
    });
  }

  private send(force: boolean): void {
    const text = this.input.trim();
    if (!text) {
      return;
    }
    if (this.attachments.some((x: Attachment) => x.state === 'uploading')) {
      promptAction.showToast({ message: '附件还在上传' });
      return;
    }
    const f = force || this.forceNext;
    Session.shared.sendText(this.address, text, f).then(() => {
      this.input = '';
      this.attachments = [];
      this.forceNext = false;
    }).catch((e: Error) => {
      const blocked = this.group === Group.NeedsYou && !f;
      promptAction.showToast({
        message: blocked ? 'Agent 正在等你回答上面的问题：点选项，或长按「发送」强制输入' : e.message,
        duration: 4000
      });
    });
  }

  private addAttachments(uris: string[]): void {
    for (const uri of uris) {
      const att = new Attachment();
      att.id = nextAttachment++;
      att.name = uri.split('/').pop() ?? 'file';
      this.attachments = this.attachments.concat([att]);
      load(uri).then((p) => {
        att.name = p.name;
        return Session.shared.put(this.address, p.name, p.bytes);
      }).then((path: string) => {
        att.state = 'done';
        att.path = path;
        const sep = this.input && !this.input.endsWith(' ') ? ' ' : '';
        this.input = `${this.input}${sep}${path} `;
        this.attachments = this.attachments.slice();
      }).catch((e: Error) => {
        att.state = 'failed';
        this.attachments = this.attachments.slice();
        promptAction.showToast({ message: `${att.name} 没发出去：${e.message}` });
      });
    }
  }

  private attachMenu(): void {
    const ctx = getContext(this) as common.UIAbilityContext;
    promptAction.showActionMenu({
      title: '交给 Agent',
      buttons: [
        { text: '相册', color: '#4C9AFF' },
        { text: '拍照', color: '#4C9AFF' },
        { text: '文件', color: '#4C9AFF' }
      ]
    }).then((r: promptAction.ActionMenuSuccessResponse) => {
      const pick = r.index === 0 ? pickPhotos() : r.index === 1 ? takePhoto(ctx) : pickFiles(ctx);
      pick.then((uris: string[]) => this.addAttachments(uris)).catch((e: Error) => {
        promptAction.showToast({ message: e.message });
      });
    }).catch((): void => {});
  }

  private removeAttachment(att: Attachment): void {
    this.attachments = this.attachments.filter((x: Attachment) => x.id !== att.id);
    if (att.path) {
      this.input = this.input.replace(`${att.path} `, '').replace(att.path, '');
    }
  }

  private openPath(path: string): void {
    const a = this.agent();
    const p = new PreviewParams();
    p.path = path;
    p.cwd = a?.cwd ?? '';
    router.pushUrl({ url: 'pages/Preview', params: p });
  }

  private openChanges(): void {
    const a = this.agent();
    const p = new PreviewParams();
    p.cwd = a?.cwd ?? '';
    p.changes = true;
    router.pushUrl({ url: 'pages/Preview', params: p });
  }

  @Builder
  promptCard(p: PromptInfo) {
    Column({ space: 8 }) {
      Row() {
        Text(p.question ? p.question : 'Agent 在等你回答').fontSize(14).fontColor('#FFD08A').fontWeight(FontWeight.Medium)
          .maxLines(this.promptOpen ? 4 : 1).textOverflow({ overflow: TextOverflow.Ellipsis }).layoutWeight(1)
        Text(this.promptOpen ? '收起' : '展开').fontSize(12).fontColor('#9AA0A6').padding({ left: 10 })
          .onClick(() => {
            this.promptOpen = !this.promptOpen;
          })
      }.width('100%')
      if (this.promptOpen) {
        Scroll() {
          Column({ space: 6 }) {
            ForEach(p.options, (o: PromptOption) => {
              Row({ space: 10 }) {
                Text(`${o.index}`).fontSize(13).fontColor('#101114').fontWeight(FontWeight.Bold)
                  .width(22).height(22).textAlign(TextAlign.Center).borderRadius(11)
                  .backgroundColor(o.index === p.current ? '#FFB547' : '#C9CDD2')
                Column({ space: 2 }) {
                  Text(o.label).fontSize(15).fontColor('#E6E6E6').maxLines(2).textOverflow({ overflow: TextOverflow.Ellipsis })
                  if (o.detail) {
                    Text(o.detail).fontSize(12).fontColor('#8A8F98').maxLines(2).textOverflow({ overflow: TextOverflow.Ellipsis })
                  }
                }.alignItems(HorizontalAlign.Start).layoutWeight(1)
                if (this.answering === o.index) {
                  LoadingProgress().width(20).height(20).color('#FFB547')
                } else if (o.freeText) {
                  Text('输入').fontSize(12).fontColor('#4C9AFF')
                }
              }
              .width('100%').padding({ left: 10, right: 10, top: 9, bottom: 9 }).borderRadius(10)
              .backgroundColor(o.index === p.current ? '#2C2618' : '#1C1E23')
              .onClick(() => this.choose(o))
            }, (o: PromptOption) => `${o.index}|${o.label}`)
          }
        }.constraintSize({ maxHeight: 260 }).scrollBar(BarState.Off)
      }
    }
    .width('100%').padding(10).backgroundColor('#15171B')
    .border({ width: { top: 1 }, color: '#3A3220' })
  }

  @Builder
  attachmentStrip() {
    Scroll() {
      Row({ space: 6 }) {
        ForEach(this.attachments, (att: Attachment) => {
          Row({ space: 6 }) {
            if (att.state === 'uploading') {
              LoadingProgress().width(14).height(14).color('#9AA0A6')
            }
            Text(att.name).fontSize(12).maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
              .constraintSize({ maxWidth: 160 })
              .fontColor(att.state === 'failed' ? '#FF5D5D' : '#C9CDD2')
            Text('×').fontSize(14).fontColor('#8A8F98').onClick(() => this.removeAttachment(att))
          }
          .padding({ left: 10, right: 8, top: 5, bottom: 5 }).borderRadius(14).backgroundColor('#22262B')
        }, (att: Attachment) => `${att.id}|${att.state}`)
      }.padding({ left: 8, right: 8 })
    }.scrollable(ScrollDirection.Horizontal).scrollBar(BarState.Off).width('100%').height(36)
  }

  build() {
    Column() {
      Row({ space: 6 }) {
        Button('‹').fontSize(26).backgroundColor(Color.Transparent).fontColor('#E6E6E6')
          .onClick(() => router.back())
        Column() {
          Text(this.title).fontSize(16).fontColor('#E6E6E6').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
          Text(this.notice ? this.notice : this.detail).fontSize(12)
            .fontColor(this.notice ? '#FF8A5D' : this.group === Group.NeedsYou ? '#FFB547' : '#9AA0A6')
            .maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
        }.alignItems(HorizontalAlign.Start).layoutWeight(1)
        Button('改动').fontSize(12).height(28).padding({ left: 10, right: 10 })
          .backgroundColor('#22262B').fontColor('#C9CDD2')
          .onClick(() => this.openChanges())
        Button(this.phoneWidth ? '手机宽度' : '原尺寸').fontSize(12).height(28).padding({ left: 10, right: 10 })
          .backgroundColor('#22262B').fontColor('#C9CDD2')
          .onClick(() => this.toggleWidth())
      }
      .width('100%').padding({ right: 10, top: 6, bottom: 6 })

      Stack({ alignContent: Alignment.Top }) {
        Web({ src: $rawfile('term/index.html'), controller: this.controller })
          .width('100%').height('100%')
          .backgroundColor('#101114')
          .javaScriptAccess(true)
          // Never enlarge xterm's small text past the size it measured cells with.
          .minFontSize(1)
          .minLogicalFontSize(1)
          .domStorageAccess(false)
          .onPageEnd(() => this.initPorts())
        if (this.altHint) {
          Text(this.altHint).fontSize(13).fontColor('#101114').backgroundColor('#E6E6E6')
            .padding({ left: 12, right: 12, top: 4, bottom: 4 }).borderRadius(12).margin({ top: 8 })
        }
      }.layoutWeight(1).width('100%')

      if (this.prompt !== null) {
        this.promptCard(this.prompt)
      }

      Scroll() {
        Row({ space: 6 }) {
          ForEach(QUICK, (k: QuickKey) => {
            Button(k.label).fontSize(14).height(34).padding({ left: 12, right: 12 })
              .backgroundColor('#22262B').fontColor('#E6E6E6')
              .onClick(() => this.press(k.keys, ''))
              .gesture(LongPressGesture({ duration: 400 }).onAction(() => this.press(k.altKeys, k.altLabel)))
          }, (k: QuickKey) => k.label)
        }.padding({ left: 8, right: 8 })
      }
      .scrollable(ScrollDirection.Horizontal).scrollBar(BarState.Off).height(46).width('100%')

      if (this.attachments.length > 0) {
        this.attachmentStrip()
      }

      Row({ space: 8 }) {
        Text('+').fontSize(26).fontColor('#C9CDD2').width(40).height(40).textAlign(TextAlign.Center)
          .borderRadius(20).backgroundColor('#22262B')
          .onClick(() => this.attachMenu())
        TextInput({ text: this.input, placeholder: '发给这个 Agent（发送后自动回车）' })
          .id('term-input')
          .layoutWeight(1).height(40).fontSize(15)
          .fontColor('#E6E6E6').placeholderColor('#6B7075').backgroundColor('#1C1E23').caretColor('#4C9AFF')
          .enterKeyType(EnterKeyType.Send)
          .onChange((v: string) => {
            this.input = v;
          })
          .onSubmit(() => this.send(false))
        Button('发送').height(40).enabled(this.input.trim().length > 0)
          .onClick(() => this.send(false))
          .gesture(LongPressGesture().onAction(() => this.send(true)))
      }
      .width('100%').padding({ left: 8, right: 8, bottom: 8 })
    }
    .width('100%').height('100%').backgroundColor('#101114')
  }
}
