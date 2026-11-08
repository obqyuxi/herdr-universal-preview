// One terminal: its live screen (xterm.js in ArkWeb, watch-only) plus quick
// keys and a text box. Typing goes through terminal.keys / terminal.send, not
// xterm, which sidesteps the soft keyboard and IME problems xterm.js has on
// HarmonyOS 7 (docs/harmony/DESIGN.md §7).
import { promptAction, router } from '@kit.ArkUI';
import { webview } from '@kit.ArkWeb';
import { Conn, Session, TermParams } from '../core/Session';
import { Agent, Group } from '../core/Agents';
import { StreamSink } from '../core/Hc';

class QuickKey {
  label: string;
  keys: string[];

  constructor(label: string, keys: string[]) {
    this.label = label;
    this.keys = keys;
  }
}

const QUICK: QuickKey[] = [
  new QuickKey('Esc', ['esc']),
  new QuickKey('Enter', ['enter']),
  new QuickKey('↑', ['up']),
  new QuickKey('↓', ['down']),
  new QuickKey('←', ['left']),
  new QuickKey('→', ['right']),
  new QuickKey('Tab', ['tab']),
  new QuickKey('⇧Tab', ['shift+tab']),
  new QuickKey('^C', ['ctrl+c']),
  new QuickKey('1', ['1']),
  new QuickKey('2', ['2']),
  new QuickKey('3', ['3']),
  new QuickKey('y', ['y']),
  new QuickKey('n', ['n'])
];

class SizeMsg {
  t: string;
  cols: number;
  rows: number;
  mode: string;

  constructor(t: string, cols: number, rows: number, mode: string) {
    this.t = t;
    this.cols = cols;
    this.rows = rows;
    this.mode = mode;
  }
}

/// What the page reports: {"t":"size",cols,rows} = how much fits on the phone at a readable size.
interface PageMsg {
  t?: string;
  cols?: number;
  rows?: number;
}

@Entry
@Component
struct Terminal {
  @State title: string = '';
  @State detail: string = '';
  @State group: number = Group.Shell;
  @State input: string = '';
  @State notice: string = '';
  /// Phone width: the terminal takes the phone's size while shown here (the core gives
  /// the desktop its size back afterwards). Off: watch at the terminal's own size.
  @State phoneWidth: boolean = true;
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
    const a = this.agent();
    if (a === undefined) {
      this.notice = Session.shared.conn === Conn.Ready ? '这个终端已经结束' : '正在重新连接…';
      return;
    }
    const folder = a.folder();
    this.title = folder ? `${a.label()} · ${folder}` : a.label();
    this.detail = a.detail();
    this.group = a.group();
    if (this.visible) {
      Session.shared.markSeen(a);
    }
    if (Session.shared.conn === Conn.Ready && this.pageReady && this.channel < 0 && !this.attaching) {
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
    this.notice = '';
    const phone = this.phoneWidth && this.fitCols > 0;
    const mode = phone ? 'phone' : 'full';
    this.post(JSON.stringify(phone ? new SizeMsg('init', this.fitCols, this.fitRows, mode) : new SizeMsg('init', a.cols, a.rows, mode)));
    const sink: StreamSink = {
      data: (b: Uint8Array) => {
        this.post(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer);
      },
      resize: (cols: number, rows: number) => {
        this.post(JSON.stringify(new SizeMsg('resize', cols, rows, this.phoneWidth ? 'phone' : 'full')));
      },
      closed: (why: string) => {
        this.channel = -1;
        this.notice = '画面已断开，正在重连…';
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

  private press(k: QuickKey): void {
    Session.shared.keys(this.address, k.keys).catch((e: Error) => {
      promptAction.showToast({ message: e.message });
    });
  }

  private send(force: boolean): void {
    const text = this.input;
    if (!text) {
      return;
    }
    Session.shared.sendText(this.address, text, force).then(() => {
      this.input = '';
    }).catch((e: Error) => {
      const blocked = this.group === Group.NeedsYou && !force;
      promptAction.showToast({
        message: blocked ? 'Agent 正在等你确认：用上面的快捷键回答，或长按「发送」强制输入' : e.message,
        duration: 4000
      });
    });
  }

  build() {
    Column() {
      Row({ space: 8 }) {
        Button('‹').fontSize(26).backgroundColor(Color.Transparent).fontColor('#E6E6E6')
          .onClick(() => router.back())
        Column() {
          Text(this.title).fontSize(16).fontColor('#E6E6E6').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
          Text(this.notice ? this.notice : this.detail).fontSize(12)
            .fontColor(this.group === Group.NeedsYou ? '#FFB547' : '#9AA0A6')
            .maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
        }.alignItems(HorizontalAlign.Start).layoutWeight(1)
        Button(this.phoneWidth ? '手机宽度' : '原尺寸').fontSize(12).height(28).padding({ left: 10, right: 10 })
          .backgroundColor('#22262B').fontColor('#C9CDD2')
          .onClick(() => this.toggleWidth())
      }
      .width('100%').padding({ right: 12, top: 6, bottom: 6 })

      Web({ src: $rawfile('term/index.html'), controller: this.controller })
        .layoutWeight(1)
        .width('100%')
        .backgroundColor('#101114')
        .javaScriptAccess(true)
        // Never enlarge xterm's small text past the size it measured cells with.
        .minFontSize(1)
        .minLogicalFontSize(1)
        .domStorageAccess(false)
        .onPageEnd(() => this.initPorts())

      Scroll() {
        Row({ space: 6 }) {
          ForEach(QUICK, (k: QuickKey) => {
            Button(k.label).fontSize(14).height(34).padding({ left: 12, right: 12 })
              .backgroundColor('#22262B').fontColor('#E6E6E6')
              .onClick(() => this.press(k))
          }, (k: QuickKey) => k.label)
        }.padding({ left: 8, right: 8 })
      }
      .scrollable(ScrollDirection.Horizontal).scrollBar(BarState.Off).height(46).width('100%')

      Row({ space: 8 }) {
        TextInput({ text: this.input, placeholder: '发给这个 Agent（发送后自动回车）' })
          .layoutWeight(1).height(40).fontSize(15)
          .fontColor('#E6E6E6').placeholderColor('#6B7075').backgroundColor('#1C1E23').caretColor('#4C9AFF')
          .enterKeyType(EnterKeyType.Send)
          .onChange((v: string) => {
            this.input = v;
          })
          .onSubmit(() => this.send(false))
        Button('发送').height(40).enabled(this.input.length > 0)
          .onClick(() => this.send(false))
          .gesture(LongPressGesture().onAction(() => this.send(true)))
      }
      .width('100%').padding({ left: 8, right: 8, bottom: 8 })
    }
    .width('100%').height('100%').backgroundColor('#101114')
  }
}
