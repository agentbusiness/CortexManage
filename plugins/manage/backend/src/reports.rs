//! Reports an app sends about itself (`contracts::reports`). They are kept when the sender's
//! repository is managed here: the GitHub repository a person set on it is the sender's, exactly
//! (GitHub's names do not tell capitals apart). Every row goes to the repository's reports stream,
//! kept a week. An error or a panic is also a failure to repair: it opens a request in the normal
//! flow, or adds to the open request of the same fingerprint.

use crate::service::Manager;
use crate::store::RepoRec;
use cc_protocol::contracts::manage::{From, RequestCreateParams};
use cc_protocol::contracts::now_ms;
use cc_protocol::contracts::reports::{ErrorRecord, SendParams, SendResult, Sender, tables};
use cc_protocol::{RpcError, parse_params, to_value};
use serde_json::{Value, json};
use std::sync::Arc;

/// Days the reports stream keeps a row.
const KEEP_DAYS: u32 = 7;
/// The most of a stack a request's text carries.
const STACK_CHARS: usize = 4000;

/// The stream of one repository's reports.
pub fn topic(repo: &str) -> String {
    format!("manage.reports.{repo}")
}

pub async fn send(manager: &Arc<Manager>, params: Value) -> Result<Value, RpcError> {
    let params: SendParams = parse_params(params)?;
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
    let kept: Vec<Value> = params
        .rows
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
    manager.store.graph.append(&topic(&repo.repo.id), kept, Some(KEEP_DAYS));
    if params.table == tables::ERRORS {
        for row in &params.rows {
            let Ok(record) = serde_json::from_value::<ErrorRecord>(row.clone()) else { continue };
            if record.is_failure() {
                failure(manager, &repo, &params.sender, record).await;
            }
        }
    }
    to_value(&SendResult { accepted: true, reason: String::new() })
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
