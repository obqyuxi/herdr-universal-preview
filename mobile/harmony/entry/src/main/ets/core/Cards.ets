// The home-screen card (docs/harmony/DESIGN.md §6b): which agents wait for you,
// and the newest one's choices as buttons. While the app runs it fills the card
// itself; otherwise the bridge sends Push Kit card-refresh messages with the same
// keys (bridge/src/card.rs). The card's form ids are kept in preferences, which
// the card's own process (EntryFormAbility) writes when a card is placed or removed.
import { common } from '@kit.AbilityKit';
import { formBindingData, formProvider } from '@kit.FormKit';
import { preferences } from '@kit.ArkData';
import { Agent, Group } from './Agents';
import { PromptInfo, PromptOption } from './Prompt';

export const CARD_KEYS: string[] = ['n', 'head', 'q', 'o1', 'o2', 'o3', 'i1', 'i2', 'i3', 'addr', 'note'];

const STORE = 'cards';
const IDS = 'ids';
const LAST = 'last';

/// What the card's buttons say for the usual agent answers (the bridge does the same).
export function shortLabel(label: string): string {
  const l = label.trim().toLowerCase();
  if (l.startsWith('yes, and don\'t ask again') || l.startsWith('yes, don\'t ask again')) {
    return '允许，不再问';
  }
  if (l.startsWith('yes, allow all') || l.startsWith('yes, and auto')) {
    return '全部允许';
  }
  if (l === 'yes' || l.startsWith('yes,') || l.startsWith('yes ')) {
    return '允许';
  }
  if (l === 'no' || l.startsWith('no,') || l.startsWith('no ')) {
    return '拒绝';
  }
  return label.trim();
}

function emptyValues(): Record<string, string> {
  const v: Record<string, string> = {};
  for (const k of CARD_KEYS) {
    v[k] = '';
  }
  return v;
}

/// The card's values for these agents (newest waiting first, as the bridge orders them).
export function cardValues(agents: Agent[]): Record<string, string> {
  const v = emptyValues();
  const waiting = agents.filter((a: Agent) => a.group() === Group.NeedsYou)
    .sort((x: Agent, y: Agent) => y.statusMs - x.statusMs);
  v['n'] = `${waiting.length}`;
  if (waiting.length === 0) {
    v['note'] = '没有在等你的任务';
    return v;
  }
  const a = waiting[0];
  const folder = a.folder();
  v['head'] = folder ? `${a.label()} · ${folder}` : a.label();
  v['addr'] = a.address;
  const p: PromptInfo | null = a.prompt;
  v['q'] = p !== null && p.question ? p.question : a.detail();
  if (p !== null) {
    const shown = p.options.filter((o: PromptOption) => !o.freeText).slice(0, 3);
    shown.forEach((o: PromptOption, k: number) => {
      v[`o${k + 1}`] = shortLabel(o.label);
      v[`i${k + 1}`] = `${o.index}`;
    });
  }
  const rest = waiting.length - 1;
  v['note'] = rest > 0 ? `还有 ${rest} 个在等` : p === null ? '点开回答' : '';
  return v;
}

function store(ctx: Context): preferences.Preferences {
  // The card process writes too: drop this process's cached copy first.
  preferences.removePreferencesFromCacheSync(ctx, { name: STORE });
  return preferences.getPreferencesSync(ctx, { name: STORE });
}

export class Cards {
  private static ctx: Context | null = null;
  private static lastSent: string = '';
  private static timer: number = -1;

  static init(ctx: common.UIAbilityContext): void {
    Cards.ctx = ctx;
  }

  static ids(ctx?: Context): string[] {
    const c = ctx ?? Cards.ctx;
    if (c === null || c === undefined) {
      return [];
    }
    return store(c).getSync(IDS, []) as string[];
  }

  /// A card was placed (card process): remember it; returns what to show first.
  static added(ctx: Context, id: string): Record<string, string> {
    const s = store(ctx);
    const ids = (s.getSync(IDS, []) as string[]).filter((x: string) => x !== id);
    ids.push(id);
    s.putSync(IDS, ids);
    s.flushSync();
    const last = s.getSync(LAST, '') as string;
    const v = last ? JSON.parse(last) as Record<string, string> : emptyValues();
    if (!last) {
      v['n'] = '0';
      v['note'] = '打开码上后显示在等你的任务';
    }
    return v;
  }

  static removed(ctx: Context, id: string): void {
    const s = store(ctx);
    s.putSync(IDS, (s.getSync(IDS, []) as string[]).filter((x: string) => x !== id));
    s.flushSync();
  }

  /// Show these values on every card now (and on cards placed later).
  static show(v: Record<string, string>): void {
    const ctx = Cards.ctx;
    if (ctx === null) {
      return;
    }
    const json = JSON.stringify(v);
    if (json === Cards.lastSent) {
      return;
    }
    Cards.lastSent = json;
    const s = store(ctx);
    s.putSync(LAST, json);
    s.flushSync();
    for (const id of s.getSync(IDS, []) as string[]) {
      formProvider.updateForm(id, formBindingData.createFormBindingData(v)).catch((): void => {});
    }
  }

  /// Change a few values on one card (a button's progress), leaving the rest.
  static patch(id: string, v: Record<string, string>): void {
    formProvider.updateForm(id, formBindingData.createFormBindingData(v)).catch((): void => {});
  }

  /// The agents changed: update the cards shortly (coalesces bursts of events).
  static soon(agents: () => Agent[]): void {
    if (Cards.timer >= 0) {
      return;
    }
    Cards.timer = setTimeout((): void => {
      Cards.timer = -1;
      Cards.show(cardValues(agents()));
    }, 400);
  }
}
