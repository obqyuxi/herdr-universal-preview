import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { rpc } from '@kit.IPCKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { notificationManager } from '@kit.NotificationKit';
import { pushService } from '@kit.PushKit';
import { KeyboardAvoidMode, window } from '@kit.ArkUI';
import { Session } from '../core/Session';
import { Cards, shortLabel } from '../core/Cards';
import { PromptOption } from '../core/Prompt';

const DOMAIN = 0xd0d0;
/// The colour of every page (pages/*.ets).
const APP_BACKGROUND = '#101114';

/// What a card button sends (ets/widget/pages/WidgetCard.ets).
interface CardAnswer {
  formId?: string;
  addr?: string;
  opt?: string;
  label?: string;
}

/// The callee has to hand something back; the card ignores it.
class Nothing implements rpc.Parcelable {
  marshalling(out: rpc.MessageSequence): boolean {
    return true;
  }

  unmarshalling(src: rpc.MessageSequence): boolean {
    return true;
  }
}

/// A card button: connect (the app may only be running in the background for this),
/// check the agent still asks the same, answer, and show how it went on the card.
async function answerFromCard(c: CardAnswer, shown: boolean): Promise<void> {
  const id = c.formId ?? '';
  const say = (note: string, clear: boolean): void => {
    if (!id) {
      return;
    }
    const v: Record<string, string> = { 'note': note };
    if (clear) {
      v['o1'] = '';
      v['o2'] = '';
      v['o3'] = '';
    }
    Cards.patch(id, v);
  };
  const address = c.addr ?? '';
  const opt = Number.parseInt(c.opt ?? '');
  if (!address || !(opt > 0)) {
    return;
  }
  say(`正在发送「${c.label ?? opt}」…`, false);
  const s = Session.shared;
  if (!shown) {
    // Not on screen: the bridge keeps pushing while this connection is open.
    s.setForeground(false);
  }
  try {
    await s.ready(10000);
    const p = await s.readPrompt(address);
    const o = p?.options.find((x: PromptOption) => x.index === opt);
    if (p === null || o === undefined || (c.label && shortLabel(o.label) !== c.label)) {
      say('问题已经变了，点开看看', true);
      return;
    }
    await s.answer(address, opt);
    say(`已选「${shortLabel(o.label)}」`, true);
  } catch (e) {
    say(`没发出去：${(e as Error).message}`, false);
  }
}

export default class EntryAbility extends UIAbility {
  /// The window is on screen (a card button can start the ability without one).
  private shown: boolean = false;

  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    Cards.init(this.context);
    this.callee.on('answer', (data: rpc.MessageSequence): rpc.Parcelable => {
      try {
        answerFromCard(JSON.parse(data.readString()) as CardAnswer, this.shown);
      } catch (e) {
        hilog.warn(DOMAIN, 'ddup', 'card answer: %{public}s', (e as Error).message);
      }
      return new Nothing();
    });
    this.takeWant(want);
    // Push Kit hands back a token once the app is set up in AppGallery Connect (client_id in module.json5).
    pushService.getToken().then((token: string) => {
      Session.shared.setPushToken(token);
    }).catch((err: BusinessError) => {
      hilog.warn(DOMAIN, 'ddup', 'no push token: %{public}d %{public}s', err.code, err.message);
    });
  }

  onNewWant(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    this.takeWant(want);
  }

  /// A tapped notification carries {terminal} (clickAction.data in bridge/src/push.rs).
  private takeWant(want: Want): void {
    if (want.uri && want.uri.startsWith('ddup://pair?')) {
      Session.shared.pendingPair = want.uri;
      Session.shared.poke();
      return;
    }
    const params = want.parameters;
    if (params === undefined) {
      return;
    }
    let t: Object | undefined = params['terminal'];
    // A tapped card: its router params arrive as a JSON string.
    const card: Object | undefined = params['params'];
    if (t === undefined && typeof card === 'string') {
      try {
        t = (JSON.parse(card) as Record<string, string>)['terminal'];
      } catch (e) {
        t = '';
      }
    }
    if (typeof t === 'string' && t.length > 0 && t !== 'test') {
      Session.shared.pendingOpen = t;
      Session.shared.poke();
    }
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index', (err: BusinessError) => {
      if (err.code) {
        hilog.error(DOMAIN, 'ddup', 'loadContent failed: %{public}s', JSON.stringify(err));
        return;
      }
      // The soft keyboard shrinks the page instead of pushing it up, so the terminal
      // re-fits (fewer rows) and the title stays on screen.
      const win = windowStage.getMainWindowSync();
      win.getUIContext().setKeyboardAvoidMode(KeyboardAvoidMode.RESIZE);
      // Every page is dark: paint the strips behind the status bar and the gesture
      // bar the same colour (they were the default white), with light status icons.
      try {
        win.setWindowBackgroundColor(APP_BACKGROUND);
      } catch (e) {
        hilog.warn(DOMAIN, 'ddup', 'window background: %{public}s', (e as BusinessError).message);
      }
      win.setWindowSystemBarProperties({
        statusBarColor: APP_BACKGROUND,
        statusBarContentColor: '#FFFFFF',
        navigationBarColor: APP_BACKGROUND,
        navigationBarContentColor: '#FFFFFF'
      }).catch((e: BusinessError) => {
        hilog.warn(DOMAIN, 'ddup', 'system bars: %{public}d', e.code);
      });
      notificationManager.requestEnableNotification(this.context).catch((e: BusinessError) => {
        hilog.warn(DOMAIN, 'ddup', 'notifications not enabled: %{public}d', e.code);
      });
    });
  }

  onForeground(): void {
    this.shown = true;
    Session.shared.setForeground(true);
  }

  onBackground(): void {
    this.shown = false;
    Session.shared.setForeground(false);
  }

  onDestroy(): void {
    this.callee.off('answer');
  }
}
