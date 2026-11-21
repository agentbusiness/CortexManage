//! The board: every open request in one of four columns (incoming, waiting, coding, done) with the
//! facts of its one line, and the decisions that wait on a person. The column is counted from
//! facts, never set by hand: a person moves work by starting, pausing, joining or splitting it.

use crate::service::Manager;
use crate::store::{Data, ReqRec};
use crate::words;
use cc_protocol::contracts::manage::{Ask, BoardItem, column, stage};
use cc_protocol::contracts::now_ms;
use cc_protocol::contracts::train::{TrainView, methods as train_methods};
use cc_protocol::codes;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// What was done stays on the board this long.
const DONE_DAYS_MS: u64 = 7 * 24 * 3_600_000;

/// `manage.board.items`.
pub async fn items(manager: &Manager) -> Value {
    let now = now_ms();
    // When each soaking package is released: the repository's first train after its soak ends.
    let soaking: Vec<(String, String, u64)> = manager.store.read(|data| {
        data.packages
            .values()
            .filter(|package| package.soak_until_ms > 0 && matches!(package.stage.as_str(), stage::LANDED | stage::SOAKING))
            .map(|package| (package.id.clone(), package.repo.clone(), package.soak_until_ms))
            .collect()
    });
    let mut release: HashMap<String, u64> = HashMap::new();
    for (package, repo, soak_until) in soaking {
        release.insert(package, train_after(manager, &repo, soak_until.max(now)).await);
    }
    // Packages that wait for a CortexCode that did not take them.
    let packages: Vec<String> = manager.store.read(|data| data.packages.keys().cloned().collect());
    let no_builder: HashSet<String> = packages.into_iter().filter(|id| manager.waits_on(&format!("runner:{id}"))).collect();
    let (items, asks): (Vec<BoardItem>, Vec<Ask>) = manager.store.read(|data| {
        let items = data.requests.values().filter_map(|rec| item(data, rec, &release, now, &no_builder)).collect();
        let asks = data.open_asks().filter(|rec| crate::inbox::asks_person(&rec.ask.kind)).map(|rec| rec.ask.clone()).collect();
        (items, asks)
    });
    json!({"items": items, "asks": asks})
}

fn item(data: &Data, rec: &ReqRec, release: &HashMap<String, u64>, now: u64, no_builder: &HashSet<String>) -> Option<BoardItem> {
    let r = &rec.r;
    if rec.hidden {
        return None;
    }
    let released = r.stage == stage::RELEASED || r.ended_as == "released";
    if !r.ended_as.is_empty() && !released {
        // Declined, withdrawn or merged: not on the board (a merged one counts on the one it joined).
        return None;
    }
    let reached = |stage_name: &str| rec.stages.iter().rev().find(|(name, _)| name == stage_name).map(|(_, ms)| *ms).unwrap_or(0);
    let released_ms = if released { reached(stage::RELEASED).max(r.updated_ms) } else { 0 };
    if released && now.saturating_sub(released_ms) > DONE_DAYS_MS {
        return None;
    }
    let package = data.packages.get(&r.package);
    let paused = package.is_some_and(|package| package.stopped) || data.hold;
    let running = matches!(r.stage.as_str(), stage::BUILDING | stage::PROVING);
    // The person started it: it shows as coding until it builds, and says what it waits on.
    let started = r.stage == stage::HELD && rec.start_wish == "now" && package.is_some_and(|package| !package.stopped);
    let column = match r.stage.as_str() {
        stage::HELD if started => column::CODING,
        stage::HELD => column::WAITING,
        stage::BUILDING | stage::PROVING if package.is_some_and(|package| package.stopped) => column::WAITING,
        stage::BUILDING | stage::PROVING => column::CODING,
        stage::RELEASED => column::SHIPPED,
        _ if released => column::SHIPPED,
        stage::LANDED | stage::SOAKING => column::DONE,
        // Still being planned: it is where the person put it, and starts or waits as they said.
        _ if rec.start_wish == "now" => column::CODING,
        _ if rec.start_wish == "wait" => column::WAITING,
        _ => column::INCOMING,
    };
    let queued = match package {
        Some(package) if started => {
            let busy = data.packages.values().any(|other| other.id != package.id && other.repo == package.repo && matches!(other.stage.as_str(), stage::BUILDING | stage::PROVING));
            if busy {
                "after"
            } else if no_builder.contains(&package.id) {
                "builder"
            } else {
                "next"
            }
        }
        _ => "",
    };
    let together = package.filter(|package| package.requests.len() > 1).map(|package| package.id.clone()).unwrap_or_default();
    let merged: Vec<&ReqRec> = data.requests.values().filter(|other| other.r.merged_into == r.id).collect();
    let mut reporters: Vec<String> = Vec::new();
    for other in merged.iter().map(|other| sender(other)).chain(std::iter::once(sender(rec))) {
        if !other.is_empty() && !reporters.contains(&other) {
            reporters.push(other);
        }
    }
    let starts_ms = match package {
        Some(package) if r.stage == stage::HELD && !package.stopped => package.hold_until_ms,
        _ => 0,
    };
    let strip = package.and_then(|package| data.strips.get(&package.id));
    let steps = data.kept(&r.id).len() as u32;
    let soak_until_ms = package.map(|package| package.soak_until_ms).unwrap_or(0);
    let release_ms = if released { 0 } else { release.get(&r.package).copied().unwrap_or(0) };
    Some(BoardItem {
        request: r.clone(),
        word: words::request(data, rec),
        column: column.to_owned(),
        paused: paused && (r.stage == stage::HELD || running),
        starts_ms,
        step: strip.map(|strip| strip.task).unwrap_or(0),
        steps: strip.map(|strip| strip.tasks).filter(|tasks| *tasks > 0).unwrap_or(steps),
        checking: r.stage == stage::PROVING,
        queued: queued.to_owned(),
        soak_until_ms,
        release_ms,
        released_ms,
        together,
        reports: merged.len() as u32,
        reporters,
    })
}

/// Who sent a request, for the board: an app's name, or nothing for the person themselves.
fn sender(rec: &ReqRec) -> String {
    if rec.r.from.kind == "app" { rec.r.from.name.clone() } else { String::new() }
}

/// The repository's first train after `after_ms`, as the release train's schedule says. 0 when
/// the train plugin does not answer: the board then names no release time.
async fn train_after(manager: &Manager, repo: &str, after_ms: u64) -> u64 {
    match manager.peer::<TrainView>(train_methods::GET, json!({"repo": repo, "after_ms": after_ms})).await {
        Ok(view) => view.next_ms,
        Err(error) => {
            if error.code != codes::UNAVAILABLE {
                tracing::debug!(repo, error = %error.message, "the next train was not read");
            }
            0
        }
    }
}
