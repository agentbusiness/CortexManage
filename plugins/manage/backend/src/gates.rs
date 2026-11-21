//! The gates a finished job passes before it lands: scope, standard, charter, build, walks,
//! converge. Each answers passed, failed, blocked or not_run and is recorded with what it found:
//! never a silent pass. Code decides all of them but one (converge, one model question on the
//! whole diff).
//!
//! A walk is run by whatever can perform its method. In this version that is the runner for
//! `command` walks; nothing on the mesh performs `computer`, `mesh` or `journal` walks yet, so
//! those are `blocked`, and a package with a blocked walk lands only with a person's yes.

use crate::packages::{self, Joined};
use crate::service::{Manager, RUNNER_BUILD, RUNNER_CALL};
use crate::{diff, job, pipeline, policy};
use cc_protocol::RpcError;
use cc_protocol::contracts::charter;
use cc_protocol::contracts::manage::{ConvergeAnswer, ConvergeParams, Evidence, Finding, Gate, flow, kinds};
use cc_protocol::contracts::now_ms;
use cc_protocol::contracts::runner::{self, Facts};
use cc_protocol::contracts::standard;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// How long one walk's command may take.
const WALK_MS: u64 = 120_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// Every gate.
    All,
    /// Build and walks only: after a rebase the code is the same, the base is not.
    Proof,
}

#[derive(Default)]
pub struct Outcome {
    /// What failed: the task to start again and the finding to give it.
    pub failed: Vec<(u32, String)>,
    /// A walk could not be performed (or the plan names none).
    pub blocked: bool,
    /// A gate could not be asked: the method that failed and its error. The gates run again
    /// when it is back.
    pub unavailable: Option<(String, RpcError)>,
    /// One line per gate, for the person who is asked to approve.
    pub summary: Vec<String>,
}

/// The task a changed file belongs to: the one whose scope holds it, else the one whose scope
/// shares the longest path with it, else the last.
pub fn task_for_file(points: &[Joined], file: &str) -> u32 {
    if let Some(joined) = points.iter().find(|joined| joined.point.touches.iter().any(|touch| policy::inside(touch, file))) {
        return joined.point.n;
    }
    let shared = |touch: &str| touch.split('/').zip(file.split('/')).take_while(|(a, b)| a == b).count();
    points
        .iter()
        .map(|joined| (joined.point.touches.iter().map(|touch| shared(touch)).max().unwrap_or(0), joined.point.n))
        .filter(|(shared, _)| *shared > 0)
        .max_by_key(|(shared, n)| (*shared, std::cmp::Reverse(*n)))
        .map(|(_, n)| n)
        .unwrap_or_else(|| points.last().map(|joined| joined.point.n).unwrap_or(1))
}

/// Record a gate's answer in the graph and in every thread of the package.
pub async fn record(manager: &Manager, package: &str, kind: &str, result: &str, findings: Vec<Finding>) {
    let Ok(found) = manager.package(package) else { return };
    let gate = Gate { kind: kind.into(), attempt: found.attempt, result: result.into(), findings, ms: now_ms() };
    manager.store.add_gate(package, gate.clone());
    let sessions = manager.store.read(|data| data.sessions_of(&found));
    manager.entry_all(&sessions, kinds::GATE, json!(gate)).await;
    manager.changed("package", package, &found.repo);
}

fn finding(title: String, evidence: &str) -> Finding {
    Finding { title, evidence: evidence.to_owned() }
}

pub async fn run(manager: &Arc<Manager>, package_id: &str, facts: &Facts, which: Which) -> Outcome {
    let mut outcome = Outcome::default();
    let Ok(package) = manager.package(package_id) else { return outcome };
    let Ok(repo) = manager.repo(&package.repo) else { return outcome };
    let repo = repo.repo;
    let node = Manager::node_of(&package, &repo);
    let app = job::app_of(&repo);
    let (points, originals) = manager.store.read(|data| {
        let points = packages::joined(data, &package);
        // The joined points carry their walks (the builder's tasks are made without them).
        let originals: Vec<cc_protocol::contracts::manage::Point> = points.clone().into_iter().map(|joined| joined.point).collect();
        (points, originals)
    });
    let (standard_change, _) = job::special(manager, &package);
    let last = points.last().map(|joined| joined.point.n).unwrap_or(1);
    macro_rules! wait_for {
        ($method:expr, $error:expr) => {{
            outcome.unavailable = Some(($method.to_owned(), $error));
            return outcome;
        }};
    }

    let whole: runner::DiffAnswer = match manager.runner(&node, runner::methods::DIFF, json!({"job_id": package.job_id, "whole": true, "limit": u32::MAX}), RUNNER_CALL).await {
        Ok(whole) => whole,
        Err(error) => wait_for!(runner::methods::DIFF, error),
    };
    let files = if whole.files.is_empty() { diff::files(&whole.diff) } else { whole.files.clone() };

    if which == Which::All {
        // Scope: nothing changed outside every task's scope.
        if facts.out_of_scope.is_empty() {
            record(manager, package_id, "scope", "passed", Vec::new()).await;
            outcome.summary.push(manager.t("proof_scope", &[]));
        } else {
            let findings = facts.out_of_scope.iter().map(|file| finding(manager.t("gate_scope", &[("file", file)]), file)).collect();
            record(manager, package_id, "scope", "failed", findings).await;
            for file in &facts.out_of_scope {
                outcome.failed.push((task_for_file(&points, file), manager.t("rework_scope", &[("file", file)])));
            }
        }

        // Standard: no standard file changed, unless this is a standard change.
        #[derive(serde::Deserialize)]
        struct Paths {
            #[serde(default)]
            paths: Vec<String>,
        }
        let touched: Vec<String> = if files.is_empty() {
            Vec::new()
        } else {
            match manager.peer::<Paths>(standard::methods::TOUCHES, json!(standard::DiffParams { member: pipeline::member_of(&repo), paths: files.clone() })).await {
                Ok(answer) => answer.paths,
                Err(error) => wait_for!(standard::methods::TOUCHES, error),
            }
        };
        if touched.is_empty() || standard_change {
            record(manager, package_id, "standard", "passed", Vec::new()).await;
        } else {
            let findings = touched.iter().map(|file| finding(manager.t("gate_standard", &[("file", file)]), file)).collect();
            record(manager, package_id, "standard", "failed", findings).await;
            for file in &touched {
                outcome.failed.push((task_for_file(&points, file), manager.t("rework_standard", &[("file", file)])));
            }
        }

        // Charter: the checks kept by code, over the whole diff. A check that is not in `ran`
        // did not run, so "ok" with nothing run is not a pass.
        let check = charter::CheckParams { repo: repo.id.clone(), diff: whole.diff.clone(), files: files.clone(), standard_change };
        let checked: charter::CheckAnswer = match manager.peer(charter::methods::CHECK, json!(check)).await {
            Ok(checked) => checked,
            Err(error) => wait_for!(charter::methods::CHECK, error),
        };
        if !checked.findings.is_empty() || !checked.ok {
            let mut findings: Vec<Finding> =
                checked.findings.iter().map(|found| finding(manager.t("gate_charter", &[("rule", &found.rule), ("check", &found.check)]), &format!("{}:{} {}", found.file, found.line, found.text))).collect();
            if findings.is_empty() {
                findings.push(finding(manager.t("gate_charter_failed", &[]), &checked.ran.join(", ")));
            }
            record(manager, package_id, "charter", "failed", findings).await;
            for found in &checked.findings {
                outcome.failed.push((task_for_file(&points, &found.file), manager.t("rework_charter", &[("rule", &found.rule), ("file", &found.file), ("text", &found.text)])));
            }
            if checked.findings.is_empty() {
                outcome.failed.push((last, manager.t("gate_charter_failed", &[])));
            }
        } else if checked.ran.is_empty() {
            record(manager, package_id, "charter", "not_run", vec![finding(manager.t("gate_charter_none", &[]), "")]).await;
            outcome.summary.push(manager.t("gate_charter_none", &[]));
        } else {
            record(manager, package_id, "charter", "passed", Vec::new()).await;
            outcome.summary.push(manager.t("proof_charter", &[("count", &checked.ran.len().to_string())]));
        }
        // What code already found is fixed first: no build of a change that will be redone.
        if !outcome.failed.is_empty() {
            return outcome;
        }
    }

    // Build: the changed parts into the proving home.
    let build = runner::BuildParams { job_id: package.job_id.clone(), attempt: package.attempt, app: app.clone(), parts: Vec::new(), soak: false };
    let built: runner::BuildAnswer = match manager.runner(&node, runner::methods::BUILD, json!(build), RUNNER_BUILD).await {
        Ok(built) => built,
        Err(error) if job::away(&error) => wait_for!(runner::methods::BUILD, error),
        Err(error) => runner::BuildAnswer { ok: false, parts: Vec::new(), tail: vec![error.message], build: String::new() },
    };
    if !built.ok {
        let tail = built.tail.join("\n");
        record(manager, package_id, "build", "failed", vec![finding(manager.t("gate_build", &[]), &tail)]).await;
        record(manager, package_id, "walks", "not_run", vec![finding(manager.t("gate_walks_no_build", &[]), "")]).await;
        outcome.failed.push((last, manager.t("rework_build", &[("tail", &diff::tail(&tail, 12))])));
        return outcome;
    }
    record(manager, package_id, "build", "passed", Vec::new()).await;
    outcome.summary.push(manager.t("proof_build", &[("parts", &built.parts.join(", "))]));

    // Walks: each one, by what can perform its method.
    let (mut passed, mut blocked, mut failed) = (0, 0, 0);
    let mut findings: Vec<Finding> = Vec::new();
    for point in &originals {
        for walk in &point.walks {
            let (result, seen, output) = if walk.method == "command" && !walk.command.trim().is_empty() {
                let command = runner::CommandParams { job_id: package.job_id.clone(), app: app.clone(), command: walk.command.clone(), timeout_ms: Some(WALK_MS) };
                match manager.runner::<runner::CommandAnswer>(&node, runner::methods::COMMAND, json!(command), Duration::from_millis(WALK_MS + 30_000)).await {
                    Ok(answer) => {
                        // What is seen must be in the output; a walk that names nothing to see
                        // passes when its command does.
                        let ok = if walk.expect.is_empty() { answer.exit_code == 0 } else { answer.output.contains(&walk.expect) };
                        (if ok { "passed" } else { "failed" }, diff::tail(&answer.output, 6), Some(answer.output))
                    }
                    Err(error) if job::away(&error) => wait_for!(runner::methods::COMMAND, error),
                    Err(error) => ("failed", error.message, None),
                }
            } else if walk.method == "command" {
                ("blocked", manager.t("walk_no_command", &[]), None)
            } else {
                ("blocked", manager.t("walk_blocked", &[("method", &walk.method)]), None)
            };
            let id = manager.store.next_evidence_id();
            if let Some(output) = output {
                // The bytes are not a record: they live in the plugin's data folder.
                let folder = manager.data_dir.join("evidence");
                if let Err(error) = tokio::fs::create_dir_all(&folder).await {
                    tracing::warn!(%error, "the evidence folder cannot be made");
                } else if let Err(error) = tokio::fs::write(folder.join(format!("{id}.log")), output).await {
                    tracing::warn!(%error, "a walk's output was not kept");
                }
            }
            let evidence = Evidence {
                id,
                walk: walk.id.clone(),
                request: point.request.clone(),
                kind: if walk.evidence_kind.is_empty() { "log".into() } else { walk.evidence_kind.clone() },
                result: result.into(),
                seen: seen.clone(),
                build: built.build.clone(),
                ms: now_ms(),
            };
            manager.store.add_evidence(package_id, evidence);
            match result {
                "passed" => passed += 1,
                "blocked" => {
                    blocked += 1;
                    findings.push(finding(walk.do_.clone(), &seen));
                }
                _ => {
                    failed += 1;
                    findings.push(finding(walk.do_.clone(), &seen));
                    outcome.failed.push((point.n, manager.t("rework_walk", &[("do", &walk.do_), ("see", &walk.see), ("seen", &seen)])));
                }
            }
        }
    }
    let (passed_text, failed_text, blocked_text) = (passed.to_string(), failed.to_string(), blocked.to_string());
    let counts = [("passed", passed_text.as_str()), ("failed", failed_text.as_str()), ("blocked", blocked_text.as_str())];
    if failed > 0 {
        record(manager, package_id, "walks", "failed", findings).await;
    } else if blocked > 0 {
        outcome.blocked = true;
        record(manager, package_id, "walks", "blocked", findings).await;
        outcome.summary.push(manager.t("proof_walks_blocked", &counts));
    } else if passed == 0 {
        // A plan with no walk proves nothing by itself.
        outcome.blocked = true;
        record(manager, package_id, "walks", "not_run", vec![finding(manager.t("walk_none", &[]), "")]).await;
        outcome.summary.push(manager.t("walk_none", &[]));
    } else {
        record(manager, package_id, "walks", "passed", Vec::new()).await;
        outcome.summary.push(manager.t("proof_walks", &counts));
    }
    if failed > 0 || which == Which::Proof {
        return outcome;
    }

    // Converge: does the whole diff do the plan, and only the plan?
    let params = ConvergeParams { points: points.iter().map(|joined| (joined.point.n, joined.point.title.clone(), joined.point.rules.clone())).collect(), diff: whole.diff.clone() };
    let answer: ConvergeAnswer = match manager.peer(flow::CONVERGE, json!(params)).await {
        Ok(answer) => answer,
        Err(error) => wait_for!(flow::CONVERGE, error),
    };
    if answer.extra.is_empty() && answer.missing.is_empty() {
        record(manager, package_id, "converge", "passed", Vec::new()).await;
        outcome.summary.push(manager.t("proof_converge", &[]));
    } else {
        let mut findings: Vec<Finding> = Vec::new();
        for (file, why) in &answer.extra {
            findings.push(finding(manager.t("gate_converge_extra", &[("file", file)]), why));
            outcome.failed.push((task_for_file(&points, file), manager.t("rework_extra", &[("file", file), ("why", why)])));
        }
        for (n, why) in &answer.missing {
            let title = points.iter().find(|joined| joined.point.n == *n).map(|joined| joined.point.title.clone()).unwrap_or_default();
            findings.push(finding(manager.t("gate_converge_missing", &[("title", &title)]), why));
            let task = if points.iter().any(|joined| joined.point.n == *n) { *n } else { last };
            outcome.failed.push((task, manager.t("rework_missing", &[("title", &title), ("why", why)])));
        }
        record(manager, package_id, "converge", "failed", findings).await;
    }
    outcome
}
