// Web preview: the dev servers running on the Mac (vite, next, python -m
// http.server, ...) and an in-app browser for one of them. The bridge opens a
// forward with a one-time token (bridge/src/ports.rs); the page then talks to
// it directly over the same network the app uses (home Wi-Fi or Tailscale),
// so hot reload works too.
import { promptAction, router } from '@kit.ArkUI';
import { webview } from '@kit.ArkWeb';
import { common, Want } from '@kit.AbilityKit';
import { Session } from '../core/Session';
import { ControlMsg, SiteInfo } from '../core/Secure';

export class WebParams {
  /// Open this port right away (from a tapped localhost link).
  port: number = 0;
  /// Path on that server, e.g. "/docs?x=1".
  path: string = '';
  /// Show servers from this folder first.
  cwd: string = '';
}

function baseName(p: string): string {
  return p.split('/').filter((x: string) => x.length > 0).pop() ?? p;
}

/// http://localhost:5173/a?b → 5173; 0 when it isn't a local address.
export function localPort(s: string): number {
  const m = s.match(/^https?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])(?::(\d+))?(\/.*)?$/);
  if (m === null) {
    return 0;
  }
  return m[1] ? Number.parseInt(m[1]) : 80;
}

export function localPath(s: string): string {
  const m = s.match(/^https?:\/\/[^/]+(\/.*)?$/);
  return m !== null && m[1] ? m[1] : '/';
}

@Entry
@Component
struct WebPreview {
  @State sites: SiteInfo[] = [];
  @State loading: boolean = true;
  @State error: string = '';
  @State open: SiteInfo | null = null;
  @State url: string = '';
  @State pageTitle: string = '';
  @State progress: number = 0;
  @State manual: string = '';
  private params: WebParams = (router.getParams() as WebParams) ?? new WebParams();
  private controller: webview.WebviewController = new webview.WebviewController();

  aboutToAppear(): void {
    if (this.params.port > 0) {
      const s: SiteInfo = { port: this.params.port, process: '', pid: 0, status: 0 };
      this.openSite(s, this.params.path);
    }
    this.load();
  }

  onBackPress(): boolean {
    if (this.open !== null) {
      if (this.controller.accessBackward()) {
        this.controller.backward();
      } else {
        this.open = null;
        this.url = '';
      }
      return true;
    }
    return false;
  }

  private load(): void {
    this.loading = true;
    this.error = '';
    Session.shared.sites().then((m: ControlMsg) => {
      const list = m.sites ?? [];
      const cwd = this.params.cwd;
      // This terminal's project first.
      this.sites = list.filter((s: SiteInfo) => cwd !== '' && (s.cwd ?? '').startsWith(cwd))
        .concat(list.filter((s: SiteInfo) => !(cwd !== '' && (s.cwd ?? '').startsWith(cwd))));
      this.loading = false;
    }).catch((e: Error) => {
      this.error = e.message;
      this.loading = false;
    });
  }

  private openSite(s: SiteInfo, path: string): void {
    this.open = s;
    this.pageTitle = s.title ? s.title : `localhost:${s.port}`;
    this.url = '';
    // The token link sets a cookie, then redirects to the path.
    Session.shared.forward(s.port, path).then((link: string) => {
      this.url = link;
    }).catch((e: Error) => {
      this.open = null;
      promptAction.showToast({ message: `打不开 ${s.port}：${e.message}`, duration: 4000 });
    });
  }

  private openManual(): void {
    const port = Number.parseInt(this.manual.trim());
    if (!(port > 0 && port < 65536)) {
      promptAction.showToast({ message: '输入端口号，例如 5173' });
      return;
    }
    const s: SiteInfo = { port: port, process: '', pid: 0, status: 0 };
    this.openSite(s, '/');
  }

  private openOutside(): void {
    const ctx = getContext(this) as common.UIAbilityContext;
    const want: Want = { action: 'ohos.want.action.viewData', entities: ['entity.system.browsable'], uri: this.controller.getUrl() };
    ctx.startAbility(want).catch((e: Error) => {
      promptAction.showToast({ message: e.message });
    });
  }

  @Builder
  siteRow(s: SiteInfo) {
    Row({ space: 12 }) {
      Text(`${s.port}`).fontSize(15).fontWeight(FontWeight.Medium).fontColor('#E6E6E6').width(56)
      Column({ space: 2 }) {
        Text(s.title ? s.title : s.process).fontSize(15).fontColor('#E6E6E6').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
        Text(`${s.process}${s.cwd ? ' · ' + baseName(s.cwd) : ''}${s.status >= 400 ? ' · ' + s.status : ''}`)
          .fontSize(12).fontColor('#8A8F98').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
      }.alignItems(HorizontalAlign.Start).layoutWeight(1)
      Text('›').fontSize(20).fontColor('#6B7075')
    }
    .width('100%').padding({ left: 16, right: 16, top: 12, bottom: 12 })
    .onClick(() => this.openSite(s, '/'))
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
          Text(this.open !== null ? this.pageTitle : '预览网页').fontSize(16).fontColor('#E6E6E6')
            .maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
          Text(this.open !== null ? `Mac 上的 localhost:${this.open.port}` : 'Mac 上正在运行的网页服务').fontSize(11).fontColor('#8A8F98')
        }.alignItems(HorizontalAlign.Start).layoutWeight(1)
        if (this.open !== null) {
          Button('刷新').fontSize(12).height(28).padding({ left: 10, right: 10 })
            .backgroundColor('#22262B').fontColor('#C9CDD2')
            .onClick(() => this.controller.refresh())
          Button('浏览器').fontSize(12).height(28).padding({ left: 10, right: 10 })
            .backgroundColor('#22262B').fontColor('#C9CDD2')
            .onClick(() => this.openOutside())
        } else {
          Button('刷新').fontSize(12).height(28).padding({ left: 10, right: 10 })
            .backgroundColor('#22262B').fontColor('#C9CDD2')
            .onClick(() => this.load())
        }
      }.width('100%').padding({ right: 10, top: 6, bottom: 6 })

      if (this.open !== null) {
        if (this.progress > 0 && this.progress < 100) {
          Progress({ value: this.progress, total: 100, type: ProgressType.Linear }).width('100%').height(2).color('#4C9AFF')
        }
        if (this.url) {
          Web({ src: this.url, controller: this.controller })
            .layoutWeight(1).width('100%')
            .javaScriptAccess(true).domStorageAccess(true).fileAccess(false)
            .onProgressChange((e) => {
              this.progress = e?.newProgress ?? 0;
            })
            .onTitleReceive((e) => {
              const t = e?.title ?? '';
              if (t && !t.startsWith('http')) {
                this.pageTitle = t;
              }
            })
        } else {
          Column() {
            LoadingProgress().width(32).height(32).color('#9AA0A6')
          }.layoutWeight(1).justifyContent(FlexAlign.Center)
        }
      } else {
        List() {
          ForEach(this.sites, (s: SiteInfo) => {
            ListItem() {
              this.siteRow(s)
            }
          }, (s: SiteInfo) => `${s.port}|${s.title}`)
        }.layoutWeight(1).divider({ strokeWidth: 0.5, color: '#22262B', startMargin: 16 })

        if (this.loading) {
          LoadingProgress().width(32).height(32).color('#9AA0A6').margin({ bottom: 24 })
        } else if (this.error || this.sites.length === 0) {
          Text(this.error ? this.error : '没有发现网页服务。让 Agent 启动开发服务器（例如 npm run dev），再点刷新。')
            .fontSize(13).fontColor('#9AA0A6').textAlign(TextAlign.Center).padding({ left: 24, right: 24, bottom: 16 })
        }
        Row({ space: 8 }) {
          TextInput({ text: this.manual, placeholder: '或输入端口，例如 5173' })
            .type(InputType.Number).layoutWeight(1).height(40).fontSize(15)
            .fontColor('#E6E6E6').placeholderColor('#6B7075').backgroundColor('#1C1E23')
            .onChange((v: string) => {
              this.manual = v;
            })
            .onSubmit(() => this.openManual())
          Button('打开').height(40).enabled(this.manual.trim().length > 0).onClick(() => this.openManual())
        }.width('100%').padding({ left: 12, right: 12, bottom: 12 })
      }
    }
    .width('100%').height('100%').backgroundColor('#101114')
  }
}
