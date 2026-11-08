// Pairing (scan or paste the bridge's link) and the agent list, grouped like
// the desktop sidebar: needs you, working, done, idle, other terminals.
import { promptAction, router } from '@kit.ArkUI';
import { scanBarcode, scanCore } from '@kit.ScanKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { common } from '@kit.AbilityKit';
import { Conn, CONN_TEXT, Session, TermParams } from '../core/Session';
import { Agent, Group, GROUP_TITLES } from '../core/Agents';
import { parsePairLink } from '../core/Link';

const GROUP_COLORS: string[] = ['#FFB547', '#4C9AFF', '#36C275', '#8A8F98', '#5C6068'];

class RowData {
  key: string = '';
  header: boolean = false;
  title: string = '';
  detail: string = '';
  address: string = '';
  group: number = 0;
  when: string = '';
}

function ago(ms: number): string {
  const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (s < 60) {
    return '刚刚';
  }
  if (s < 3600) {
    return `${Math.floor(s / 60)} 分钟前`;
  }
  if (s < 86400) {
    return `${Math.floor(s / 3600)} 小时前`;
  }
  return `${Math.floor(s / 86400)} 天前`;
}

function rowsOf(agents: Agent[]): RowData[] {
  const out: RowData[] = [];
  let last = -1;
  for (const a of agents) {
    const g = a.group() as number;
    if (g !== last) {
      const h = new RowData();
      h.key = `h${g}`;
      h.header = true;
      h.title = GROUP_TITLES[g];
      h.group = g;
      out.push(h);
      last = g;
    }
    const r = new RowData();
    r.address = a.address;
    r.group = g;
    const folder = a.folder();
    r.title = folder ? `${a.label()} · ${folder}` : a.label();
    r.detail = a.detail();
    r.when = a.state ? ago(a.statusMs) : a.address;
    r.key = `${a.address}|${g}|${r.title}|${r.detail}|${r.when}`;
    out.push(r);
  }
  return out;
}

@Entry
@Component
struct Index {
  @State paired: boolean = false;
  @State loaded: boolean = false;
  @State machineName: string = '';
  @State conn: number = Conn.Idle;
  @State error: string = '';
  @State rows: RowData[] = [];
  @State linkText: string = '';
  @State busy: boolean = false;
  private listener: () => void = () => this.refresh();
  private ticker: number = -1;

  aboutToAppear(): void {
    Session.shared.subscribe(this.listener);
    this.refresh();
    Session.shared.start();
    // Keep the "x 分钟前" labels fresh.
    this.ticker = setInterval(() => this.refresh(), 30000);
  }

  aboutToDisappear(): void {
    Session.shared.unsubscribe(this.listener);
    clearInterval(this.ticker);
  }

  onPageShow(): void {
    this.refresh();
    this.openPending();
  }

  private refresh(): void {
    const s = Session.shared;
    this.loaded = s.loaded;
    this.paired = s.machine !== null;
    this.machineName = s.machine?.name ?? '';
    this.conn = s.conn;
    this.error = s.error;
    this.rows = rowsOf(s.agents.list());
    this.openPending();
  }

  private openPending(): void {
    const s = Session.shared;
    if (s.pendingPair && s.loaded && !this.busy) {
      const link = s.pendingPair;
      s.pendingPair = '';
      this.pairWith(link);
      return;
    }
    if (s.pendingOpen && s.conn === Conn.Ready && s.agents.get(s.pendingOpen) !== undefined) {
      const address = s.pendingOpen;
      s.pendingOpen = '';
      this.open(address);
    }
  }

  private open(address: string): void {
    const p = new TermParams();
    p.address = address;
    router.pushUrl({ url: 'pages/Terminal', params: p });
  }

  private async pairWith(text: string): Promise<void> {
    if (this.busy) {
      return;
    }
    this.busy = true;
    try {
      if (Session.shared.machine !== null) {
        await Session.shared.forget();
      }
      await Session.shared.pair(parsePairLink(text));
      this.linkText = '';
      promptAction.showToast({ message: '配对成功' });
    } catch (e) {
      const m = (e as Error).message;
      const text = m === 'pair_invalid' ? '配对码无效或已用过，请在 DD-up 里点手机图标，再扫一次新的二维码'
        : m === 'pair_expired' ? '配对码已过期（10 分钟），请重新生成' : m;
      promptAction.showToast({ message: text, duration: 4000 });
    }
    this.busy = false;
    this.refresh();
  }

  private scan(): void {
    const ctx = getContext(this) as common.UIAbilityContext;
    const options: scanBarcode.ScanOptions = { scanTypes: [scanCore.ScanType.QR_CODE], enableMultiMode: false, enableAlbum: true };
    scanBarcode.startScanForResult(ctx, options).then((r: scanBarcode.ScanResult) => {
      this.pairWith(r.originalValue);
    }).catch((e: BusinessError) => {
      if (e.code !== 1000500002) { // user cancelled
        promptAction.showToast({ message: `扫码失败：${e.message}` });
      }
    });
  }

  private menu(): void {
    promptAction.showActionMenu({
      title: this.machineName,
      buttons: [
        { text: '重新连接', color: '#4C9AFF' },
        { text: '测试推送', color: '#4C9AFF' },
        { text: '移除这台 Mac', color: '#FF5D5D' }
      ]
    }).then((r: promptAction.ActionMenuSuccessResponse) => {
      if (r.index === 0) {
        Session.shared.connect();
      } else if (r.index === 1) {
        if (!Session.shared.hasPushToken()) {
          promptAction.showToast({ message: '这台手机还没拿到推送 token（需在 AppGallery Connect 开通推送并填写 client_id）', duration: 5000 });
          return;
        }
        Session.shared.testPush().then((m: string) => {
          promptAction.showToast({ message: `已发送：${m}` });
        }).catch((e: Error) => {
          promptAction.showToast({ message: e.message, duration: 6000 });
        });
      } else if (r.index === 2) {
        Session.shared.forget();
      }
    }).catch(() => {});
  }

  @Builder
  pairView() {
    Column({ space: 16 }) {
      Text('DD-up').fontSize(34).fontWeight(FontWeight.Bold).fontColor('#E6E6E6')
      Text('在 Mac 的 DD-up 里点 Settings 旁边的手机图标，扫描弹出的二维码').fontSize(15).fontColor('#9AA0A6').textAlign(TextAlign.Center)
        .padding({ left: 32, right: 32 })
      Button(this.busy ? '连接中…' : '扫码配对')
        .width('80%').height(48).enabled(!this.busy)
        .onClick(() => this.scan())
      Text('或粘贴配对链接').fontSize(13).fontColor('#6B7075').margin({ top: 12 })
      TextArea({ text: this.linkText, placeholder: 'ddup://pair?…' })
        .width('86%').height(96).fontSize(13)
        .fontColor('#E6E6E6').placeholderColor('#6B7075').backgroundColor('#1C1E23').caretColor('#4C9AFF')
        .onChange((v: string) => {
          this.linkText = v;
        })
      Button('配对').width('80%').height(44).enabled(!this.busy && this.linkText.length > 0)
        .backgroundColor('#2A2D33')
        .onClick(() => this.pairWith(this.linkText))
    }
    .width('100%').height('100%').justifyContent(FlexAlign.Center)
  }

  @Builder
  header() {
    Row() {
      Column() {
        Text(this.machineName).fontSize(20).fontWeight(FontWeight.Medium).fontColor('#E6E6E6')
        Row({ space: 6 }) {
          Circle().width(8).height(8)
            .fill(this.conn === Conn.Ready ? '#36C275' : this.conn === Conn.Connecting ? '#FFB547' : '#FF5D5D')
          Text(this.conn === Conn.Failed && this.error ? `${CONN_TEXT[this.conn]}：${this.error}` : CONN_TEXT[this.conn])
            .fontSize(12).fontColor('#9AA0A6').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
        }
      }.alignItems(HorizontalAlign.Start).layoutWeight(1)
      Button('⋯').fontSize(20).backgroundColor(Color.Transparent).fontColor('#E6E6E6')
        .onClick(() => this.menu())
    }
    .width('100%').padding({ left: 16, right: 8, top: 12, bottom: 8 })
  }

  @Builder
  agentRow(r: RowData) {
    Row({ space: 12 }) {
      Circle().width(10).height(10).fill(GROUP_COLORS[r.group])
      Column({ space: 2 }) {
        Text(r.title).fontSize(16).fontColor('#E6E6E6').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
        Text(r.detail).fontSize(13).fontColor(r.group === Group.NeedsYou ? '#FFB547' : '#9AA0A6')
          .maxLines(2).textOverflow({ overflow: TextOverflow.Ellipsis })
      }.alignItems(HorizontalAlign.Start).layoutWeight(1)
      Text(r.when).fontSize(11).fontColor('#6B7075')
    }
    .width('100%').padding({ left: 16, right: 16, top: 12, bottom: 12 })
    .onClick(() => this.open(r.address))
  }

  @Builder
  listView() {
    Column() {
      this.header()
      if (this.rows.length === 0) {
        Text(this.conn === Conn.Ready ? '没有正在运行的终端' : '').fontColor('#6B7075').margin({ top: 48 })
      }
      List() {
        ForEach(this.rows, (r: RowData) => {
          ListItem() {
            if (r.header) {
              Text(r.title).fontSize(12).fontColor(GROUP_COLORS[r.group]).fontWeight(FontWeight.Medium)
                .padding({ left: 16, top: 16, bottom: 4 }).width('100%')
            } else {
              this.agentRow(r)
            }
          }
        }, (r: RowData) => r.key)
      }
      .layoutWeight(1).divider({ strokeWidth: 0.5, color: '#22262B', startMargin: 38 })
    }
    .width('100%').height('100%')
  }

  build() {
    Column() {
      if (this.loaded && !this.paired) {
        this.pairView()
      } else if (this.paired) {
        this.listView()
      }
    }
    .width('100%').height('100%').backgroundColor('#101114')
  }
}
