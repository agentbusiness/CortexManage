//! The ledger of a repository: an append-only stream in the graph (`manage.ledger.<repo>`) of what
//! was done, for which symptoms, and in which version it shipped. One record when a request's work
//! lands on the main branch, one when it is released. A new request is checked against it, so a
//! problem that is already fixed is not built again.

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
    };
    manager.store.graph.append(&topic(&rec.r.repo), vec![json!(entry)], None);
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
