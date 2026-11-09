//! The home-screen card (docs/harmony/DESIGN.md §6b): which agents wait for
//! you, and the first one's choices as buttons. The bridge reads that agent's
//! screen (the same parse as the app's ets/core/Prompt.ets, minus the key
//! logic: a button only names an option, and the app re-reads the screen
//! before it answers) and sends Push Kit card-refresh messages.

use serde_json::{Map, Value, json};

/// Keys the card binds (mobile/harmony entry/src/main/ets/widget/pages/WidgetCard.ets).
pub const KEYS: [&str; 11] = ["n", "head", "q", "o1", "o2", "o3", "i1", "i2", "i3", "addr", "note"];

/// Push Kit's daily cap per card is 50 for tool/efficiency apps; keep a margin.
pub const DAILY_CAP: u32 = 45;

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub index: u32,
    pub label: String,
    pub free: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Prompt {
    pub question: String,
    pub options: Vec<Choice>,
}

/// An agent that needs you, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Waiting {
    pub address: String,
    pub label: String,
    pub message: String,
}

const MARKS: &[char] = &['❯', '›', '>', '▶', '►', '●'];
const BOX: &[char] = &['│', '┃', '|', '╭', '╮', '╰', '╯'];

fn clean(line: &str) -> &str {
    line.trim_matches(|c: char| c.is_whitespace() || BOX.contains(&c))
}

fn is_rule(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_whitespace() || "─━—-_=┄┈╌·".contains(c))
}

fn is_free(label: &str) -> bool {
    let l = label.to_lowercase();
    ["type something", "chat about this", "tell claude what to do", "tell codex what to do", "tell me what to do", "something else", "输入", "自己"]
        .iter()
        .any(|k| l.contains(k))
}

/// "❯ 2. Yes, and ..." → (2, "Yes, and ...", highlighted).
fn option_line(line: &str) -> Option<(u32, String)> {
    let mut s = line.trim_start();
    if let Some(c) = s.chars().next()
        && MARKS.contains(&c)
    {
        s = s[c.len_utf8()..].trim_start();
    }
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let n: u32 = s[..digits].parse().ok()?;
    let rest = &s[digits..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let label = rest.trim();
    let label = label.strip_suffix("(esc)").map(str::trim_end).unwrap_or(label);
    (!label.is_empty()).then(|| (n, label.to_owned()))
}

fn question_above(lines: &[&str], first: usize) -> String {
    for i in (first.saturating_sub(12)..first).rev() {
        let t = lines[i].trim();
        if t.is_empty() || is_rule(t) || t.starts_with(['←', '→']) || t.to_lowercase().ends_with("submit →") {
            continue;
        }
        return t.to_owned();
    }
    String::new()
}

/// Options on one line: "继续吗？ 1) 继续  2) 停下  >".
fn inline_options(line: &str) -> Option<Prompt> {
    let b = line.as_bytes();
    let mut marks = Vec::new(); // (start of "N.", start of label, N)
    let mut i = 0;
    while i < b.len() {
        let at_word = i == 0 || b[i - 1].is_ascii_whitespace();
        if at_word && b[i].is_ascii_digit() {
            let mut j = i;
            while j < b.len() && j - i < 2 && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && (b[j] == b'.' || b[j] == b')') && j + 1 < b.len() && b[j + 1].is_ascii_whitespace() {
                let mut k = j + 1;
                while k < b.len() && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                marks.push((i, k, line[i..j].parse::<u32>().ok()?));
                i = k;
                continue;
            }
        }
        i += 1;
    }
    if marks.len() < 2 || marks[0].2 != 1 || marks.windows(2).any(|w| w[1].2 != w[0].2 + 1) {
        return None;
    }
    let mut p = Prompt { question: line[..marks[0].0].trim().to_owned(), options: Vec::new() };
    for (k, m) in marks.iter().enumerate() {
        let end = marks.get(k + 1).map_or(line.len(), |n| n.0);
        let label = line[m.1..end].trim_end_matches(|c: char| c.is_whitespace() || ">›❯:：".contains(c)).trim();
        if label.is_empty() {
            return None;
        }
        p.options.push(Choice { index: m.2, label: label.to_owned(), free: is_free(label) });
    }
    Some(p)
}

fn yes_no(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    ["(y/n)", "[y/n]", "(y/n]", "[y/n)"].iter().any(|k| t.contains(k))
}

/// What the screen asks, or None when nothing parses.
pub fn parse(screen: &str) -> Option<Prompt> {
    let lines: Vec<&str> = screen.split('\n').map(clean).collect();
    if let Some(last) = lines.iter().rposition(|l| option_line(l).is_some()) {
        let mut found: Vec<(usize, u32, String)> = Vec::new();
        let mut want: Option<u32> = None;
        for i in (last.saturating_sub(60)..=last).rev() {
            let Some((n, label)) = option_line(lines[i]) else { continue };
            if want.is_some_and(|w| w != n) {
                continue;
            }
            found.insert(0, (i, n, label));
            if n == 1 {
                break;
            }
            want = Some(n - 1);
        }
        if found.len() >= 2 && found[0].1 == 1 {
            let question = question_above(&lines, found[0].0);
            let options = found.into_iter().map(|(_, n, label)| Choice { index: n, free: is_free(&label), label }).collect();
            return Some(Prompt { question, options });
        }
    }
    let tail = lines.len().saturating_sub(8);
    for i in (tail..lines.len()).rev() {
        if let Some(mut p) = inline_options(lines[i]) {
            if p.question.is_empty() {
                p.question = question_above(&lines, i);
            }
            return Some(p);
        }
    }
    for i in (tail..lines.len()).rev() {
        if yes_no(lines[i]) {
            let options = vec![Choice { index: 1, label: "Yes".into(), free: false }, Choice { index: 2, label: "No".into(), free: false }];
            return Some(Prompt { question: lines[i].trim().to_owned(), options });
        }
    }
    None
}

/// A button-sized name for the usual agent answers; anything else as written.
pub fn short(label: &str) -> String {
    let l = label.trim().to_lowercase();
    let s = if l.starts_with("yes, and don't ask again") || l.starts_with("yes, don't ask again") {
        "允许，不再问"
    } else if l.starts_with("yes, allow all") || l.starts_with("yes, and auto") {
        "全部允许"
    } else if l == "yes" || l.starts_with("yes,") || l.starts_with("yes ") {
        "允许"
    } else if l == "no" || l.starts_with("no,") || l.starts_with("no ") {
        "拒绝"
    } else {
        return label.trim().to_owned();
    };
    s.to_owned()
}

/// The card's values (all strings, every key present so old values get cleared).
pub fn data(waiting: &[Waiting], head: Option<&Prompt>) -> Value {
    let mut m = Map::new();
    for k in KEYS {
        m.insert(k.into(), json!(""));
    }
    m.insert("n".into(), json!(waiting.len().to_string()));
    let Some(w) = waiting.first() else {
        m.insert("note".into(), json!("没有在等你的任务"));
        return Value::Object(m);
    };
    m.insert("head".into(), json!(w.label));
    m.insert("addr".into(), json!(w.address));
    let question = head.map(|p| p.question.clone()).filter(|q| !q.is_empty()).unwrap_or_else(|| w.message.clone());
    m.insert("q".into(), json!(question));
    if let Some(p) = head {
        for (k, c) in p.options.iter().filter(|c| !c.free).take(3).enumerate() {
            m.insert(format!("o{}", k + 1), json!(short(&c.label)));
            m.insert(format!("i{}", k + 1), json!(c.index.to_string()));
        }
    }
    let rest = waiting.len() - 1;
    let note = match (rest, head.is_some()) {
        (0, true) => String::new(),
        (0, false) => "点开回答".into(),
        (r, _) => format!("还有 {r} 个在等"),
    };
    m.insert("note".into(), json!(note));
    Value::Object(m)
}

/// Card refreshes sent today, per card (China time, as Push Kit counts).
#[derive(Default, Debug)]
pub struct Budget {
    day: u64,
    used: u32,
}

impl Budget {
    pub fn take(&mut self, now_ms: u64) -> bool {
        let day = (now_ms / 1000 + 8 * 3600) / 86_400;
        if day != self.day {
            self.day = day;
            self.used = 0;
        }
        if self.used >= DAILY_CAP {
            return false;
        }
        self.used += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERMISSION: &str = include_str!("../../tests/harmony-logic/screens/claude-permission.txt");
    const QUESTION: &str = include_str!("../../tests/harmony-logic/screens/claude-question.txt");
    const YESNO: &str = include_str!("../../tests/harmony-logic/screens/yesno.txt");

    #[test]
    fn same_prompts_as_the_app() {
        let p = parse(PERMISSION).unwrap();
        assert_eq!(p.question, "Do you want to proceed?");
        assert_eq!(p.options.len(), 3);
        assert!(p.options[1].label.starts_with("Yes, and don't ask again"));
        assert!(p.options[2].free && !p.options[2].label.contains("(esc)"));

        let q = parse(QUESTION).unwrap();
        assert_eq!(q.options.len(), 6);
        assert!(q.question.starts_with("界面改版先拿哪个页面试点"), "{}", q.question);
        assert!(q.options[4].free && !q.options[0].free);

        let yn = parse(YESNO).unwrap();
        assert_eq!(yn.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["Yes", "No"]);

        let inl = parse("第 1 轮完成\n要继续下一轮吗？ 1) 继续  2) 停下  > ").unwrap();
        assert_eq!(inl.question, "要继续下一轮吗？");
        assert_eq!(inl.options[1].label, "停下");

        assert!(parse("just some\noutput 1. not a menu\n").is_none());
    }

    #[test]
    fn card_values() {
        let w = vec![
            Waiting { address: "t3".into(), label: "claude · proj".into(), message: "要你批准权限".into() },
            Waiting { address: "t5".into(), label: "codex · api".into(), message: "在等你".into() },
        ];
        let d = data(&w, parse(PERMISSION).as_ref());
        assert_eq!(d["n"], "2");
        assert_eq!(d["addr"], "t3");
        assert_eq!(d["q"], "Do you want to proceed?");
        assert_eq!((d["o1"].as_str(), d["o2"].as_str(), d["o3"].as_str()), (Some("允许"), Some("允许，不再问"), Some("")));
        assert_eq!((d["i1"].as_str(), d["i2"].as_str()), (Some("1"), Some("2")));
        assert_eq!(d["note"], "还有 1 个在等");

        let unread = data(&w[..1], None);
        assert_eq!((unread["q"].as_str(), unread["o1"].as_str(), unread["note"].as_str()), (Some("要你批准权限"), Some(""), Some("点开回答")));

        let none = data(&[], None);
        assert_eq!((none["n"].as_str(), none["addr"].as_str()), (Some("0"), Some("")));
        assert!(KEYS.iter().all(|k| none.get(*k).is_some()));
    }

    #[test]
    fn daily_budget() {
        let mut b = Budget::default();
        let day = 1_700_000_000_000;
        assert!((0..DAILY_CAP).all(|_| b.take(day)));
        assert!(!b.take(day));
        assert!(b.take(day + 86_400_000));
    }
}
