//! A repository's release, by its delivery profile (`manage.runner.release`, which the release
//! train calls): plan what would ship, then ship it. One road for every kind:
//!
//! | `release_kind` | Plan | Release |
//! |---|---|---|
//! | `steps` | the profile's `plan_step` on the release machine, else the version it gets | its `release_steps`, in order, in a clean checkout of main |
//! | `merge` | the packages waiting on their branches | they land, one after the other; the CI run of the new head is the release, and its result is the answer |
//! | `tag` | the version the tag gets | a tag on main's head; its CI run is the answer |
//! | `none` | nothing | nothing |
//!
//! A repository whose profile was never learned releases as before: `scripts/release.sh` on its
//! runner. Commands run where the code is, on the runner; GitHub is read and tagged with the
//! repository's own connection.

use crate::service::{Manager, RUNNER_BUILD};
use cc_protocol::contracts::manage::{Repo, stage};
use cc_protocol::contracts::runner::{self, ReleaseAnswer, ReleasedPart};
use cc_protocol::{RpcError, codes, to_value};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// How often a CI run is looked at while it builds.
const CI_LOOK: Duration = Duration::from_secs(20);
/// How long a CI run may take to appear after main moved before it counts as not started.
const CI_APPEARS_MS: u64 = 10 * 60_000;
/// Lines of a release's output kept with it.
const TAIL: usize = 30;

/// The repository's GitHub token for its connection, for the runner's clones and pushes. Empty
/// when it has no GitHub repository or no connection answers: the runner then uses its own.
pub async fn token_of(manager: &Manager, repo: &Repo) -> String {
    if repo.source.github.is_empty() && repo.source.connection.is_empty() {
        return String::new();
    }
    match manager.peer::<Value>("vcs.github.credential", json!({"connection": repo.source.connection})).await {
        Ok(answer) => answer.get("token").and_then(Value::as_str).unwrap_or_default().to_owned(),
        Err(error) => {
            tracing::info!(repo = %repo.id, error = %error.message, "no GitHub token for the repository's connection; the runner uses its own sign-in");
            String::new()
        }
    }
}

/// The strictest tier of what the next release of the repository carries.
fn tier_of(manager: &Manager, repo: &str) -> String {
    manager.store.read(|data| {
        data.packages
            .values()
            .filter(|package| package.repo == repo && matches!(package.stage.as_str(), stage::READY | stage::LANDED | stage::SOAKING))
            .map(|package| package.tier)
            .max()
            .unwrap_or_default()
            .as_str()
            .to_owned()
    })
}

/// `{version}` filled into a command.
fn filled(command: &str, version: &str) -> String {
    command.replace("{version}", version)
}

fn answer(ok: bool, parts: Vec<ReleasedPart>, tail: Vec<String>, head: String) -> ReleaseAnswer {
    ReleaseAnswer { ok, parts, tail, head, stepped: true }
}

pub async fn run(manager: &Arc<Manager>, repo_id: &str, plan_only: bool) -> Result<Value, RpcError> {
    let repo = manager.repo(repo_id)?.repo;
    if plan_only {
        // A file the profile was read from may have changed: learned again in the background.
        let (manager, repo) = (manager.clone(), repo.clone());
        tokio::spawn(async move { crate::delivery::refresh_if_changed(&manager, &repo).await });
    }
    let delivery = repo.delivery.clone();
    let kind = if delivery.state == "learned" || !delivery.set_by_person.is_empty() { delivery.release_kind.as_str() } else { "" };
    match kind {
        "none" => to_value(&answer(true, Vec::new(), vec![manager.t("release_none", &[])], String::new())),
        "merge" => merge(manager, &repo, plan_only).await,
        "tag" => tag(manager, &repo, plan_only).await,
        "steps" if !delivery.release_steps.is_empty() => steps(manager, &repo, plan_only).await,
        // Never learned, or learned without steps: the repository's own release script.
        _ => script(manager, &repo, plan_only).await,
    }
}

/// `scripts/release.sh` on the runner, as before profiles.
async fn script(manager: &Arc<Manager>, repo: &Repo, plan_only: bool) -> Result<Value, RpcError> {
    let node = manager.runner_of(repo).await?;
    let release = runner::ReleaseParams {
        repo: repo.id.clone(),
        url: repo.source.url.clone(),
        main: repo.source.main.clone(),
        plan_only,
        token: token_of(manager, repo).await,
        ..runner::ReleaseParams::default()
    };
    let answer: ReleaseAnswer = manager.runner(&node, runner::methods::RELEASE, json!(release), RUNNER_BUILD).await?;
    to_value(&answer)
}

/// The profile's commands on the runner.
async fn steps(manager: &Arc<Manager>, repo: &Repo, plan_only: bool) -> Result<Value, RpcError> {
    let delivery = &repo.delivery;
    let given = delivery.version_kind == "given";
    let version = if given { Some(crate::delivery::next_version(manager, repo, &tier_of(manager, &repo.id)).await?) } else { None };
    let version_text = version.clone().unwrap_or_default();
    // A release that sets its own versions says what it would move (`--plan`); otherwise what
    // moves is the repository, to the version it gets.
    let whole = || vec![ReleasedPart { part: repo.name.clone(), from: delivery.version_now.clone(), to: version_text.clone() }];
    if plan_only && (delivery.plan_step.is_empty() || given) {
        return to_value(&answer(true, whole(), Vec::new(), String::new()));
    }
    let node = manager.runner_of(repo).await?;
    let mut env = Vec::new();
    if let Some(version) = &version {
        env.push(("RELEASE_VERSION".to_owned(), version.clone()));
    }
    let release = runner::ReleaseParams {
        repo: repo.id.clone(),
        url: repo.source.url.clone(),
        main: repo.source.main.clone(),
        plan_only,
        steps: delivery.release_steps.iter().map(|step| filled(step, &version_text)).collect(),
        plan_step: filled(&delivery.plan_step, &version_text),
        env,
        secret_files: delivery.secret_files.clone(),
        secrets_from: delivery.secrets_from.clone(),
        fetch_only: false,
        token: token_of(manager, repo).await,
    };
    let mut ran: ReleaseAnswer = manager.runner(&node, runner::methods::RELEASE, json!(release), RUNNER_BUILD).await?;
    if !ran.stepped {
        // An older runner ran `scripts/release.sh` whatever it was sent.
        ran.ok = false;
        ran.tail.push(manager.t("runner_no_steps", &[]));
        return to_value(&ran);
    }
    if ran.ok && ran.parts.is_empty() {
        ran.parts = whole();
    }
    if ran.ok && !plan_only && let Some(version) = version {
        manager.store.update_repo(&repo.id, |rec| rec.repo.delivery.version_now = version.clone());
    }
    to_value(&ran)
}

/// The packages waiting on their branches land, one after the other; the CI run of main's new
/// head is the release.
async fn merge(manager: &Arc<Manager>, repo: &Repo, plan_only: bool) -> Result<Value, RpcError> {
    let ready: Vec<String> = manager.store.read(|data| {
        let mut ready: Vec<&cc_protocol::contracts::manage::Package> = data.packages.values().filter(|package| package.repo == repo.id && package.stage == stage::READY).collect();
        // Urgent first, then in the order they became ready.
        ready.sort_by_key(|package| (!package.urgent, package.landed_ms, package.created_ms));
        ready.into_iter().map(|package| package.id.clone()).collect()
    });
    if plan_only {
        let parts = if ready.is_empty() { Vec::new() } else { vec![ReleasedPart { part: repo.source.main.clone(), from: String::new(), to: String::new() }] };
        return to_value(&answer(true, parts, Vec::new(), String::new()));
    }
    let before = head(manager, repo).await.unwrap_or_default();
    let mut tail: Vec<String> = Vec::new();
    let mut held: Vec<String> = Vec::new();
    let mut last = String::new();
    for package in &ready {
        match crate::job::land_now(manager, package).await {
            Ok(commit) => {
                tail.push(manager.t("release_landed", &[("package", package), ("commit", &short(&commit))]));
                last = commit;
            }
            Err(why) => {
                tail.push(manager.t("release_held", &[("package", package), ("why", &why)]));
                held.push(package.clone());
            }
        }
    }
    if last.is_empty() {
        let mut out = to_value(&answer(false, Vec::new(), tail, String::new()))?;
        out["held"] = json!(held);
        return Ok(out);
    }
    let (ok, mut lines) = watch_ci(manager, repo, &last).await;
    tail.append(&mut lines);
    let parts = vec![ReleasedPart { part: repo.source.main.clone(), from: short(&before), to: short(&last) }];
    let mut out = to_value(&answer(ok, parts, tail, last))?;
    out["held"] = json!(held);
    Ok(out)
}

/// A tag on main's head; its CI run is the release.
async fn tag(manager: &Arc<Manager>, repo: &Repo, plan_only: bool) -> Result<Value, RpcError> {
    let delivery = &repo.delivery;
    let version = if delivery.version_kind == "given" { crate::delivery::next_version(manager, repo, &tier_of(manager, &repo.id)).await? } else { short(&head(manager, repo).await?) };
    let name = filled(if delivery.tag.is_empty() { "v{version}" } else { &delivery.tag }, &version);
    let parts = vec![ReleasedPart { part: repo.name.clone(), from: delivery.version_now.clone(), to: version.clone() }];
    if plan_only {
        return to_value(&answer(true, parts, Vec::new(), String::new()));
    }
    let made: Value = manager.peer("vcs.github.tag", json!({"repo": repo.source.github, "connection": repo.source.connection, "branch": repo.source.main, "path": name})).await?;
    let sha = made.get("sha").and_then(Value::as_str).unwrap_or_default().to_owned();
    let mut tail = vec![manager.t("release_tagged", &[("tag", &name), ("commit", &short(&sha))])];
    let (ok, mut lines) = if delivery.ci.is_empty() { (true, Vec::new()) } else { watch_ci(manager, repo, &sha).await };
    tail.append(&mut lines);
    if ok && delivery.version_kind == "given" {
        manager.store.update_repo(&repo.id, |rec| rec.repo.delivery.version_now = version.clone());
    }
    to_value(&answer(ok, parts, tail, sha))
}

fn short(commit: &str) -> String {
    commit.chars().take(7).collect()
}

async fn head(manager: &Manager, repo: &Repo) -> Result<String, RpcError> {
    let found: Value = manager.peer("vcs.github.head", json!({"repo": repo.source.github, "connection": repo.source.connection, "branch": repo.source.main})).await?;
    Ok(found.get("sha").and_then(Value::as_str).unwrap_or_default().to_owned())
}

/// Wait for the CI runs of a commit to finish. True when every run ended well. A run that never
/// starts is a failure, said as such; one that started is waited for as long as it takes.
async fn watch_ci(manager: &Arc<Manager>, repo: &Repo, sha: &str) -> (bool, Vec<String>) {
    if repo.source.github.is_empty() {
        return (true, vec![manager.t("release_no_ci_repo", &[])]);
    }
    let since = cc_protocol::contracts::now_ms();
    let mut tick = tokio::time::interval(CI_LOOK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = manager.stop.cancelled() => return (false, vec![manager.t("release_stopped", &[])]),
            _ = tick.tick() => {}
        }
        let found = manager.peer::<Value>("vcs.github.runs", json!({"repo": repo.source.github, "connection": repo.source.connection, "sha": sha})).await;
        let runs: Vec<Value> = match found {
            Ok(found) => found.get("runs").and_then(Value::as_array).cloned().unwrap_or_default(),
            Err(error) if error.code == codes::UNAVAILABLE || error.code == codes::BUSY => continue,
            Err(error) => return (false, vec![error.message]),
        };
        // Only the runs a push to main starts are the release's.
        let runs: Vec<&Value> = runs.iter().filter(|run| matches!(run.get("event").and_then(Value::as_str), Some("push" | "workflow_dispatch") | None)).collect();
        if runs.is_empty() {
            if cc_protocol::contracts::now_ms().saturating_sub(since) > CI_APPEARS_MS {
                return (false, vec![manager.t("release_no_ci", &[("commit", &short(sha))])]);
            }
            continue;
        }
        let done = runs.iter().all(|run| run.get("status").and_then(Value::as_str) == Some("completed"));
        if !done {
            continue;
        }
        let mut ok = true;
        let mut lines = Vec::new();
        for run in runs {
            let name = run.get("name").and_then(Value::as_str).unwrap_or_default();
            let conclusion = run.get("conclusion").and_then(Value::as_str).unwrap_or_default();
            let url = run.get("url").and_then(Value::as_str).unwrap_or_default();
            ok &= matches!(conclusion, "success" | "skipped" | "neutral");
            lines.push(format!("{name}: {conclusion} {url}").trim().to_owned());
        }
        lines.truncate(TAIL);
        return (ok, lines);
    }
}
