//! The ledger of a repository: an append-only stream in the graph (`manage.ledger.<repo>`) of what
//! happened to its requests and why, and in which version the work shipped. One record when a
//! request's work lands on the main branch, one when it is released, and one for every other end
//! or turn a person would want to know about: merged into other work, closed as already done,
//! declined, withdrawn, reopened, a release that failed. A new request is checked against the
//! landings and releases, so a problem that is already fixed is not built again; the Timeline page
//! reads all of it.

use crate::service::Manager;
use crate::store::ReqRec;
use cc_protocol::contracts::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

/// How many of the newest ledger records a new request is compared with.
const LOOK_BACK: u64 = 200;

pub fn topic(repo: &str) -> String {
    format!("manage.ledger.{repo}")
}

/// One record of the ledger.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// `landed` | `released`.
    pub event: String,
    pub request: String,
    #[serde(default)]
    pub title: String,
    /// What was asked or reported: the symptoms, in the words that came in.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub tier: String,
    /// Who reported it, when apps did.
    #[serde(default)]
    pub reporters: Vec<String>,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub parts: Vec<String>,
    /// The versions it shipped in (`plugin 0.5.0`), once released.
    #[serde(default)]
    pub versions: Vec<String>,
    pub ms: u64,
    /// Why, in a sentence a person reads.
    #[serde(default)]
    pub why: String,
    /// Who decided: you | model | rule | app.
    #[serde(default)]
    pub by: String,
    /// What it points at: the request it was joined to, the one that follows it.
    #[serde(default)]
    pub link: String,
}

fn reporters(manager: &Manager, rec: &ReqRec) -> Vec<String> {
    manager.store.read(|data| {
        let mut out: Vec<String> = Vec::new();
        for other in data.requests.values().filter(|other| other.r.merged_into == rec.r.id).chain(std::iter::once(rec)) {
            if other.r.from.kind == "app" && !other.r.from.name.is_empty() && !out.contains(&other.r.from.name) {
                out.push(other.r.from.name.clone());
            }
        }
        out
    })
}

/// A request's work landed on the main branch.
pub fn landed(manager: &Manager, rec: &ReqRec, commit: &str) {
    let entry = Entry {
        event: "landed".into(),
        request: rec.r.id.clone(),
        title: rec.r.title.clone(),
        text: rec.r.text.clone(),
        kind: rec.r.kind.clone(),
        tier: rec.r.tier.as_str().into(),
        reporters: reporters(manager, rec),
        commit: commit.to_owned(),
        parts: rec.r.parts.clone(),
        versions: Vec::new(),
        ms: now_ms(),
        why: manager.t("why_landed", &[("commit", &short(commit))]),
        by: "rule".into(),
        link: String::new(),
    };
    manager.store.graph.append(&topic(&rec.r.repo), vec![json!(entry)], None);
}

fn short(commit: &str) -> String {
    commit.chars().take(7).collect()
}

/// Something else happened to a request: `event` is merged, closed, declined, withdrawn,
/// reopened, ready or waits; `why` the sentence a person reads; `by` who decided.
pub fn note(manager: &Manager, rec: &ReqRec, event: &str, why: &str, by: &str, link: &str) {
    let entry = Entry {
        event: event.to_owned(),
        request: rec.r.id.clone(),
        title: rec.r.title.clone(),
        text: String::new(),
        kind: rec.r.kind.clone(),
        tier: rec.r.tier.as_str().into(),
        reporters: Vec::new(),
        commit: String::new(),
        parts: Vec::new(),
        versions: Vec::new(),
        ms: now_ms(),
        why: why.trim().to_owned(),
        by: by.to_owned(),
        link: link.to_owned(),
    };
    manager.store.graph.append(&topic(&rec.r.repo), vec![json!(entry)], None);
}

/// The way the repository ships was learned again, or something else about the repository as a
/// whole that is not one request's.
pub fn release_note(manager: &Manager, repo: &str, why: &str) {
    let entry = Entry { event: "delivery".into(), ms: now_ms(), why: why.trim().to_owned(), by: "rule".into(), ..Entry::default() };
    manager.store.graph.append(&topic(repo), vec![json!(entry)], None);
}

/// A release of the repository did not go out.
pub fn release_failed(manager: &Manager, repo: &str, why: &str) {
    let entry = Entry { event: "release_failed".into(), ms: now_ms(), why: why.trim().to_owned(), by: "rule".into(), ..Entry::default() };
    manager.store.graph.append(&topic(repo), vec![json!(entry)], None);
}

/// A request's work shipped in a release.
pub fn released(manager: &Manager, rec: &ReqRec, versions: &[String]) {
    let entry = Entry {
        event: "released".into(),
        request: rec.r.id.clone(),
        title: rec.r.title.clone(),
        text: rec.r.text.clone(),
        kind: rec.r.kind.clone(),
        tier: rec.r.tier.as_str().into(),
        reporters: reporters(manager, rec),
        commit: String::new(),
        parts: rec.r.parts.clone(),
        versions: versions.to_vec(),
        ms: now_ms(),
        why: manager.t("why_released", &[("versions", &versions.join(", "))]),
        by: "rule".into(),
        link: String::new(),
    };
    manager.store.graph.append(&topic(&rec.r.repo), vec![json!(entry)], None);
}

/// What the repository has done lately, one entry per request: its landing, with the release's
/// versions and time when it shipped. Newest first.
pub async fn done(manager: &Manager, repo: &str) -> Vec<Entry> {
    let records = match manager.store.graph.newest(&topic(repo), LOOK_BACK).await {
        Ok(records) => records,
        Err(error) => {
            tracing::debug!(repo, error = %error.message, "the ledger could not be read");
            return Vec::new();
        }
    };
    let mut by_request: BTreeMap<String, Entry> = BTreeMap::new();
    for record in records {
        let Ok(entry) = serde_json::from_value::<Entry>(record.value) else { continue };
        // Only what was done counts as done.
        if entry.event != "landed" && entry.event != "released" {
            continue;
        }
        let kept = by_request.entry(entry.request.clone()).or_insert_with(|| entry.clone());
        if entry.event == "released" {
            kept.versions = entry.versions.clone();
            kept.event = "released".into();
            kept.ms = entry.ms;
        }
    }
    let mut out: Vec<Entry> = by_request.into_values().collect();
    out.sort_by(|a, b| b.ms.cmp(&a.ms));
    out
}

/// How many records of each repository the Timeline reads (a busy week fits many times over).
const TIMELINE_LOOK_BACK: u64 = 2000;

/// What happened between two moments in every repository, newest first.
pub async fn timeline(manager: &Manager, since_ms: u64, before_ms: u64) -> Vec<cc_protocol::contracts::manage::TimelineEntry> {
    use cc_protocol::contracts::manage::{TimelineEntry, ended, stage};
    let repos: Vec<(String, String)> = manager.store.read(|data| data.repos.values().map(|rec| (rec.repo.id.clone(), rec.repo.name.clone())).collect());
    let mut out: Vec<TimelineEntry> = Vec::new();
    for (repo, name) in repos {
        let records = match manager.store.graph.newest(&topic(&repo), TIMELINE_LOOK_BACK).await {
            Ok(records) => records,
            Err(error) => {
                tracing::debug!(repo, error = %error.message, "the ledger could not be read");
                continue;
            }
        };
        for record in records {
            let Ok(entry) = serde_json::from_value::<Entry>(record.value) else { continue };
            if entry.ms < since_ms || entry.ms > before_ms {
                continue;
            }
            out.push(TimelineEntry {
                repo: repo.clone(),
                repo_name: name.clone(),
                event: entry.event,
                request: entry.request,
                title: entry.title,
                why: entry.why,
                by: entry.by,
                link: if entry.link.is_empty() && !entry.commit.is_empty() { short(&entry.commit) } else { entry.link },
                versions: entry.versions,
                ms: entry.ms,
                reopen: false,
            });
        }
    }
    // Whether each request can be reopened is how it stands now, not how it stood then.
    manager.store.read(|data| {
        for entry in &mut out {
            entry.reopen = data.requests.get(&entry.request).is_some_and(|rec| {
                let finished = matches!(rec.r.ended_as.as_str(), ended::DECLINED | ended::MERGED | ended::WITHDRAWN | ended::RELEASED);
                finished || matches!(rec.r.stage.as_str(), stage::LANDED | stage::SOAKING | stage::RELEASED)
            });
        }
    });
    out.sort_by(|a, b| b.ms.cmp(&a.ms));
    out
}

/// Every ledger record of one request, oldest first.
pub async fn of_request(manager: &Manager, repo: &str, request: &str) -> Vec<cc_protocol::contracts::manage::TimelineEntry> {
    use cc_protocol::contracts::manage::TimelineEntry;
    let name = manager.store.read(|data| data.repos.get(repo).map(|rec| rec.repo.name.clone()).unwrap_or_default());
    let Ok(records) = manager.store.graph.newest(&topic(repo), TIMELINE_LOOK_BACK).await else { return Vec::new() };
    let mut out: Vec<TimelineEntry> = records
        .into_iter()
        .filter_map(|record| serde_json::from_value::<Entry>(record.value).ok())
        .filter(|entry| entry.request == request)
        .map(|entry| TimelineEntry {
            repo: repo.to_owned(),
            repo_name: name.clone(),
            event: entry.event,
            request: entry.request,
            title: entry.title,
            why: entry.why,
            by: entry.by,
            link: if entry.link.is_empty() && !entry.commit.is_empty() { short(&entry.commit) } else { entry.link },
            versions: entry.versions,
            ms: entry.ms,
            reopen: false,
        })
        .collect();
    out.sort_by_key(|entry| entry.ms);
    out
}
