//! Follows the core's terminal events and decides when an agent needs a push
//! (docs/harmony/DESIGN.md §6): it went blocked/error, or finished a turn
//! nobody has seen (`herdr.seen`, docs/labels.md).

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    NeedsYou,
    Done,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub terminal: String,
    pub why: Why,
    pub title: String,
    pub body: String,
}

#[derive(Default, Debug)]
struct Term {
    state: Option<String>,
    status: Value,
    turn: u64,
    seen: Option<u64>,
    reported: bool,
    name: String,
    cwd: String,
}

impl Term {
    fn from_record(r: &Value) -> Term {
        let mut t = Term { turn: r["turn"].as_u64().unwrap_or(0), cwd: r["cwd"].as_str().unwrap_or_default().to_owned(), ..Default::default() };
        t.set_status(&r["status"]);
        t.set_labels(&r["labels"]);
        t.name = r["name"].as_str().filter(|s| !s.is_empty()).or(r["title"].as_str().filter(|s| !s.is_empty())).unwrap_or_default().to_owned();
        t
    }

    fn set_status(&mut self, s: &Value) {
        self.state = s["state"].as_str().map(str::to_owned);
        if self.state.is_some() {
            self.reported = true;
        }
        self.status = s.clone();
    }

    fn set_labels(&mut self, l: &Value) {
        self.seen = l["herdr.seen"].as_str().and_then(|s| s.parse().ok());
    }

    fn needs_you(&self) -> bool {
        matches!(self.state.as_deref(), Some("blocked" | "error"))
    }

    /// "claude · project" style label for a notification.
    fn label(&self, address: &str) -> String {
        let app = self.status["app"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).or_else(|| (!self.name.is_empty()).then(|| self.name.clone()));
        let folder = self.cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or_default();
        match (app, folder.is_empty()) {
            (Some(a), false) => format!("{a} · {folder}"),
            (Some(a), true) => a,
            (None, false) => format!("{address} · {folder}"),
            (None, true) => address.to_owned(),
        }
    }

    fn message(&self) -> Option<String> {
        ["msg", "title"].iter().find_map(|k| self.status[*k].as_str().filter(|s| !s.is_empty()).map(str::to_owned))
    }
}

#[derive(Default)]
pub struct Tracker {
    terms: HashMap<String, Term>,
    seq: u64,
}

impl Tracker {
    /// `events.subscribe`'s reply, or a `missed` snapshot: replaces everything, alerts nothing.
    pub fn snapshot(&mut self, seq: u64, terminals: &Value) {
        self.seq = seq;
        self.terms = terminals.as_array().into_iter().flatten().filter_map(|r| Some((r["address"].as_str()?.to_owned(), Term::from_record(r)))).collect();
    }

    /// One JSON line from the event stream.
    pub fn line(&mut self, line: &Value) -> Vec<Alert> {
        let p = &line["params"];
        match line["method"].as_str() {
            Some("missed") => {
                self.snapshot(p["seq"].as_u64().unwrap_or(self.seq), &p["terminals"]);
                vec![]
            }
            Some("event") => {
                if let Some(seq) = p["seq"].as_u64() {
                    if seq <= self.seq {
                        return vec![];
                    }
                    self.seq = seq;
                }
                self.event(p).into_iter().collect()
            }
            _ => vec![],
        }
    }

    fn event(&mut self, p: &Value) -> Option<Alert> {
        let kind = p["type"].as_str()?;
        let address = p["terminal"].as_str()?.to_owned();
        let data = &p["data"];
        match kind {
            "created" | "restarted" => {
                self.terms.insert(address, Term::from_record(data));
                None
            }
            "removed" => {
                self.terms.remove(&address);
                None
            }
            "finished" => {
                if let Some(t) = self.terms.get_mut(&address) {
                    t.state = None;
                }
                None
            }
            "labels" => {
                let t = self.terms.entry(address).or_default();
                t.set_labels(&data["labels"]);
                None
            }
            "title" => {
                let t = self.terms.entry(address).or_default();
                if let Some(title) = data["title"].as_str().filter(|s| !s.is_empty()) {
                    t.name = title.to_owned();
                }
                None
            }
            "status" => {
                let t = self.terms.entry(address.clone()).or_default();
                let was = t.needs_you();
                t.set_status(&data["status"]);
                (!was && t.needs_you()).then(|| {
                    let kind = t.status["kind"].as_str().map(|k| match k {
                        "permission" => "要你批准权限".to_owned(),
                        "question" => "在问你问题".to_owned(),
                        "auth" => "要你登录".to_owned(),
                        other => other.to_owned(),
                    });
                    let body = t.message().or(kind).unwrap_or_else(|| if t.state.as_deref() == Some("error") { "出错了".into() } else { "在等你".into() });
                    Alert { terminal: address.clone(), why: Why::NeedsYou, title: format!("需要你处理 · {}", t.label(&address)), body }
                })
            }
            "turn" => {
                let t = self.terms.entry(address.clone()).or_default();
                let turn = data["turn"].as_u64().unwrap_or(t.turn);
                let advanced = turn > t.turn;
                t.turn = turn;
                let unseen = t.seen.is_none_or(|s| turn > s);
                (advanced && t.reported && !t.needs_you() && unseen).then(|| Alert {
                    terminal: address.clone(),
                    why: Why::Done,
                    title: format!("已完成 · {}", t.label(&address)),
                    body: t.message().unwrap_or_else(|| "这一轮做完了".into()),
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: u64, kind: &str, t: &str, data: Value) -> Value {
        json!({"method": "event", "params": {"seq": seq, "type": kind, "terminal": t, "data": data}})
    }

    fn tracker() -> Tracker {
        let mut tr = Tracker::default();
        tr.snapshot(
            10,
            &json!([
                {"address": "t1", "turn": 2, "cwd": "/Users/a/proj", "status": {"state": "working", "app": "claude"}, "labels": {"herdr.seen": "2"}},
                {"address": "t2", "turn": 0, "cwd": "/tmp", "status": null, "labels": {}},
            ]),
        );
        tr
    }

    #[test]
    fn going_blocked_alerts_once() {
        let mut tr = tracker();
        let a = tr.line(&ev(11, "status", "t1", json!({"status": {"state": "blocked", "kind": "permission", "app": "claude"}})));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].why, Why::NeedsYou);
        assert_eq!(a[0].title, "需要你处理 · claude · proj");
        assert_eq!(a[0].body, "要你批准权限");
        assert!(tr.line(&ev(12, "status", "t1", json!({"status": {"state": "blocked", "msg": "still"}}))).is_empty());
    }

    #[test]
    fn a_finished_unseen_turn_alerts_and_a_seen_one_does_not() {
        let mut tr = tracker();
        tr.line(&ev(11, "status", "t1", json!({"status": {"state": "done", "msg": "Tests pass", "app": "claude"}})));
        let a = tr.line(&ev(12, "turn", "t1", json!({"turn": 3})));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].why, Why::Done);
        assert_eq!(a[0].body, "Tests pass");
        tr.line(&ev(13, "labels", "t1", json!({"labels": {"herdr.seen": "4"}})));
        assert!(tr.line(&ev(14, "turn", "t1", json!({"turn": 4}))).is_empty(), "already seen on another screen");
    }

    #[test]
    fn non_agents_old_events_and_snapshots_stay_quiet() {
        let mut tr = tracker();
        assert!(tr.line(&ev(11, "turn", "t2", json!({"turn": 1}))).is_empty(), "never reported status");
        assert!(tr.line(&ev(5, "status", "t1", json!({"status": {"state": "blocked"}}))).is_empty(), "seq not above");
        tr.line(&json!({"method": "missed", "params": {"seq": 20, "terminals": [{"address": "t1", "turn": 9, "status": {"state": "blocked"}, "labels": {}}]}}));
        assert!(tr.line(&ev(21, "status", "t1", json!({"status": {"state": "blocked"}}))).is_empty(), "was already blocked in the snapshot");
    }
}
