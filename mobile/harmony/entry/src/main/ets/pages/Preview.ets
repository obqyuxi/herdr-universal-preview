// What an agent delivered: the git changes in its folder (files, +/- counts,
// each file's diff) and a preview of any file it mentions — markdown, code,
// images, HTML (no scripts). Everything is read on the Mac through the bridge
// (machine.exec / file.read); nothing is stored on the phone.
import { promptAction, router } from '@kit.ArkUI';
import { webview } from '@kit.ArkWeb';
import { util } from '@kit.ArkTS';
import { Session } from '../core/Session';
import { fromUtf8 } from '../core/Bytes';

export class PreviewParams {
  path: string = '';
  cwd: string = '';
  /// Open on the folder's git changes instead of one file.
  changes: boolean = false;
}

class Change {
  status: string = '';
  path: string = '';
  add: number = -1;
  del: number = -1;
  untracked: boolean = false;
}

class ShowMsg {
  kind: string;
  text: string;
  data: string;

  constructor(kind: string, text: string, data: string) {
    this.kind = kind;
    this.text = text;
    this.data = data;
  }
}

const IMAGE: Record<string, string> = {
  'png': 'image/png', 'jpg': 'image/jpeg', 'jpeg': 'image/jpeg', 'gif': 'image/gif',
  'webp': 'image/webp', 'bmp': 'image/bmp', 'svg': 'image/svg+xml'
};
const MAX_LINES = 4000;

function q(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

function baseName(p: string): string {
  return p.split('/').filter((x: string) => x.length > 0).pop() ?? p;
}

@Entry
@Component
struct Preview {
  @State title: string = '';
  @State subtitle: string = '';
  @State loading: boolean = true;
  @State error: string = '';
  @State changes: Change[] = [];
  @State summary: string = '';
  @State selected: Change | null = null;
  @State view: string = 'diff';
  private params: PreviewParams = router.getParams() as PreviewParams;
  private root: string = '';
  private controller: webview.WebviewController = new webview.WebviewController();
  private ports: webview.WebMessagePort[] = [];
  private ready: boolean = false;
  private pending: ShowMsg | null = null;

  aboutToAppear(): void {
    if (this.params.changes) {
      this.title = '改动';
      this.loadChanges();
    } else {
      this.openFile(this.params.path);
    }
  }

  aboutToDisappear(): void {
    for (const p of this.ports) {
      p.close();
    }
  }

  onBackPress(): boolean {
    if (this.params.changes && this.selected !== null) {
      this.selected = null;
      this.title = '改动';
      this.subtitle = this.summary;
      return true;
    }
    return false;
  }

  private show(m: ShowMsg): void {
    this.loading = false;
    if (this.ready && this.ports.length > 1) {
      this.ports[1].postMessageEvent(JSON.stringify(m));
    } else {
      this.pending = m;
    }
  }

  private initPorts(): void {
    this.ports = this.controller.createWebMessagePorts();
    this.ports[1].onMessageEvent((m: webview.WebMessage) => {
      if (m === 'ready') {
        this.ready = true;
        const p = this.pending;
        this.pending = null;
        if (p !== null) {
          this.show(p);
        }
      }
    });
    this.controller.postMessage('__init_port__', [this.ports[0]], '*');
  }

  private async loadChanges(): Promise<void> {
    this.loading = true;
    this.error = '';
    try {
      const s = Session.shared;
      const top = await s.exec(`git -C ${q(this.params.cwd)} rev-parse --show-toplevel`, null);
      if (top.code !== 0) {
        this.error = '这个终端的目录不是 git 仓库，没有可看的改动';
        this.loading = false;
        return;
      }
      this.root = fromUtf8(top.stdout).trim();
      const st = await s.exec(`git -C ${q(this.root)} status --porcelain=v1 -uall`, null);
      const ns = await s.exec(`git -C ${q(this.root)} diff --numstat HEAD 2>/dev/null`, null);
      const br = await s.exec(`git -C ${q(this.root)} branch --show-current`, null);
      const counts = new Map<string, number[]>();
      for (const line of fromUtf8(ns.stdout).split('\n')) {
        const f = line.split('\t');
        if (f.length >= 3) {
          counts.set(f[2], [f[0] === '-' ? -1 : Number.parseInt(f[0]), f[1] === '-' ? -1 : Number.parseInt(f[1])]);
        }
      }
      const list: Change[] = [];
      let add = 0;
      let del = 0;
      for (const line of fromUtf8(st.stdout).split('\n')) {
        if (line.length < 4) {
          continue;
        }
        const c = new Change();
        c.status = line.substring(0, 2).trim();
        let path = line.substring(3);
        if (path.includes(' -> ')) {
          path = path.split(' -> ')[1];
        }
        c.path = path.replace(/^"|"$/g, '');
        c.untracked = c.status === '??';
        const n = counts.get(c.path);
        if (n !== undefined) {
          c.add = n[0];
          c.del = n[1];
          add += Math.max(0, n[0]);
          del += Math.max(0, n[1]);
        }
        list.push(c);
      }
      this.changes = list;
      const branch = fromUtf8(br.stdout).trim();
      this.summary = `${baseName(this.root)}${branch ? ' · ' + branch : ''} · ${list.length} 个文件 · +${add} −${del}`;
      this.subtitle = this.summary;
      this.loading = false;
    } catch (e) {
      this.error = (e as Error).message;
      this.loading = false;
    }
  }

  private openChange(c: Change): void {
    this.selected = c;
    this.title = baseName(c.path);
    this.subtitle = c.path;
    this.view = c.untracked ? 'file' : 'diff';
    if (c.untracked) {
      this.openFile(`${this.root}/${c.path}`);
    } else {
      this.showDiff(c);
    }
  }

  private async showDiff(c: Change): Promise<void> {
    this.loading = true;
    try {
      const d = await Session.shared.exec(`git -C ${q(this.root)} diff HEAD -- ${q(c.path)}`, null);
      const text = fromUtf8(d.stdout);
      this.show(new ShowMsg(text ? 'diff' : 'note', text ? text : '没有文本差异（可能是二进制文件或只改了权限）', ''));
    } catch (e) {
      this.show(new ShowMsg('note', (e as Error).message, ''));
    }
  }

  private async resolve(path: string): Promise<string> {
    let p = path.trim().replace(/:\d+(:\d+)?$/, '');
    if (p.startsWith('~/')) {
      const home = await Session.shared.exec('printf %s "$HOME"', null);
      p = fromUtf8(home.stdout) + p.substring(1);
    } else if (!p.startsWith('/')) {
      p = `${this.params.cwd}/${p}`;
    }
    return p;
  }

  private async openFile(path: string): Promise<void> {
    this.loading = true;
    this.error = '';
    try {
      const abs = await this.resolve(path);
      if (!this.params.changes) {
        this.title = baseName(abs);
        this.subtitle = abs;
      }
      const dot = abs.lastIndexOf('.');
      const ext = dot < 0 ? '' : abs.substring(dot + 1).toLowerCase();
      const mime = IMAGE[ext];
      if (mime !== undefined) {
        const r = await Session.shared.exec(`cat ${q(abs)}`, null);
        if (r.code !== 0) {
          throw new Error(r.stderr || '读不到这个文件');
        }
        const data = `data:${mime};base64,${new util.Base64Helper().encodeToStringSync(r.stdout)}`;
        this.show(new ShowMsg('image', '', data));
        return;
      }
      const f = await Session.shared.readFile(abs, null, MAX_LINES);
      let text = f.text ?? '';
      if ((f.lines ?? 0) > MAX_LINES) {
        text += `\n… 还有 ${(f.lines ?? 0) - MAX_LINES} 行没显示`;
      }
      const kind = ext === 'md' || ext === 'markdown' ? 'markdown' : ext === 'html' || ext === 'htm' ? 'html' : 'text';
      this.show(new ShowMsg(kind, text, ''));
    } catch (e) {
      this.show(new ShowMsg('note', `无法预览：${(e as Error).message}`, ''));
    }
  }

  @Builder
  changeRow(c: Change) {
    Row({ space: 10 }) {
      Text(c.untracked ? 'U' : c.status.substring(0, 1)).fontSize(12).fontWeight(FontWeight.Bold).width(18)
        .fontColor(c.untracked ? '#36C275' : c.status.startsWith('D') ? '#FF5D5D' : '#FFB547')
      Column({ space: 2 }) {
        Text(baseName(c.path)).fontSize(15).fontColor('#E6E6E6').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
        Text(c.path).fontSize(11).fontColor('#6B7075').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis }).ellipsisMode(EllipsisMode.CENTER)
      }.alignItems(HorizontalAlign.Start).layoutWeight(1)
      if (c.add >= 0) {
        Text(`+${c.add}`).fontSize(12).fontColor('#36C275')
        Text(`−${c.del}`).fontSize(12).fontColor('#FF5D5D')
      } else if (c.untracked) {
        Text('新文件').fontSize(12).fontColor('#36C275')
      }
    }
    .width('100%').padding({ left: 16, right: 16, top: 11, bottom: 11 })
    .onClick(() => this.openChange(c))
  }

  build() {
    Column() {
      Row({ space: 6 }) {
        Button('‹').fontSize(26).backgroundColor(Color.Transparent).fontColor('#E6E6E6')
          .onClick(() => {
            if (!this.onBackPress()) {
              router.back();
            }
          })
        Column() {
          Text(this.title).fontSize(16).fontColor('#E6E6E6').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
          Text(this.subtitle).fontSize(11).fontColor('#8A8F98').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis }).ellipsisMode(EllipsisMode.CENTER)
        }.alignItems(HorizontalAlign.Start).layoutWeight(1)
        if (this.selected !== null && !this.selected.untracked) {
          Button(this.view === 'diff' ? '看文件' : '看差异').fontSize(12).height(28).padding({ left: 10, right: 10 })
            .backgroundColor('#22262B').fontColor('#C9CDD2')
            .onClick(() => {
              const c = this.selected as Change;
              this.view = this.view === 'diff' ? 'file' : 'diff';
              if (this.view === 'diff') {
                this.showDiff(c);
              } else {
                this.openFile(`${this.root}/${c.path}`);
              }
            })
        } else if (this.params.changes && this.selected === null) {
          Button('刷新').fontSize(12).height(28).padding({ left: 10, right: 10 })
            .backgroundColor('#22262B').fontColor('#C9CDD2')
            .onClick(() => this.loadChanges())
        }
      }.width('100%').padding({ right: 10, top: 6, bottom: 6 })

      Stack() {
        // The web view stays loaded (port ready); the list covers it on the changes screen.
        Web({ src: $rawfile('preview/index.html'), controller: this.controller })
          .width('100%').height('100%').backgroundColor('#101114')
          .javaScriptAccess(true).domStorageAccess(false)
          .onPageEnd(() => this.initPorts())
        if (this.params.changes && this.selected === null) {
          Column() {
            if (this.error) {
              Text(this.error).fontSize(14).fontColor('#9AA0A6').margin({ top: 48 }).padding({ left: 24, right: 24 })
                .textAlign(TextAlign.Center)
            } else if (!this.loading && this.changes.length === 0) {
              Text('没有未提交的改动').fontSize(14).fontColor('#9AA0A6').margin({ top: 48 })
            }
            List() {
              ForEach(this.changes, (c: Change) => {
                ListItem() {
                  this.changeRow(c)
                }
              }, (c: Change) => `${c.status}|${c.path}|${c.add}|${c.del}`)
            }.layoutWeight(1).divider({ strokeWidth: 0.5, color: '#22262B', startMargin: 44 })
          }.width('100%').height('100%').backgroundColor('#101114')
        }
        if (this.loading) {
          LoadingProgress().width(36).height(36).color('#9AA0A6')
        }
      }.layoutWeight(1).width('100%')
    }
    .width('100%').height('100%').backgroundColor('#101114')
  }
}
