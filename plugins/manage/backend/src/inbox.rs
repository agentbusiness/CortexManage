//! What waits on a person: the open asks, each with why it waits and what a yes or a no does, the
//! things that go by themselves and can still be stopped, and `manage.decide`.
//!
//! An ask is stored as a `Decision` that is still open; answering it closes it with who answered
//! and how. The words of an ask come from the chat flow (`why`), written from the facts; until it
//! answers, and when it cannot, they are the templates of `data/texts.toml`.

use crate::service::Manager;
use crate::store::AskRec;
use crate::{job, packages, pipeline};
use cc_protocol::contracts::manage::{Ask, DecideParams, Going, Inbox, WhyAnswer, flow, kinds, stage};
use cc_protocol::contracts::now_ms;
use cc_protocol::{RpcError, codes};
use serde_json::json;
use std::sync::Arc;

/// A new ask, before it has an id.
#[derive(Default)]
pub struct NewAsk {
    /// approve_plan | approve_landing | stuck | contradiction | blocked_walk.
    pub kind: String,
    pub repo: String,
    pub request: String,
    pub other_request: String,
    pub package: String,
    pub title: String,
    /// The question in one line.
    pub subject: String,
    /// The fact that made it, for the template's `{detail}` and for the model.
    pub detail: String,
    pub actions: Vec<String>,
    pub attempt: u32,
    pub resume: String,
}

pub fn actions(list: &[&str]) -> Vec<String> {
    list.iter().map(|action| (*action).to_owned()).collect()
}

fn sessions(manager: &Manager, rec: &AskRec) -> Vec<String> {
    manager.store.read(|data| {
        let mut out: Vec<String> = match data.packages.get(&rec.ask.package) {
            Some(package) => data.sessions_of(package),
            None => Vec::new(),
        };
        for id in [&rec.ask.request, &rec.other_request] {
            if let Some(request) = data.requests.get(id)
                && !out.contains(&request.r.session_id)
            {
                out.push(request.r.session_id.clone());
            }
        }
        out
    })
}

/// Open an ask. It is in the inbox at once with the template's words; the chat flow's own words
/// replace them when it answers.
pub async fn raise(manager: &Arc<Manager>, new: NewAsk) -> String {
    let id = manager.store.next_ask_id();
    let detail = [("detail", new.detail.as_str())];
    let ask = Ask {
        id: id.clone(),
        kind: new.kind.clone(),
        repo: new.repo.clone(),
        request: new.request.clone(),
        package: new.package.clone(),
        title: new.title.clone(),
        subject: new.subject.clone(),
        why: manager.t(&format!("why_{}", new.kind), &detail),
        if_yes: manager.t(&format!("yes_{}", new.kind), &detail),
        if_no: manager.t(&format!("no_{}", new.kind), &detail),
        actions: new.actions.clone(),
        attempt: new.attempt,
        since_ms: now_ms(),
    };
    let rec = AskRec { ask: ask.clone(), open: true, other_request: new.other_request.clone(), resume: new.resume.clone(), ..AskRec::default() };
    manager.store.put_ask(rec.clone());
    manager.entry_all(&sessions(manager, &rec), kinds::ASK, json!({"ask": ask})).await;
    manager.changed("inbox", &id, &new.repo);
    if !new.package.is_empty() {
        manager.changed("package", &new.package, &new.repo);
    }
    if !new.request.is_empty() {
        manager.changed("request", &new.request, &new.repo);
    }
    // The model's words, when it has them. Nothing is decided from them.
    let facts = json!({
        "kind": new.kind, "title": new.title, "subject": new.subject, "detail": new.detail, "repo": new.repo,
        "request": new.request, "package": new.package, "actions": new.actions,
    });
    let (manager, ask_id, repo) = (manager.clone(), id.clone(), new.repo);
    tokio::spawn(async move {
        match manager.peer::<WhyAnswer>(flow::WHY, json!({"facts": facts})).await {
            Ok(answer) if !answer.why.trim().is_empty() => {
                let rec = manager.store.read(|data| data.asks.get(&ask_id).filter(|rec| rec.open).cloned());
                if let Some(mut rec) = rec {
                    rec.ask.why = answer.why.trim().to_owned();
                    if !answer.if_yes.trim().is_empty() {
                        rec.ask.if_yes = answer.if_yes.trim().to_owned();
                    }
                    if !answer.if_no.trim().is_empty() {
                        rec.ask.if_no = answer.if_no.trim().to_owned();
                    }
                    manager.store.put_ask(rec);
                    manager.changed("inbox", &ask_id, &repo);
                }
            }
            Ok(_) => {}
            Err(error) => tracing::debug!(ask = %ask_id, code = %error.code, "the chat flow gave no words for this ask; the template's stay"),
        }
    });
    id
}

/// Close every open ask `which` picks, without a person (the thing it asked about changed).
pub fn close_where(manager: &Manager, by: &str, action: &str, which: impl Fn(&AskRec) -> bool) -> usize {
    let closing: Vec<AskRec> = manager.store.read(|data| data.open_asks().filter(|rec| which(rec)).cloned().collect());
    let count = closing.len();
    for mut rec in closing {
        rec.open = false;
        rec.by = by.to_owned();
        rec.action = action.to_owned();
        rec.answered_ms = now_ms();
        manager.store.put_ask(rec);
    }
    if count > 0 {
        manager.changed("inbox", "", "");
    }
    count
}

/// `manage.decide`: a person answers an open ask.
pub async fn decide(manager: &Arc<Manager>, params: &DecideParams) -> Result<(), RpcError> {
    let rec = manager.store.read(|data| data.asks.get(&params.id).cloned()).ok_or_else(|| RpcError::not_found(format!("no decision `{}` is wanted", params.id)))?;
    if !rec.open {
        return Err(RpcError::new(codes::CONFLICT, manager.t("decide_closed", &[])));
    }
    let package = manager.store.read(|data| data.packages.get(&rec.ask.package).cloned());
    let current = package.as_ref().map(|package| package.attempt).unwrap_or(rec.ask.attempt);
    if params.attempt != rec.ask.attempt || current != rec.ask.attempt {
        // The work moved on since this was asked: an answer to the old attempt decides nothing.
        if current != rec.ask.attempt {
            close_where(manager, "policy", "superseded", |ask| ask.ask.id == rec.ask.id);
        }
        return Err(RpcError::new(codes::CONFLICT, manager.t("decide_stale", &[])));
    }
    if !rec.ask.actions.contains(&params.action) {
        return Err(RpcError::invalid_params(format!("`{}` is not one of this decision's answers ({})", params.action, rec.ask.actions.join(", "))));
    }
    let mut closed = rec.clone();
    closed.open = false;
    closed.by = "person".into();
    closed.action = params.action.clone();
    closed.note = params.note.trim().to_owned();
    closed.answered_ms = now_ms();
    manager.store.put_ask(closed.clone());
    manager.entry_all(&sessions(manager, &rec), kinds::ASK, json!({"ask_id": rec.ask.id, "action": params.action, "by": "person", "note": closed.note})).await;
    act(manager, &closed).await;
    manager.changed("inbox", &rec.ask.id, &rec.ask.repo);
    if !rec.ask.package.is_empty() {
        manager.changed("package", &rec.ask.package, &rec.ask.repo);
    }
    Ok(())
}

/// What an answer sets going.
async fn act(manager: &Arc<Manager>, rec: &AskRec) {
    let ask = &rec.ask;
    let (action, note) = (rec.action.as_str(), rec.note.as_str());
    let package = manager.store.read(|data| data.packages.get(&ask.package).cloned());
    match ask.kind.as_str() {
        "contradiction" => {
            // `first` keeps the older request, `second` the newer; the other one ends.
            let (kept, dropped) = if action == "second" { (&rec.other_request, &ask.request) } else { (&ask.request, &rec.other_request) };
            packages::take_out(manager, dropped);
            pipeline::decline(manager, dropped, &manager.t("declined_contradiction", &[("kept", kept)])).await;
            manager.look_now();
        }
        "approve_plan" => {
            let Some(package) = package else { return };
            match action {
                "approve" => {
                    manager.store.update_package(&package.id, |package| {
                        package.hold_until_ms = now_ms();
                        package.stopped = false;
                    });
                    manager.look_now();
                }
                "send_back" => {
                    for request in &package.requests {
                        pipeline::again(manager, request, note, true);
                    }
                }
                _ => packages::end(manager, &package.id, &reason(manager, note)).await,
            }
        }
        _ => {
            let Some(package) = package else { return };
            if matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING) {
                job::tell(manager, &package.id, job::Msg::Decided { ask: Box::new(rec.clone()) });
                return;
            }
            // The job never started (no runner, or the workspace could not be made).
            match action {
                "rerun" => manager.look_now(),
                "take_over" => {
                    manager.store.update_package(&package.id, |package| package.stopped = true);
                }
                _ => packages::end(manager, &package.id, &reason(manager, note)).await,
            }
        }
    }
}

fn reason(manager: &Manager, note: &str) -> String {
    if note.is_empty() { manager.t("declined_by_you", &[]) } else { manager.t("declined_by_you_note", &[("note", note)]) }
}

/// `21:13` in this computer's time.
fn clock(ms: u64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms as i64).single() {
        Some(time) => time.format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// `manage.inbox.list`.
pub fn list(manager: &Manager) -> Inbox {
    manager.store.read(|data| {
        let mut asks: Vec<Ask> = data.open_asks().map(|rec| rec.ask.clone()).collect();
        asks.sort_by_key(|ask| ask.since_ms);
        let title = |package: &cc_protocol::contracts::manage::Package| {
            let first = package.requests.first().and_then(|id| data.requests.get(id)).map(|rec| rec.r.title.clone()).unwrap_or_default();
            match package.requests.len() {
                0 | 1 => first,
                count => manager.t("title_bundle", &[("title", &first), ("more", &(count - 1).to_string())]),
            }
        };
        let mut going: Vec<Going> = Vec::new();
        let (mut building, mut held) = (0, 0);
        for package in data.packages.values() {
            let running = matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING);
            if running {
                building += 1;
            } else if package.stage == stage::HELD {
                held += 1;
            }
            let blocked = !data.asks_of(package).is_empty();
            let line = if running && package.urgent {
                manager.t("going_urgent_building", &[])
            } else if package.stage == stage::HELD && !package.stopped && !blocked && package.urgent {
                manager.t("going_urgent_held", &[])
            } else if package.stage == stage::HELD && !package.stopped && !blocked && package.hold_until_ms != 0 {
                manager.t("going_builds", &[("time", &clock(package.hold_until_ms))])
            } else {
                continue;
            };
            going.push(Going { package: package.id.clone(), repo: package.repo.clone(), title: title(package), line, hold_until_ms: package.hold_until_ms, urgent: package.urgent });
        }
        going.sort_by_key(|item| (!item.urgent, item.hold_until_ms));
        let mut parts: Vec<String> = Vec::new();
        // What is in motion besides the asks: the band's button already counts those, and with
        // nothing in motion the line is empty.
        for (count, key) in [(building, "count_building"), (held, "count_held")] {
            if count > 0 {
                parts.push(manager.t(key, &[("count", &count.to_string())]));
            }
        }
        let count_line = parts.join(" · ");
        Inbox { asks, going, count_line }
    })
}
