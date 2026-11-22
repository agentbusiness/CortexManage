//! Reports an app sends about itself (`contracts::reports`). They are kept when the sender's
//! repository is managed here: the GitHub repository a person set on it is the sender's, exactly
//! (GitHub's names do not tell capitals apart). Only the standard's tables are kept (`errors`,
//! `calls`, `models`). Every row goes to the repository's reports stream, kept a week, once: a row
//! whose event id the stream holds already is not kept again. The sender hears `accepted` only once
//! the rows are in the graph. An error or a panic is also a failure to repair: it opens a request in
//! the normal flow, or adds to the open request of the same fingerprint.

use crate::service::Manager;
use crate::store::RepoRec;
use cc_protocol::contracts::manage::{From, RequestCreateParams};
use cc_protocol::contracts::now_ms;
use cc_protocol::contracts::reports::{ErrorRecord, SendParams, SendResult, Sender, tables};
use cc_protocol::contracts::memory::StreamEntry;
use cc_protocol::{RpcError, parse_params, to_value};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// Days the reports stream keeps a row.
const KEEP_DAYS: u32 = 7;
/// The most of a stack a request's text carries.
const STACK_CHARS: usize = 4000;
/// How long the flow pages' "new reports" is held so that the pieces of one delivery are one read.
const CHANGED_AFTER: Duration = Duration::from_secs(2);

/// The reports streams as far as this process has read them, by topic and table: the Error flow
/// and Metrics flow pages read only the records appended since the last read. Not a record: the
/// stream in the graph is; this is what was read of it, gone with the process.
#[derive(Default)]
pub struct Cache {
    reads: std::sync::Mutex<HashMap<String, Read>>,
    /// Repositories whose "new reports" is waiting to be published.
    changing: std::sync::Mutex<HashSet<String>>,
}

#[derive(Default, Clone)]
struct Read {
    /// The offset after the last record read.
    next: Option<u64>,
    /// Each table's kept records, oldest first.
    tables: HashMap<String, Arc<Vec<StreamEntry>>>,
    /// The event ids of the kept records, with when each was appended.
    ids: HashMap<String, u64>,
}

impl Cache {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Read>> {
        self.reads.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The rows whose event id the stream does not hold yet (each once).
    async fn unseen(&self, manager: &Manager, topic: &str, rows: &[Value]) -> Result<Vec<Value>, RpcError> {
        let since = now_ms().saturating_sub((KEEP_DAYS as u64 + 1) * DAY_MS as u64);
        self.table(manager, topic, "", since).await?;
        let reads = self.lock();
        let known = reads.get(topic).map(|read| &read.ids);
        let mut taken: HashSet<&str> = HashSet::new();
        Ok(rows
            .iter()
            .filter(|row| {
                let Some(id) = row.get("event_id").and_then(Value::as_str) else { return true };
                !known.is_some_and(|ids| ids.contains_key(id)) && taken.insert(id)
            })
            .cloned()
            .collect())
    }

    /// Publish "new reports" for a repository once, a moment after its first piece.
    fn changed(&self, manager: &Arc<Manager>, repo_id: &str) {
        let first = self.changing.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(repo_id.to_owned());
        if !first {
            return;
        }
        let manager = manager.clone();
        let repo_id = repo_id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(CHANGED_AFTER).await;
            manager.reports.changing.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&repo_id);
            manager.changed("reports", "", &repo_id);
        });
    }

    /// One table of a stream from `since` on: what was read before, and what came after it.
    async fn table(&self, manager: &Manager, topic: &str, table: &str, since: u64) -> Result<Arc<Vec<StreamEntry>>, RpcError> {
        let next = self.lock().get(topic).and_then(|read| read.next);
        let fresh = manager.store.graph.since_from(topic, next, since).await?;
        let mut reads = self.lock();
        let read = reads.entry(topic.to_owned()).or_default();
        let after = read.next.unwrap_or(0);
        for entry in fresh {
            // Another page's read may have taken these already.
            if read.next.is_some() && entry.offset < after {
                continue;
            }
            read.next = Some(entry.offset + 1);
            if let Some(id) = entry.value.get("row").and_then(|row| row.get("event_id")).and_then(Value::as_str) {
                read.ids.insert(id.to_owned(), entry.at_ms);
            }
            let name = entry.value.get("table").and_then(Value::as_str).unwrap_or_default().to_owned();
            Arc::make_mut(read.tables.entry(name).or_default()).push(entry);
        }
        for entries in read.tables.values_mut() {
            let old = entries.partition_point(|entry| entry.at_ms < since);
            if old > 0 {
                Arc::make_mut(entries).drain(..old);
            }
        }
        read.ids.retain(|_, at| *at >= since);
        Ok(read.tables.get(table).cloned().unwrap_or_default())
    }
}

/// The stream of one repository's reports.
pub fn topic(repo: &str) -> String {
    format!("manage.reports.{repo}")
}

pub async fn send(manager: &Arc<Manager>, params: Value) -> Result<Value, RpcError> {
    let params: SendParams = parse_params(params)?;
    if !tables::ALL.contains(&params.table.as_str()) {
        return Err(RpcError::invalid_params(format!("no table `{}`: the tables are {}", params.table, tables::ALL.join(", "))));
    }
    let wanted = params.sender.repo.trim();
    let repo: Option<RepoRec> = manager.store.read(|data| {
        data.repos
            .values()
            .find(|rec| rec.repo.managed && !wanted.is_empty() && rec.repo.source.github.trim().eq_ignore_ascii_case(wanted))
            .cloned()
    });
    let Some(repo) = repo else {
        return to_value(&SendResult { accepted: false, reason: format!("{wanted} is not managed here") });
    };
    let now = now_ms();
    let stream = topic(&repo.repo.id);
    let rows = manager.reports.unseen(manager, &stream, &params.rows).await?;
    let kept: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "table": params.table,
                "part": params.part,
                "day": params.day,
                "repo": params.sender.repo,
                "software": params.sender.software,
                "version": params.sender.version,
                "node": params.sender.machine.node,
                "machine": params.sender.machine.name,
                "received_ms": now,
                "row": row,
            })
        })
        .collect();
    if kept.is_empty() {
        return to_value(&SendResult { accepted: true, reason: String::new() });
    }
    manager.store.graph.append(&stream, kept, Some(KEEP_DAYS));
    // Accepted only once the rows are in the graph: the sender then marks them sent.
    if !manager.store.graph.flush().await {
        return Err(RpcError::new(cc_protocol::codes::UNAVAILABLE, "the reports could not be kept now; send them again"));
    }
    manager.reports.changed(manager, &repo.repo.id);
    if params.table == tables::ERRORS {
        for row in &rows {
            let Ok(record) = serde_json::from_value::<ErrorRecord>(row.clone()) else { continue };
            if record.is_failure() {
                failure(manager, &repo, &params.sender, record).await;
            }
        }
    }
    to_value(&SendResult { accepted: true, reason: String::new() })
}

/// What the Error flow and Metrics flow pages read: one table of every repository's reports (or
/// one repository's, one computer's), the kept days with their totals, the computers that sent,
/// and every row of the chosen days. A row is the app's own, with who sent it beside it.
pub async fn read(manager: &Arc<Manager>, params: Value) -> Result<Value, RpcError> {
    let table = params.get("table").and_then(Value::as_str).unwrap_or(tables::ERRORS).to_owned();
    let only_repo = params.get("repo").and_then(Value::as_str).unwrap_or_default().to_owned();
    let only_node = params.get("node").and_then(Value::as_str).unwrap_or_default().to_owned();
    let now = now_ms();
    let today = day_of(now as i64);
    let days: Vec<String> = (0..KEEP_DAYS as i64).rev().map(|back| day_of(now as i64 - back * DAY_MS)).collect();
    let from = params.get("from").and_then(Value::as_str).filter(|day| !day.is_empty()).unwrap_or(&today).to_owned();
    let to = params.get("to").and_then(Value::as_str).filter(|day| !day.is_empty()).unwrap_or(&today).to_owned();
    // A day more than kept: a row arrives a little after it happened.
    let since = now.saturating_sub((KEEP_DAYS as u64 + 1) * DAY_MS as u64);
    let prefix = topic("");
    let repos: Vec<String> = manager.store.read(|data| data.repos.keys().cloned().collect());
    let mut totals: std::collections::BTreeMap<String, (u64, u64, serde_json::Map<String, Value>)> = days.iter().map(|day| (day.clone(), (0, 0, serde_json::Map::new()))).collect();
    let mut computers: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut rows: Vec<Value> = Vec::new();
    for name in manager.store.graph.topics(&prefix).await? {
        let repo = name.strip_prefix(&prefix).unwrap_or_default().to_owned();
        if !repos.contains(&repo) || (!only_repo.is_empty() && repo != only_repo) {
            continue;
        }
        for entry in manager.reports.table(manager, &name, &table, since).await?.iter() {
            let kept = &entry.value;
            let node = kept.get("node").and_then(Value::as_str).unwrap_or_default().to_owned();
            let machine = kept.get("machine").and_then(Value::as_str).unwrap_or_default().to_owned();
            if !node.is_empty() {
                computers.insert(node.clone(), machine.clone());
            }
            if !only_node.is_empty() && node != only_node {
                continue;
            }
            let Some(Value::Object(mut row)) = kept.get("row").cloned() else { continue };
            if table == tables::MODELS {
                // A row sums one bucket's calls: its times as the other tables name theirs.
                let started = row.get("started_at_ms").cloned().unwrap_or(Value::Null);
                let ended = row.get("ended_at_ms").cloned().unwrap_or(Value::Null);
                row.insert("first_at_ms".into(), started);
                row.insert("last_at_ms".into(), ended);
            }
            let at = row.get("first_at_ms").and_then(Value::as_i64).unwrap_or(entry.at_ms as i64);
            let day = day_of(at);
            let count = row.get("count").and_then(Value::as_u64).unwrap_or(1);
            let kind_field = if table == tables::ERRORS { "kind" } else { "outcome" };
            let kind = row.get(kind_field).and_then(Value::as_str).unwrap_or_default().to_owned();
            if let Some((sum, groups, by_kind)) = totals.get_mut(&day) {
                *sum += count;
                *groups += 1;
                let before = by_kind.get(&kind).and_then(Value::as_u64).unwrap_or(0);
                by_kind.insert(kind, json!(before + count));
            }
            if day < from || day > to {
                continue;
            }
            row.insert("repo".into(), json!(repo));
            row.insert("app".into(), kept.get("software").cloned().unwrap_or(Value::Null));
            row.insert("app_version".into(), kept.get("version").cloned().unwrap_or(Value::Null));
            row.insert("node".into(), json!(node));
            let computer = if machine.is_empty() { node.clone() } else { machine };
            // A model call no claim placed elsewhere ran on the sender's own computer.
            if table == tables::MODELS && row.get("served_by").and_then(Value::as_str).unwrap_or_default().is_empty() {
                row.insert("served_by".into(), json!(computer));
            }
            row.insert("computer".into(), json!(computer));
            rows.push(Value::Object(row));
        }
    }
    // Each failure's request: the open one, else the newest, so the page can say what became of it.
    let requests: serde_json::Map<String, Value> = if table == tables::ERRORS {
        manager.store.read(|data| {
            let mut out: std::collections::BTreeMap<String, (bool, u64, Value)> = std::collections::BTreeMap::new();
            for rec in data.requests.values().filter(|rec| !rec.r.fingerprint.is_empty()) {
                let key = format!("{}|{}", rec.r.repo, rec.r.fingerprint);
                let open = rec.r.ended_as.is_empty();
                let rank = (open, rec.r.created_ms);
                if out.get(&key).is_some_and(|(o, c, _)| (*o, *c) >= rank) {
                    continue;
                }
                let state = if open { rec.r.stage.clone() } else { rec.r.ended_as.clone() };
                out.insert(key, (open, rec.r.created_ms, json!({"id": rec.r.id, "repo": rec.r.repo, "package": rec.r.package, "open": open, "state": state})));
            }
            out.into_iter().map(|(key, (_, _, view))| (key, view)).collect()
        })
    } else {
        serde_json::Map::new()
    };
    let days: Vec<Value> = totals.into_iter().map(|(date, (count, groups, by_kind))| json!({"date": date, "count": count, "groups": groups, "by_kind": by_kind})).collect();
    let computers: Vec<Value> = computers.into_iter().map(|(node, name)| json!({"node": node, "name": if name.is_empty() { node.clone() } else { name }})).collect();
    Ok(json!({"table": table, "today": today, "keep_days": KEEP_DAYS, "days": days, "computers": computers, "rows": rows, "requests": requests}))
}

/// A person asks for a reported failure to be repaired: its request is opened as a report opens
/// one, from the newest row of that fingerprint, unless one is open already (then that one).
pub async fn request(manager: &Arc<Manager>, params: Value) -> Result<Value, RpcError> {
    let repo_id = params.get("repo").and_then(Value::as_str).unwrap_or_default().to_owned();
    let fingerprint = params.get("fingerprint").and_then(Value::as_str).unwrap_or_default().to_owned();
    if fingerprint.is_empty() {
        return Err(RpcError::invalid_params("a failure is named by its repository and fingerprint"));
    }
    let repo = manager.repo(&repo_id)?;
    let open: Option<String> = manager.store.read(|data| {
        data.requests.values().find(|rec| rec.r.repo == repo_id && rec.r.ended_as.is_empty() && rec.r.fingerprint == fingerprint).map(|rec| rec.r.id.clone())
    });
    if let Some(id) = open {
        return to_value(&manager.request_view_of(&id)?);
    }
    let since = now_ms().saturating_sub((KEEP_DAYS as u64 + 1) * DAY_MS as u64);
    let mut newest: Option<(Sender, ErrorRecord)> = None;
    for entry in manager.store.graph.since(&topic(&repo_id), since).await? {
        let kept = entry.value;
        if kept.get("table").and_then(Value::as_str) != Some(tables::ERRORS) {
            continue;
        }
        let Some(row) = kept.get("row").cloned() else { continue };
        let Ok(record) = serde_json::from_value::<ErrorRecord>(row) else { continue };
        if record.fingerprint != fingerprint || newest.as_ref().is_some_and(|(_, best)| best.last_at_ms > record.last_at_ms) {
            continue;
        }
        let text = |field: &str| kept.get(field).and_then(Value::as_str).unwrap_or_default().to_owned();
        let sender = Sender {
            repo: text("repo"),
            software: text("software"),
            version: text("version"),
            machine: cc_protocol::contracts::reports::Machine { node: text("node"), name: text("machine"), ..Default::default() },
        };
        newest = Some((sender, record));
    }
    let Some((sender, record)) = newest else {
        return Err(RpcError::not_found("this failure is no longer kept: reports are kept a week"));
    };
    failure(manager, &repo, &sender, record).await;
    let id: Option<String> = manager.store.read(|data| {
        data.requests.values().find(|rec| rec.r.repo == repo_id && rec.r.ended_as.is_empty() && rec.r.fingerprint == fingerprint).map(|rec| rec.r.id.clone())
    });
    match id {
        Some(id) => to_value(&manager.request_view_of(&id)?),
        None => Err(RpcError::internal("the request could not be opened; the manage log says why")),
    }
}

const DAY_MS: i64 = 86_400_000;

/// `YYYY-MM-DD` (UTC) of a time.
fn day_of(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|at| at.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// One failure: counted on the open request of its fingerprint, or a new request.
async fn failure(manager: &Arc<Manager>, repo: &RepoRec, sender: &Sender, record: ErrorRecord) {
    let open: Option<String> = manager.store.read(|data| {
        data.requests
            .values()
            .find(|rec| rec.r.repo == repo.repo.id && rec.r.ended_as.is_empty() && rec.r.fingerprint == record.fingerprint)
            .map(|rec| rec.r.id.clone())
    });
    let last = record.last_at_ms.max(0) as u64;
    if let Some(id) = open {
        manager.store.update_request(&id, |rec| {
            rec.r.seen = rec.r.seen.saturating_add(record.count);
            rec.r.seen_last_ms = rec.r.seen_last_ms.max(last);
        });
        manager.changed("request", &id, &repo.repo.id);
        return;
    }
    let text = text(manager, sender, &record);
    let title = record.message.lines().next().unwrap_or_default().chars().take(100).collect::<String>();
    let from = From {
        kind: "app".into(),
        name: manager.t("report_from", &[("software", &sender.software), ("version", &sender.version), ("machine", &sender.machine.name)]),
        node: sender.machine.node.clone(),
    };
    let params = RequestCreateParams { repo: repo.repo.id.clone(), text, title, ..RequestCreateParams::default() };
    if let Err(error) = manager.open_request(params, from, Some((record.fingerprint.clone(), record.count, last))).await {
        tracing::warn!(repo = %repo.repo.id, error = %error.message, "a reported failure could not become a request");
    }
}

/// What the request says: the failure as the app recorded it.
fn text(manager: &Manager, sender: &Sender, record: &ErrorRecord) -> String {
    let others: Vec<&String> = record.samples.iter().filter(|sample| **sample != record.message).collect();
    let others = if others.is_empty() { String::new() } else { others.iter().map(|sample| format!("- {sample}")).collect::<Vec<_>>().join("\n") };
    let stack: String = record.stack.as_deref().unwrap_or_default().chars().take(STACK_CHARS).collect();
    let fields = if record.fields.as_object().is_some_and(|f| !f.is_empty()) { record.fields.to_string() } else { String::new() };
    let scenario = if record.scenario.as_object().is_some_and(|f| !f.is_empty()) { record.scenario.to_string() } else { String::new() };
    let count = record.count.to_string();
    manager.t(
        "report_text",
        &[
            ("message", &record.message),
            ("software", &sender.software),
            ("version", &sender.version),
            ("machine", &sender.machine.name),
            ("os", &sender.machine.os_version),
            ("kind", &record.kind),
            ("location", &record.location),
            ("target", &record.target),
            ("count", &count),
            ("others", &others),
            ("fields", &fields),
            ("scenario", &scenario),
            ("stack", &stack),
        ],
    )
}
