//! How a repository ships (its delivery profile, [`Delivery`]): learned when the repository is
//! added, again when a person asks or a file it was learned from changed, and corrected by a person.
//!
//! What is declared is read, nothing is guessed (law 9): code first gathers what GitHub declares
//! for the main branch (its rules, the workflows, the newest merged pull requests), then one model
//! question (DELIVERY, with read-only tools on GitHub) fills the profile, every value with the file,
//! the line and the words it was read from. Code keeps a value only when those words are found
//! there; a value nothing declares stays empty and the window says so. What a person set is never
//! changed by learning again.

use crate::service::Manager;
use cc_protocol::contracts::manage::{Delivery, DeliveryAnswer, DeliveryEditParams, DeliveryFact, DeliveryParams, Repo, flow};
use cc_protocol::contracts::now_ms;
use cc_protocol::{RpcError, codes};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The fields of a profile a person or the model can set, and whether each is a list.
pub const FIELDS: &[(&str, bool)] = &[
    ("reach_main", false),
    ("main_is", false),
    ("release_kind", false),
    ("release_steps", true),
    ("plan_step", false),
    ("tag", false),
    ("version_kind", false),
    ("version_file", false),
    ("version_key", false),
    ("checks", true),
    ("ships", false),
    ("ci", false),
    ("take_back", false),
    ("secrets", true),
    ("secret_files", true),
    ("secrets_from", false),
];

/// The words each closed field may take.
fn allowed(field: &str) -> Option<&'static [&'static str]> {
    match field {
        "reach_main" => Some(&["direct", "pr", "pr-review"]),
        "main_is" => Some(&["collects", "ships"]),
        "release_kind" => Some(&["steps", "merge", "tag", "none"]),
        "version_kind" => Some(&["by-script", "given", "commit", "none"]),
        _ => None,
    }
}

/// A value of `delivery`'s field as text (a list one item per line).
pub fn get(delivery: &Delivery, field: &str) -> String {
    let d = delivery;
    match field {
        "reach_main" => d.reach_main.clone(),
        "main_is" => d.main_is.clone(),
        "release_kind" => d.release_kind.clone(),
        "release_steps" => d.release_steps.join("\n"),
        "plan_step" => d.plan_step.clone(),
        "tag" => d.tag.clone(),
        "version_kind" => d.version_kind.clone(),
        "version_file" => d.version_file.clone(),
        "version_key" => d.version_key.clone(),
        "checks" => d.checks.join("\n"),
        "ships" => d.ships.clone(),
        "ci" => d.ci.clone(),
        "take_back" => d.take_back.clone(),
        "secrets" => d.secrets.join("\n"),
        "secret_files" => d.secret_files.join("\n"),
        "secrets_from" => d.secrets_from.clone(),
        _ => String::new(),
    }
}

/// Set a field from text. Err: the sentence saying why the value cannot be taken.
pub fn set(delivery: &mut Delivery, field: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    if let Some(words) = allowed(field)
        && !value.is_empty()
        && !words.contains(&value)
    {
        return Err(format!("{field} is one of {}", words.join(", ")));
    }
    let lines = || value.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect::<Vec<_>>();
    let d = delivery;
    match field {
        "reach_main" => d.reach_main = value.to_owned(),
        "main_is" => d.main_is = value.to_owned(),
        "release_kind" => d.release_kind = value.to_owned(),
        "release_steps" => d.release_steps = lines(),
        "plan_step" => d.plan_step = value.to_owned(),
        "tag" => d.tag = value.to_owned(),
        "version_kind" => d.version_kind = value.to_owned(),
        "version_file" => d.version_file = value.to_owned(),
        "version_key" => d.version_key = value.to_owned(),
        "checks" => d.checks = lines(),
        "ships" => d.ships = value.to_owned(),
        "ci" => d.ci = value.to_owned(),
        "take_back" => d.take_back = value.to_owned(),
        "secrets" => d.secrets = lines(),
        "secret_files" => d.secret_files = lines(),
        "secrets_from" => {
            if !value.is_empty() && !value.starts_with('/') && !value.starts_with("~/") {
                return Err("the folder the secret files come from is a full path on the release machine".into());
            }
            d.secrets_from = value.to_owned();
        }
        other => return Err(format!("`{other}` is not a field of how a repository ships")),
    }
    Ok(())
}

/// A text with its blanks made single, in lower case: how a quote is looked for.
fn plain(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A read of the repository on GitHub with its connection (`vcs.github.*`).
async fn github(manager: &Manager, repo: &Repo, method: &str, extra: Value) -> Result<Value, RpcError> {
    let mut params = json!({"repo": repo.source.github, "connection": repo.source.connection, "branch": repo.source.main});
    if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        params.extend(extra.clone());
    }
    manager.peer::<Value>(method, params).await
}

/// One file of the main branch on GitHub, as text.
pub async fn file(manager: &Manager, repo: &Repo, path: &str) -> Result<String, RpcError> {
    let answer = github(manager, repo, "vcs.github.file", json!({"path": path})).await?;
    Ok(answer.get("text").and_then(Value::as_str).unwrap_or_default().to_owned())
}

/// What GitHub declares about the main branch, as the lines the model reads and the quotes are
/// checked against. Each part says where it came from.
async fn declared(manager: &Manager, repo: &Repo) -> String {
    let mut parts: Vec<String> = Vec::new();
    match github(manager, repo, "vcs.github.rules", json!({})).await {
        Ok(answer) => {
            let rules: Vec<String> = answer
                .get("rules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|rule| {
                    let kind = rule.get("type").and_then(Value::as_str).unwrap_or_default();
                    let parameters = rule.get("parameters").filter(|value| !value.is_null()).map(Value::to_string).unwrap_or_default();
                    format!("- {kind} {parameters}").trim_end().to_owned()
                })
                .collect();
            parts.push(if rules.is_empty() { format!("github:rules (branch {}): none", repo.source.main) } else { format!("github:rules (branch {}):\n{}", repo.source.main, rules.join("\n")) });
        }
        Err(error) => parts.push(format!("github:rules: could not be read ({})", error.message)),
    }
    match github(manager, repo, "vcs.github.workflows", json!({})).await {
        Ok(answer) => {
            let flows: Vec<String> = answer
                .get("workflows")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|flow| format!("- {} ({}, {})", flow.get("name").and_then(Value::as_str).unwrap_or_default(), flow.get("path").and_then(Value::as_str).unwrap_or_default(), flow.get("state").and_then(Value::as_str).unwrap_or_default()))
                .collect();
            parts.push(if flows.is_empty() { "github:workflows: none".into() } else { format!("github:workflows:\n{}", flows.join("\n")) });
        }
        Err(error) => parts.push(format!("github:workflows: could not be read ({})", error.message)),
    }
    match github(manager, repo, "vcs.github.pulls", json!({})).await {
        Ok(answer) => {
            let pulls: Vec<String> = answer
                .get("pulls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|pr| {
                    let merged = pr.get("merged").and_then(Value::as_bool).unwrap_or(false);
                    format!("- #{} {} ({})", pr.get("number").and_then(Value::as_u64).unwrap_or(0), pr.get("title").and_then(Value::as_str).unwrap_or_default(), if merged { "merged" } else { "closed unmerged" })
                })
                .collect();
            parts.push(if pulls.is_empty() { format!("github:pulls (into {}): none", repo.source.main) } else { format!("github:pulls (newest closed into {}):\n{}", repo.source.main, pulls.join("\n")) });
        }
        Err(error) => parts.push(format!("github:pulls: could not be read ({})", error.message)),
    }
    parts.join("\n\n")
}

/// Whether `quote` is at `line` of `text`, give or take three lines (a model counts lines
/// loosely; the words must still be there).
fn found_at(text: &str, line: u32, quote: &str) -> bool {
    let quote = plain(quote);
    if quote.is_empty() {
        return false;
    }
    let lines: Vec<&str> = text.lines().collect();
    if line == 0 {
        return plain(text).contains(&quote);
    }
    let at = line as usize;
    let from = at.saturating_sub(4);
    let to = (at + 3).min(lines.len());
    from < to && plain(&lines[from..to].join("\n")).contains(&quote)
}

/// Learn the profile in the background; answers at once with the profile marked `learning`.
pub fn learn(manager: &Arc<Manager>, repo_id: &str) -> Result<Delivery, RpcError> {
    let rec = manager.repo(repo_id)?;
    if rec.repo.source.github.trim().is_empty() {
        return Err(RpcError::invalid_params(manager.t("delivery_no_github", &[])));
    }
    if rec.repo.delivery.state == "learning" {
        return Ok(rec.repo.delivery);
    }
    let updated = manager.store.update_repo(repo_id, |rec| {
        rec.repo.delivery.state = "learning".into();
        rec.repo.delivery.error.clear();
    });
    manager.changed("repo", repo_id, repo_id);
    let (manager, id) = (manager.clone(), repo_id.to_owned());
    tokio::spawn(async move {
        let outcome = learned(&manager, &id).await;
        let finished = manager.store.update_repo(&id, |rec| match outcome {
            Ok(fresh) => rec.repo.delivery = fresh,
            Err(error) => {
                rec.repo.delivery.state = "failed".into();
                rec.repo.delivery.error = error.message;
            }
        });
        if let Some(rec) = finished {
            tracing::info!(repo = %id, state = %rec.repo.delivery.state, kind = %rec.repo.delivery.release_kind, "how the repository ships was learned");
        }
        manager.changed("repo", &id, &id);
    });
    Ok(updated.map(|rec| rec.repo.delivery).unwrap_or_default())
}

/// The profile as it is learned now, over what a person set.
async fn learned(manager: &Arc<Manager>, repo_id: &str) -> Result<Delivery, RpcError> {
    let repo = manager.repo(repo_id)?.repo;
    let github = declared(manager, &repo).await;
    let params = DeliveryParams { repo: repo.id.clone(), repo_name: repo.name.clone(), github: github.clone(), main: repo.source.main.clone() };
    let answer: DeliveryAnswer = manager.peer(flow::DELIVERY, json!(params)).await?;
    let old = manager.repo(repo_id)?.repo.delivery;
    let mut fresh = Delivery { state: "learned".into(), learned_ms: now_ms(), version_now: old.version_now.clone(), secrets_from: old.secrets_from.clone(), ..Delivery::default() };
    // Each value is kept only when one of its quotes is where it says.
    let mut texts: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut verified: Vec<DeliveryFact> = Vec::new();
    for fact in &answer.facts {
        let ok = if fact.path.starts_with("github:") {
            plain(&github).contains(&plain(&fact.quote))
        } else {
            if !texts.contains_key(&fact.path) {
                let text = file(manager, &repo, &fact.path).await.unwrap_or_default();
                texts.insert(fact.path.clone(), text);
            }
            texts.get(&fact.path).is_some_and(|text| found_at(text, fact.line, &fact.quote))
        };
        if ok {
            verified.push(fact.clone());
        } else {
            tracing::info!(repo = %repo_id, field = %fact.field, path = %fact.path, line = fact.line, "a delivery fact's quote was not found where it says; its value is left out");
        }
    }
    let backed: BTreeSet<&str> = verified.iter().map(|fact| fact.field.as_str()).collect();
    for (field, value) in &answer.values {
        if value.trim().is_empty() || !backed.contains(field.as_str()) {
            continue;
        }
        if let Err(problem) = set(&mut fresh, field, value) {
            tracing::info!(repo = %repo_id, %field, %problem, "a learned delivery value is not one the field takes");
        }
    }
    // What can be said from what was found is said: nothing ships without a release kind.
    if fresh.release_kind.is_empty() {
        fresh.release_kind = "none".into();
    }
    // What a person set stays theirs.
    for field in &old.set_by_person {
        let value = get(&old, field);
        let _ = set(&mut fresh, field, &value);
    }
    fresh.set_by_person = old.set_by_person.clone();
    fresh.facts = verified;
    let mut files: Vec<String> = texts.keys().filter(|path| fresh.facts.iter().any(|fact| &fact.path == *path)).cloned().collect();
    files.sort();
    fresh.hashes = files.iter().map(|path| cc_protocol::sha256::hex(texts.get(path).map(String::as_bytes).unwrap_or_default())).collect();
    fresh.files = files;
    Ok(fresh)
}

/// A person's correction. The fields they set are kept when the profile is learned again.
pub fn edit(manager: &Manager, params: &DeliveryEditParams) -> Result<Delivery, RpcError> {
    let rec = manager.repo(&params.repo)?;
    let mut delivery = rec.repo.delivery.clone();
    for (field, value) in &params.set {
        set(&mut delivery, field, value).map_err(RpcError::invalid_params)?;
        if !delivery.set_by_person.contains(field) {
            delivery.set_by_person.push(field.clone());
        }
    }
    if delivery.state.is_empty() {
        delivery.state = "learned".into();
    }
    let updated = manager.store.update_repo(&params.repo, |rec| rec.repo.delivery = delivery.clone()).ok_or_else(|| RpcError::not_found(format!("no repository `{}`", params.repo)))?;
    manager.changed("repo", &params.repo, &params.repo);
    Ok(updated.repo.delivery)
}

/// Whether a file the profile was learned from changed on the main branch: then it is learned
/// again in the background. Nothing waits for it.
pub async fn refresh_if_changed(manager: &Arc<Manager>, repo: &Repo) {
    let delivery = &repo.delivery;
    if delivery.state != "learned" || repo.source.github.is_empty() {
        return;
    }
    for (path, hash) in delivery.files.iter().zip(&delivery.hashes) {
        let now = match file(manager, repo, path).await {
            Ok(text) => cc_protocol::sha256::hex(text.as_bytes()),
            // Gone counts as changed; unreachable says nothing.
            Err(error) if error.code == codes::NOT_FOUND => String::new(),
            Err(_) => return,
        };
        if &now != hash {
            tracing::info!(repo = %repo.id, %path, "a file the release method was learned from changed; it is learned again");
            if learn(manager, &repo.id).is_ok() {
                crate::ledger::release_note(manager, &repo.id, &manager.t("why_delivery_changed", &[("path", path)]));
            }
            return;
        }
    }
}

/// The version a `given` release gets: the next of the newest of the version file's and the last
/// release's, by the strictest tier of what it carries.
pub async fn next_version(manager: &Manager, repo: &Repo, tier: &str) -> Result<String, RpcError> {
    let delivery = &repo.delivery;
    let mut now = String::new();
    if !delivery.version_file.is_empty() {
        let text = file(manager, repo, &delivery.version_file).await?;
        now = version_in(&text, &delivery.version_file, &delivery.version_key).unwrap_or_default();
    }
    if newer(&delivery.version_now, &now) {
        now = delivery.version_now.clone();
    }
    if now.is_empty() {
        return Err(RpcError::new(codes::CONFLICT, manager.t("version_unknown", &[("file", &delivery.version_file)])));
    }
    // The part before a pre-release or build suffix is what moves.
    let core: String = now.split(['-', '+']).next().unwrap_or_default().to_owned();
    let answer: Value = manager.peer("train.bump", json!({"version": core, "tier": tier, "breaking": false})).await?;
    answer.get("version").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| RpcError::internal("train.bump gave no version"))
}

fn parts(version: &str) -> Vec<u64> {
    version.split(['-', '+']).next().unwrap_or_default().split('.').map(|part| part.parse::<u64>().unwrap_or(0)).collect()
}

/// `a` is a later version than `b` (an empty one is never later).
fn newer(a: &str, b: &str) -> bool {
    !a.is_empty() && (b.is_empty() || parts(a) > parts(b))
}

/// The version a file declares under `key`: a TOML table path and key (`[workspace.package]
/// version`, or `workspace.package.version`), or a JSON key path (`version`).
fn version_in(text: &str, file: &str, key: &str) -> Option<String> {
    let key = key.trim().replace(['[', ']'], "").replace(' ', ".");
    let path: Vec<&str> = key.split('.').filter(|part| !part.is_empty()).collect();
    if path.is_empty() {
        return None;
    }
    let value: Value = if file.ends_with(".json") { serde_json::from_str(text).ok()? } else { serde_json::to_value(toml::from_str::<toml::Value>(text).ok()?).ok()? };
    let mut at = &value;
    for part in path {
        at = at.get(part)?;
    }
    at.as_str().map(str::to_owned)
}
