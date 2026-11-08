import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { notificationManager } from '@kit.NotificationKit';
import { pushService } from '@kit.PushKit';
import { KeyboardAvoidMode, window } from '@kit.ArkUI';
import { Session } from '../core/Session';

const DOMAIN = 0xd0d0;

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
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
    const t = params['terminal'];
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
      windowStage.getMainWindowSync().getUIContext().setKeyboardAvoidMode(KeyboardAvoidMode.RESIZE);
      notificationManager.requestEnableNotification(this.context).catch((e: BusinessError) => {
        hilog.warn(DOMAIN, 'ddup', 'notifications not enabled: %{public}d', e.code);
      });
    });
  }

  onForeground(): void {
    Session.shared.setForeground(true);
  }

  onBackground(): void {
    Session.shared.setForeground(false);
  }
}
