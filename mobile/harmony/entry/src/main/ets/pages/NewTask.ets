// Start an agent from the phone: pick a folder (where your terminals already
// run, or type one), pick the agent, write the first message, go. It runs in
// your login shell on the Mac (so PATH has your tools), then opens here.
import { LengthMetrics, promptAction, router } from '@kit.ArkUI';
import { Session, TermParams } from '../core/Session';

class AgentChoice {
  label: string;
  /// Shell line run through the login shell; $DDUP_PROMPT holds the first message.
  line: string;
  /// The first message goes on the command line (else it's sent once it starts).
  takesPrompt: boolean;

  constructor(label: string, line: string, takesPrompt: boolean) {
    this.label = label;
    this.line = line;
    this.takesPrompt = takesPrompt;
  }
}

const AGENTS: AgentChoice[] = [
  new AgentChoice('Claude Code', 'exec claude', true),
  new AgentChoice('Codex', 'exec codex', true),
  new AgentChoice('omp', 'exec omp', false),
  new AgentChoice('Shell', 'exec "${SHELL:-/bin/zsh}" -l', false)
];

@Entry
@Component
struct NewTask {
  @State folders: string[] = [];
  @State folder: string = '';
  @State custom: string = '';
  @State agent: number = 0;
  @State prompt: string = '';
  @State busy: boolean = false;

  aboutToAppear(): void {
    this.folders = Session.shared.recentFolders().slice(0, 8);
    this.folder = this.folders.length > 0 ? this.folders[0] : '';
  }

  private chosenFolder(): string {
    return this.custom.trim() ? this.custom.trim() : this.folder;
  }

  private async start(): Promise<void> {
    const cwd = this.chosenFolder();
    if (!cwd) {
      promptAction.showToast({ message: '先选一个目录' });
      return;
    }
    this.busy = true;
    const choice = AGENTS[this.agent];
    const text = this.prompt.trim();
    const line = choice.takesPrompt && text ? `${choice.line} "$DDUP_PROMPT"` : choice.line;
    const env: Record<string, string> = { 'DDUP_CMD': line, 'DDUP_PROMPT': text };
    try {
      const address = await Session.shared.run('exec "${SHELL:-/bin/zsh}" -lic \'eval "$DDUP_CMD"\'', cwd, env);
      if (!address) {
        throw new Error('core 没有返回终端地址');
      }
      if (text && !choice.takesPrompt) {
        // Programs without a prompt argument get the first message once they're up.
        setTimeout(() => {
          Session.shared.sendText(address, text, false).catch((): void => {});
        }, 4000);
      }
      const p = new TermParams();
      p.address = address;
      router.replaceUrl({ url: 'pages/Terminal', params: p });
    } catch (e) {
      this.busy = false;
      promptAction.showToast({ message: `没能启动：${(e as Error).message}`, duration: 4000 });
    }
  }

  @Builder
  chip(label: string, on: boolean, action: () => void) {
    Text(label).fontSize(14).fontColor(on ? '#101114' : '#E6E6E6')
      .padding({ left: 14, right: 14, top: 8, bottom: 8 }).borderRadius(18)
      .backgroundColor(on ? '#E6E6E6' : '#22262B')
      .onClick(action)
  }

  build() {
    Column() {
      Row({ space: 6 }) {
        Button('‹').fontSize(26).backgroundColor(Color.Transparent).fontColor('#E6E6E6')
          .onClick(() => router.back())
        Text('新建任务').fontSize(18).fontColor('#E6E6E6').fontWeight(FontWeight.Medium).layoutWeight(1)
      }.width('100%').padding({ right: 12, top: 6, bottom: 6 })

      Scroll() {
        Column({ space: 14 }) {
          Text('在哪个目录').fontSize(13).fontColor('#9AA0A6')
          Column({ space: 6 }) {
            ForEach(this.folders, (f: string) => {
              Row({ space: 10 }) {
                Circle().width(10).height(10).fill(this.folder === f && !this.custom ? '#4C9AFF' : '#3A3D44')
                Column() {
                  Text(f.split('/').filter((x: string) => x).pop() ?? f).fontSize(15).fontColor('#E6E6E6')
                  Text(f).fontSize(11).fontColor('#6B7075').maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis }).ellipsisMode(EllipsisMode.CENTER)
                }.alignItems(HorizontalAlign.Start).layoutWeight(1)
              }
              .width('100%').padding(10).borderRadius(10).backgroundColor('#1C1E23')
              .onClick(() => {
                this.folder = f;
                this.custom = '';
              })
            }, (f: string) => f)
          }
          TextInput({ text: this.custom, placeholder: '或者输入完整路径，例如 /Users/you/project' })
            .fontSize(14).fontColor('#E6E6E6').placeholderColor('#6B7075').backgroundColor('#1C1E23')
            .onChange((v: string) => {
              this.custom = v;
            })

          Text('用哪个 Agent').fontSize(13).fontColor('#9AA0A6').margin({ top: 6 })
          Flex({ wrap: FlexWrap.Wrap, space: { main: LengthMetrics.vp(8), cross: LengthMetrics.vp(8) } }) {
            ForEach(AGENTS, (a: AgentChoice, i: number) => {
              this.chip(a.label, this.agent === i, () => {
                this.agent = i;
              })
            }, (a: AgentChoice) => a.label)
          }

          Text('第一句话').fontSize(13).fontColor('#9AA0A6').margin({ top: 6 })
          TextArea({ text: this.prompt, placeholder: '例如：把首页的列表改成可以按状态筛选，改完跑一遍测试' })
            .height(140).fontSize(15).fontColor('#E6E6E6').placeholderColor('#6B7075').backgroundColor('#1C1E23')
            .onChange((v: string) => {
              this.prompt = v;
            })
        }.alignItems(HorizontalAlign.Start).padding({ left: 16, right: 16, bottom: 24 })
      }.layoutWeight(1).scrollBar(BarState.Off)

      Button(this.busy ? '启动中…' : '启动').width('90%').height(46).margin({ bottom: 16 })
        .enabled(!this.busy && this.chosenFolder().length > 0)
        .onClick(() => this.start())
    }
    .width('100%').height('100%').backgroundColor('#101114')
  }
}
