// What a blocked agent is asking, read off its screen (terminal.read): the
// numbered choices TUIs draw (Claude Code permission dialogs and questions,
// Codex approvals, ...) or a y/n prompt. The terminal stays the authority:
// when nothing parses, the app shows the raw terminal and generic keys.

export class PromptOption {
  index: number = 0;
  label: string = '';
  detail: string = '';
  /// Typing your own answer (Claude's "Type something." / "tell Claude what to do").
  freeText: boolean = false;
}

export class PromptInfo {
  question: string = '';
  options: PromptOption[] = [];
  /// The highlighted option (1-based), 0 when the screen shows none.
  current: number = 0;
  /// "choice": move the highlight and press enter; "yesno": type y or n.
  kind: string = 'none';
}

const OPTION = /^\s*([❯›>▶►●]\s*)?(\d{1,2})[.)]\s+(.+?)\s*$/;
const BOX = /^[\s│┃|╭╮╰╯]+|[\s│┃|╭╮╰╯]+$/g;
const RULE = /^[\s─━—\-_=┄┈╌·]+$/;
const FREE = /type something|chat about this|tell (claude|codex|me) what to do|something else|输入|自己/i;
const YESNO = /[(\[]\s*y\s*\/\s*n\s*[)\]]/i;

function clean(line: string): string {
  return line.replace(BOX, '');
}

export function parsePrompt(screen: string): PromptInfo {
  const info = new PromptInfo();
  const lines = screen.split('\n').map((l: string) => clean(l));
  // The last run of numbered options on the screen, counted up from its bottom.
  let last = -1;
  for (let i = lines.length - 1; i >= 0; i--) {
    if (OPTION.test(lines[i])) {
      last = i;
      break;
    }
  }
  if (last >= 0) {
    const found: PromptOption[] = [];
    const at: number[] = [];
    let current = 0;
    let want = -1;
    for (let i = last; i >= 0 && i >= last - 60; i--) {
      const m = OPTION.exec(lines[i]);
      if (m === null) {
        continue;
      }
      const n = Number.parseInt(m[2]);
      if (want >= 0 && n !== want) {
        continue;
      }
      const o = new PromptOption();
      o.index = n;
      o.label = m[3].replace(/\s+\(esc\)$/i, '').trim();
      o.freeText = FREE.test(o.label);
      if (m[1]) {
        current = n;
      }
      found.unshift(o);
      at.unshift(i);
      if (n === 1) {
        break;
      }
      want = n - 1;
    }
    if (found.length >= 2 && found[0].index === 1) {
      // Detail lines: indented text under an option, up to the next one.
      for (let k = 0; k < found.length; k++) {
        const end = k + 1 < at.length ? at[k + 1] : Math.min(lines.length, at[k] + 4);
        const extra: string[] = [];
        for (let i = at[k] + 1; i < end; i++) {
          const t = lines[i].trim();
          if (t && !RULE.test(t) && !/enter to select|esc to cancel|tab\/arrow/i.test(t)) {
            extra.push(t);
          }
        }
        found[k].detail = extra.join(' ');
      }
      info.options = found;
      info.current = current;
      info.kind = 'choice';
      info.question = questionAbove(lines, at[0]);
      return info;
    }
  }
  // Options written on one line near the bottom: "继续吗？ 1) 继续  2) 停下  >".
  for (let i = lines.length - 1; i >= 0 && i >= lines.length - 8; i--) {
    const inline = inlineOptions(lines[i]);
    if (inline !== null) {
      inline.question = inline.question || questionAbove(lines, i);
      return inline;
    }
  }
  // A y/n prompt near the bottom.
  for (let i = lines.length - 1; i >= 0 && i >= lines.length - 8; i--) {
    if (YESNO.test(lines[i])) {
      const yes = new PromptOption();
      yes.index = 1;
      yes.label = 'Yes';
      const no = new PromptOption();
      no.index = 2;
      no.label = 'No';
      info.options = [yes, no];
      info.kind = 'yesno';
      info.question = lines[i].trim();
      return info;
    }
  }
  return info;
}

function inlineOptions(line: string): PromptInfo | null {
  const marks: number[] = [];
  const nums: number[] = [];
  const ends: number[] = [];
  const re = /(^|\s)(\d{1,2})[.)]\s+/g;
  let m = re.exec(line);
  while (m !== null) {
    marks.push(m.index + m[1].length);
    nums.push(Number.parseInt(m[2]));
    ends.push(m.index + m[0].length);
    m = re.exec(line);
  }
  if (nums.length < 2 || nums[0] !== 1) {
    return null;
  }
  for (let k = 1; k < nums.length; k++) {
    if (nums[k] !== nums[k - 1] + 1) {
      return null;
    }
  }
  const info = new PromptInfo();
  for (let k = 0; k < nums.length; k++) {
    const o = new PromptOption();
    o.index = nums[k];
    const end = k + 1 < marks.length ? marks[k + 1] : line.length;
    o.label = line.substring(ends[k], end).replace(/[\s>›❯:：]+$/, '').trim();
    o.freeText = FREE.test(o.label);
    if (!o.label) {
      return null;
    }
    info.options.push(o);
  }
  info.kind = 'choice';
  info.question = line.substring(0, marks[0]).trim();
  return info;
}

function questionAbove(lines: string[], first: number): string {
  for (let i = first - 1; i >= 0 && i >= first - 12; i--) {
    const t = lines[i].trim();
    if (!t || RULE.test(t) || /^[←→]|submit\s*→$/i.test(t)) {
      continue;
    }
    return t;
  }
  return '';
}

/// The keys that choose `target` (1-based): move the highlight there, then enter
/// (or type the number when the screen highlights nothing).
/// A free-text option is only highlighted, so the answer can be typed next.
export function keysFor(p: PromptInfo, target: number): string[] {
  if (p.kind === 'yesno') {
    return [target === 1 ? 'y' : 'n', 'enter'];
  }
  if (p.current === 0) {
    // No highlight on screen: a prompt that reads a typed number.
    return [`${target}`, 'enter'];
  }
  const keys: string[] = [];
  const from = p.current;
  const steps = target - from;
  for (let i = 0; i < Math.abs(steps); i++) {
    keys.push(steps > 0 ? 'down' : 'up');
  }
  const o = p.options.find((x: PromptOption) => x.index === target);
  if (o === undefined || !o.freeText) {
    keys.push('enter');
  }
  return keys;
}
