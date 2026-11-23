//! A request's way to its hold: arrived → assessed → planned → validated, then into a package
//! ([`crate::packages`]). It runs as a background task, so `manage.request.create` answers at
//! once. Every question of meaning is one call to the chat flow (`chatflow-manage`), which answers
//! when the model has; everything else here is code.
//!
//! A run goes on from the request's stage, so it can be taken up again after a restart or after a
//! plugin it needs came back. A newer run of the same request (the person said more) replaces an
//! older one, which stops at its next step.

use crate::service::{Manager, retitle};
use crate::store::{PlanRec, ReqRec};
use crate::packages;
use cc_protocol::RpcError;
use cc_protocol::contracts::charter::{self, Charter, Rule};
use cc_protocol::contracts::manage::{
    AssessParams, AssessedParams, Plan, PlanParams, Point, Question, Repo, Tier, ValidateAnswer, ValidateParams, ended, flow, kinds, stage,
};
use cc_protocol::contracts::now_ms;
use cc_protocol::contracts::standard;
use serde_json::json;
use std::sync::Arc;

/// The id a clarify question about a plan's point begins with (`point-3`).
const POINT_QUESTION: &str = "point-";

/// What a repository's charter gives the model. Empty when there is no charter (yet).
#[derive(Debug, Clone, Default)]
pub struct CharterView {
    /// What the app is and what it must never become.
    pub identity: String,
    /// Laws and constraints, one line each.
    pub rules: Vec<String>,
    /// The laws alone.
    pub laws: Vec<String>,
    pub version: String,
    pub digest: String,
}

fn rule_line(rule: &Rule) -> String {
    format!("{}: {}", rule.id, rule.text)
}

/// The charter of a repository. A missing charter (or charter plugin) is tolerated: the work goes
/// on without its lines.
pub async fn charter_of(manager: &Manager, repo: &str) -> CharterView {
    let charter: Charter = match manager.peer(charter::methods::GET, json!({"repo": repo})).await {
        Ok(charter) => charter,
        Err(error) => {
            tracing::info!(repo, code = %error.code, error = %error.message, "no charter for this repository; going on without one");
            return CharterView::default();
        }
    };
    let mut identity = charter.identity.what.trim().to_owned();
    if !charter.identity.who_for.trim().is_empty() {
        identity.push_str(&format!("\nFor: {}", charter.identity.who_for.trim()));
    }
    if !charter.identity.never.is_empty() {
        identity.push_str(&format!("\nIt must never become: {}", charter.identity.never.join("; ")));
    }
    let all = || charter.rules.iter().chain(charter.standard.iter());
    CharterView {
        identity,
        rules: all().filter(|rule| rule.level == charter::level::LAW || rule.level == charter::level::CONSTRAINT).map(rule_line).collect(),
        laws: all().filter(|rule| rule.level == charter::level::LAW).map(rule_line).collect(),
        version: charter.version,
        digest: charter.digest,
    }
}

/// The charter's laws and constraints for these paths, one line each (none when it is missing).
pub async fn rules_for(manager: &Manager, repo: &str, paths: &[String]) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Rules {
        #[serde(default)]
        rules: Vec<Rule>,
    }
    let params = charter::RulesForParams { repo: repo.to_owned(), paths: paths.to_vec(), levels: Vec::new() };
    match manager.peer::<Rules>(charter::methods::RULES_FOR, json!(params)).await {
        Ok(answer) => answer.rules.iter().map(rule_line).collect(),
        Err(error) => {
            tracing::info!(repo, code = %error.code, "no charter rules for these paths");
            Vec::new()
        }
    }
}

/// The repository's name in the standard: its `software` id, or its own id.
pub fn member_of(repo: &Repo) -> String {
    if repo.software.is_empty() { repo.id.clone() } else { repo.software.clone() }
}

/// Which of these paths are standard files, as the standard plugin says.
pub async fn standard_touches(manager: &Manager, repo: &Repo, paths: &[String]) -> Result<Vec<String>, RpcError> {
    #[derive(serde::Deserialize)]
    struct Paths {
        #[serde(default)]
        paths: Vec<String>,
    }
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let params = standard::DiffParams { member: member_of(repo), paths: paths.to_vec() };
    Ok(manager.peer::<Paths>(standard::methods::TOUCHES, json!(params)).await?.paths)
}

/// The tier is the stricter of the model's and the floor; urgent only for a fix or a security
/// change that is not major, unless the person set the switch themselves (a major change is
/// never urgent: it asks at once instead).
pub fn settle_urgent(rec: &mut ReqRec) {
    rec.r.tier = rec.tier_model.max(rec.r.tier_floor);
    let proposed = rec.urgent_model && matches!(rec.r.kind.as_str(), "fix" | "security");
    rec.r.urgent = rec.urgent_person.unwrap_or(proposed) && rec.r.tier != Tier::Major;
}

/// The parts (plugins, `frame`, `protocol` ...) a plan's scopes name.
fn parts_of(points: &[Point]) -> Vec<String> {
    let mut out: Vec<String> = points
        .iter()
        .flat_map(|point| point.touches.iter())
        .filter_map(|touch| {
            let mut names = touch.split('/').filter(|name| !name.is_empty() && *name != ".");
            match (names.next(), names.next()) {
                (Some("plugins"), Some(plugin)) => Some(plugin.to_owned()),
                (Some(first), _) => Some(first.to_owned()),
                _ => None,
            }
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A text with its blanks made single, in lower case: how a quoted sentence is looked for.
fn plain(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

async fn stage_entry(manager: &Manager, session: &str, stage_now: &str) {
    let text = manager.t(&format!("stage_{stage_now}"), &[]);
    manager.entry(session, kinds::STAGE, json!({"stage": stage_now, "text": text})).await;
}

/// Start (or start again) a request's run from the stage it is at.
pub fn spawn(manager: &Arc<Manager>, id: &str) {
    let run = manager.next_run(id);
    let (manager, id) = (manager.clone(), id.to_owned());
    tokio::spawn(async move { go(&manager, &id, run).await });
}

async fn go(manager: &Arc<Manager>, id: &str, run: u64) {
    loop {
        if !manager.is_current(id, run) {
            return;
        }
        let Ok(rec) = manager.request(id) else { return };
        if !rec.r.ended_as.is_empty() {
            return;
        }
        let on = match rec.r.stage.as_str() {
            stage::ARRIVED => assess(manager, &rec, run).await,
            stage::ASSESSED => match packages::merge_same(manager, &rec).await {
                // It was merged into an older request, or the chat flow did not answer.
                Ok(true) | Err(()) => false,
                Ok(false) => manager.is_current(id, run) && plan(manager, &rec, run).await,
            },
            stage::PLANNED => validate(manager, &rec, run).await,
            stage::VALIDATED => {
                packages::place(manager, id).await;
                false
            }
            _ => false,
        };
        if !on {
            return;
        }
    }
}

/// The person wrote more before the build: the text joins the request (when `add`), the package
/// it was in is formed again without it, and it is assessed again. The next plan is a new version.
pub fn again(manager: &Arc<Manager>, id: &str, text: &str, add: bool) {
    packages::take_out(manager, id);
    let updated = manager.store.update_request(id, |rec| {
        if add && !text.is_empty() {
            rec.r.text = format!("{}\n\n{}", rec.r.text.trim_end(), text);
            if !rec.r.fingerprint.is_empty() {
                rec.added = format!("{}\n\n{}", rec.added.trim_end(), text).trim().to_owned();
            }
        }
        rec.plan_note = text.to_owned();
        rec.r.questions.clear();
    });
    manager.store.set_stage(id, stage::ARRIVED);
    if let Some(rec) = updated {
        manager.changed("request", id, &rec.r.repo);
    }
    spawn(manager, id);
}

/// A reason as a sentence: its first letter a capital, a full stop at the end.
pub fn sentence(text: &str) -> String {
    let text = text.trim();
    let mut chars = text.chars();
    let Some(first) = chars.next() else { return String::new() };
    let mut out: String = first.to_uppercase().chain(chars).collect();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// End a request as declined, with the reason in its thread and its ledger. `by`: you | model.
pub async fn decline(manager: &Arc<Manager>, id: &str, reason: &str, by: &str) {
    manager.next_run(id);
    let Some(rec) = manager.store.update_request(id, |rec| {
        rec.r.ended_as = ended::DECLINED.into();
        rec.r.ended_reason = reason.to_owned();
        rec.r.questions.clear();
    }) else {
        return;
    };
    manager.entry(&rec.r.session_id, kinds::STAGE, json!({"stage": ended::DECLINED, "text": manager.t("stage_declined", &[("reason", reason)])})).await;
    manager.line(&rec.r.session_id, &manager.t("line_declined", &[("reason", reason)])).await;
    crate::ledger::note(manager, &rec, "declined", &sentence(reason), by, "");
    manager.changed("request", id, &rec.r.repo);
}

// ---- assess ------------------------------------------------------------------------------------

async fn assess(manager: &Arc<Manager>, rec: &ReqRec, run: u64) -> bool {
    let id = rec.r.id.as_str();
    let session = rec.r.session_id.clone();
    let Ok(repo) = manager.repo(&rec.r.repo) else { return false };
    let charter = charter_of(manager, &repo.repo.id).await;
    let params = AssessParams { session_id: session.clone(), request: id.to_owned(), text: rec.r.text.clone(), repo_name: repo.repo.name.clone(), identity: charter.identity };
    let answer: AssessedParams = match manager.peer(flow::ASSESS, json!(params)).await {
        Ok(answer) => answer,
        Err(error) => {
            manager.unavailable(&format!("request:{id}"), flow::ASSESS, &[session], &error).await;
            return false;
        }
    };
    manager.settled(&format!("request:{id}"));
    if !manager.is_current(id, run) {
        return false;
    }
    if !answer.actionable {
        let mut questions = answer.questions;
        if questions.is_empty() {
            questions.push(Question { id: "what".into(), text: manager.t("clarify_what", &[]), ..Question::default() });
        }
        for (index, question) in questions.iter_mut().enumerate() {
            // An id of ours would be read as an answer about a plan's point.
            if question.id.is_empty() || question.id.starts_with(POINT_QUESTION) {
                question.id = format!("q{}", index + 1);
            }
            question.answer.clear();
        }
        // Nobody waits on a form: the questions are written down and the plan is made from what
        // the request says. The person can add to it in the chat at any time.
        let asked: Vec<String> = questions.iter().map(|question| question.text.clone()).collect();
        manager.note(&session, &manager.t("note_open_questions", &[("questions", &asked.join(" "))])).await;
    }
    // A request an app reported (it declares the failure's fingerprint) is a failure happening in
    // a released app: a fix, and urgent. Its text says what went wrong, not what the fix changes,
    // so a sentence of the report never makes it major; only what a person wrote to it since can.
    let reported = !rec.r.fingerprint.is_empty();
    // The model's sentence for "urgent" must be in the request; one that is not counts as no.
    let quoted = !answer.urgent_because.trim().is_empty() && plain(&rec.r.text).contains(&plain(&answer.urgent_because));
    // Major stops and asks a person, so it needs its sentence in the request too; without one the
    // model's major counts as minor.
    let major_source = if reported { rec.added.as_str() } else { rec.r.text.as_str() };
    let major = answer.tier == Tier::Major && !answer.major_because.trim().is_empty() && plain(major_source).contains(&plain(&answer.major_because));
    let tier_model = if answer.tier == Tier::Major && !major { Tier::Minor } else { answer.tier };
    let tier_because = if major { answer.major_because.trim().to_owned() } else if reported { manager.t("because_reported", &[]) } else { answer.because.trim().to_owned() };
    let tier_because = tier_because.as_str();
    let (kind, urgent_model, urgent_because) = if reported {
        ("fix".to_owned(), true, manager.t("urgent_reported", &[]))
    } else {
        (answer.kind.trim().to_lowercase(), answer.urgent && quoted, if quoted { answer.urgent_because.trim().to_owned() } else { String::new() })
    };
    let new_title = (rec.title_auto && !answer.title.trim().is_empty()).then(|| answer.title.trim().to_owned());
    let model_reason = manager.t("reason_model", &[("tier", tier_model.as_str()), ("because", tier_because)]);
    let floor_reason = manager.t("reason_no_plan", &[]);
    let Some(updated) = manager.store.update_request(id, |rec| {
        rec.r.kind = kind.clone();
        rec.r.surface = answer.surface.clone();
        rec.tier_model = tier_model;
        rec.because = tier_because.to_owned();
        // Before there is a plan there are no paths to judge by: the floor is safe.
        rec.r.tier_floor = Tier::Safe;
        rec.urgent_model = urgent_model;
        rec.r.urgent_because = urgent_because.clone();
        rec.r.tier_reasons = vec![model_reason.clone(), floor_reason.clone()];
        rec.r.questions.clear();
        if let Some(title) = &new_title {
            rec.r.title = title.clone();
            rec.title_auto = false;
        }
        settle_urgent(rec);
    }) else {
        return false;
    };
    if let Some(title) = &new_title {
        retitle(manager, &session, title).await;
    }
    manager.store.set_stage(id, stage::ASSESSED);
    let r = &updated.r;
    manager
        .entry(
            &session,
            kinds::ASSESSMENT,
            json!({
                "kind": r.kind, "tier": r.tier, "tier_floor": r.tier_floor, "reasons": r.tier_reasons, "surface": r.surface,
                "urgent": r.urgent, "urgent_because": r.urgent_because, "because": updated.because, "parts": r.parts,
            }),
        )
        .await;
    let line = if r.urgent { manager.t("line_assessed_urgent", &[("kind", &r.kind), ("tier", r.tier.as_str())]) } else { manager.t("line_assessed", &[("kind", &r.kind), ("tier", r.tier.as_str())]) };
    manager.line(&session, &line).await;
    manager.changed("request", id, &rec.r.repo);
    true
}

// ---- plan --------------------------------------------------------------------------------------

async fn plan(manager: &Arc<Manager>, rec: &ReqRec, run: u64) -> bool {
    let id = rec.r.id.as_str();
    let session = rec.r.session_id.clone();
    let Ok(repo) = manager.repo(&rec.r.repo) else { return false };
    let charter = charter_of(manager, &repo.repo.id).await;
    let params = PlanParams {
        session_id: session.clone(),
        request: id.to_owned(),
        text: rec.r.text.clone(),
        repo: repo.repo.id.clone(),
        repo_name: repo.repo.name.clone(),
        identity: charter.identity,
        rules: charter.rules,
        tier: rec.r.tier,
        kind: rec.r.kind.clone(),
        note: rec.plan_note.clone(),
    };
    let mut plan: Plan = match manager.peer(flow::PLAN, json!(params)).await {
        Ok(plan) => plan,
        Err(error) => {
            manager.unavailable(&format!("request:{id}"), flow::PLAN, &[session], &error).await;
            return false;
        }
    };
    manager.settled(&format!("request:{id}"));
    if !manager.is_current(id, run) {
        return false;
    }
    if plan.points.is_empty() {
        decline(manager, id, &manager.t("declined_nothing", &[]), "model").await;
        return false;
    }
    plan.version = manager.store.read(|data| data.plans.get(id).map(|plans| plans.len() as u32).unwrap_or(0)) + 1;
    plan.created_ms = now_ms();
    plan.note = rec.plan_note.clone();
    // Points are numbered 1..n; a plan whose own numbers do not do that is numbered again.
    let numbered = plan.points.iter().enumerate().all(|(index, point)| point.n == index as u32 + 1);
    if !numbered {
        let old: Vec<u32> = plan.points.iter().map(|point| point.n).collect();
        let unique = old.iter().all(|n| *n != 0 && old.iter().filter(|other| *other == n).count() == 1);
        for (index, point) in plan.points.iter_mut().enumerate() {
            point.n = index as u32 + 1;
            // What it depends on is kept only when the old numbers named points without doubt.
            point.depends_on = if unique { point.depends_on.iter().filter_map(|n| old.iter().position(|o| o == n)).map(|at| at as u32 + 1).collect() } else { Vec::new() };
        }
    }
    for point in &mut plan.points {
        point.request = id.to_owned();
        point.verdict.clear();
        point.verdict_reason.clear();
        for (index, walk) in point.walks.iter_mut().enumerate() {
            if walk.id.is_empty() {
                walk.id = format!("w{}.{}", point.n, index + 1);
            }
            walk.method = walk.method.trim().to_lowercase();
        }
    }
    let parts = parts_of(&plan.points);
    let seq = manager.entry(&session, kinds::PLAN, json!({"plan": plan})).await;
    let count = plan.points.len().to_string();
    let version = plan.version.to_string();
    manager.store.put_plan(id, PlanRec { plan, entry_seq: seq });
    manager.store.update_request(id, |rec| {
        rec.r.parts = parts.clone();
        rec.plan_note.clear();
    });
    manager.store.set_stage(id, stage::PLANNED);
    manager.line(&session, &manager.t("line_planned", &[("count", &count), ("version", &version)])).await;
    manager.changed("request", id, &rec.r.repo);
    true
}

// ---- validate ----------------------------------------------------------------------------------

async fn validate(manager: &Arc<Manager>, rec: &ReqRec, run: u64) -> bool {
    let id = rec.r.id.as_str();
    let session = rec.r.session_id.clone();
    let Ok(repo) = manager.repo(&rec.r.repo) else { return false };
    let Some(mut plan_rec) = manager.store.read(|data| data.plan(id).cloned()) else { return false };
    let charter = charter_of(manager, &repo.repo.id).await;
    let standard_change = rec.r.kind == "standard";
    let mut unfit: Vec<String> = Vec::new();
    // Points the validator would keep but for "no walk proves it" (see below).
    let mut unproven: Vec<usize> = Vec::new();
    // One question per point, one after the other: the model answers one thing at a time.
    for index in 0..plan_rec.plan.points.len() {
        let point = plan_rec.plan.points[index].clone();
        let touched = match standard_touches(manager, &repo.repo, &point.touches).await {
            Ok(paths) => paths,
            Err(error) => {
                manager.unavailable(&format!("request:{id}"), standard::methods::TOUCHES, &[session], &error).await;
                return false;
            }
        };
        let touches_standard = !standard_change && !touched.is_empty();
        let params = ValidateParams {
            session_id: session.clone(),
            text: rec.r.text.clone(),
            point: Point { verdict: String::new(), verdict_reason: String::new(), ..point.clone() },
            identity: charter.identity.clone(),
            laws: charter.laws.clone(),
            touches_standard,
        };
        let answer: ValidateAnswer = match manager.peer(flow::VALIDATE, json!(params)).await {
            Ok(answer) => answer,
            Err(error) => {
                manager.unavailable(&format!("request:{id}"), flow::VALIDATE, &[session], &error).await;
                return false;
            }
        };
        if !manager.is_current(id, run) {
            return false;
        }
        let (verdict, reason) = checked_verdict(manager, &answer, touches_standard);
        if !answer.fits {
            unfit.push(reason.clone());
        }
        if verdict == "ask" && answer.necessary && answer.sensible && answer.fits && answer.standard_safe && !answer.provable {
            unproven.push(index);
        }
        let stored = &mut plan_rec.plan.points[index];
        stored.verdict = verdict;
        stored.verdict_reason = reason;
    }
    // The validator sees one point at a time and no tier. In a safe plan one walk proves several
    // small tasks, so a point whose only fault is that it carries no walk of its own is kept when
    // a kept point of the plan has one.
    let proven = plan_rec.plan.points.iter().any(|point| point.verdict == "keep" && !point.walks.is_empty());
    if rec.r.tier == Tier::Safe && proven {
        for index in unproven {
            let point = &mut plan_rec.plan.points[index];
            point.verdict = "keep".into();
            point.verdict_reason = manager.t("verdict_proven_together", &[("reason", &point.verdict_reason)]);
        }
    }
    // A point the validator is not sure about is a person's to decide only in a major change.
    // In any other the plan's own word stands: the point is kept.
    if rec.r.tier != Tier::Major {
        for point in plan_rec.plan.points.iter_mut().filter(|point| point.verdict == "ask") {
            point.verdict = "keep".into();
            point.verdict_reason = manager.t("verdict_kept_unsure", &[("reason", &point.verdict_reason)]);
        }
    }
    manager.settled(&format!("request:{id}"));
    manager.store.put_plan(id, plan_rec.clone());
    show_verdicts(manager, &session, &plan_rec).await;
    let total = plan_rec.plan.points.len();
    // The request does not fit the app when most of its points do not.
    if unfit.len() * 2 > total {
        let reason = manager.t("declined_unfit", &[("reason", unfit.first().map(String::as_str).unwrap_or_default())]);
        decline(manager, id, &reason, "model").await;
        return false;
    }
    let asks: Vec<&Point> = plan_rec.plan.points.iter().filter(|point| point.verdict == "ask").collect();
    if !asks.is_empty() {
        // A point the validator is not sure about is the person's to decide.
        let questions: Vec<Question> = asks
            .iter()
            .map(|point| Question {
                id: format!("{POINT_QUESTION}{}", point.n),
                text: manager.t("clarify_point", &[("title", &point.title), ("reason", &point.verdict_reason)]),
                options: vec![manager.t("clarify_point_keep", &[]), manager.t("clarify_point_cut", &[])],
                multi: false,
                answer: Vec::new(),
            })
            .collect();
        manager.store.update_request(id, |rec| rec.r.questions = questions.clone());
        manager.store.set_stage(id, stage::CLARIFYING);
        manager.entry(&session, kinds::CLARIFY, json!({"questions": questions})).await;
        manager.line(&session, &manager.t("line_clarify_points", &[])).await;
        manager.changed("request", id, &rec.r.repo);
        return false;
    }
    finish(manager, id, run).await
}

/// The host's checks on a verdict: one of the four words, with a reason, and never "keep" for a
/// point that reaches standard files in a change that is not a standard change. What fails
/// becomes the cautious choice, `ask`.
fn checked_verdict(manager: &Manager, answer: &ValidateAnswer, touches_standard: bool) -> (String, String) {
    let verdict = answer.verdict.trim().to_lowercase();
    let reason = answer.reason.trim().to_owned();
    if !matches!(verdict.as_str(), "keep" | "cut" | "merge" | "ask") {
        return ("ask".into(), if reason.is_empty() { manager.t("verdict_unclear", &[]) } else { reason });
    }
    if reason.is_empty() {
        return ("ask".into(), manager.t("verdict_no_reason", &[]));
    }
    if verdict == "keep" && touches_standard && !answer.standard_safe {
        return ("ask".into(), manager.t("verdict_standard", &[("reason", &reason)]));
    }
    (verdict, reason)
}

/// Put the verdicts into the plan's entry in the thread.
async fn show_verdicts(manager: &Manager, session: &str, plan: &PlanRec) {
    if plan.entry_seq == 0 {
        return;
    }
    let Ok(host) = manager.host() else { return };
    if let Err(error) = host.journal_set(session, plan.entry_seq, "plan", json!(plan.plan)).await {
        tracing::warn!(session, error = %error.message, "the plan's entry was not updated with its verdicts");
    }
}

/// The person answered the clarify form.
pub async fn answered(manager: &Arc<Manager>, rec: &ReqRec) {
    let id = rec.r.id.as_str();
    let about_points = !rec.r.questions.is_empty() && rec.r.questions.iter().all(|question| question.id.starts_with(POINT_QUESTION));
    if !about_points {
        // The answers become part of the request, and it is assessed again.
        let said: Vec<String> = rec.r.questions.iter().map(|question| format!("{} {}", question.text, question.answer.join(", "))).collect();
        again(manager, id, &said.join("\n"), true);
        return;
    }
    let Some(mut plan) = manager.store.read(|data| data.plan(id).cloned()) else { return };
    for question in &rec.r.questions {
        let Some(n) = question.id.strip_prefix(POINT_QUESTION).and_then(|n| n.parse::<u32>().ok()) else { continue };
        // The first option is "keep it"; anything else leaves the point out.
        let keep = question.answer.first() == question.options.first();
        if let Some(point) = plan.plan.points.iter_mut().find(|point| point.n == n) {
            point.verdict = if keep { "keep".into() } else { "cut".into() };
            point.verdict_reason = manager.t(if keep { "verdict_person_keep" } else { "verdict_person_cut" }, &[("reason", &point.verdict_reason)]);
        }
    }
    manager.store.put_plan(id, plan.clone());
    show_verdicts(manager, &rec.r.session_id, &plan).await;
    manager.store.update_request(id, |rec| rec.r.questions.clear());
    let run = manager.next_run(id);
    let (manager, id) = (manager.clone(), id.to_owned());
    tokio::spawn(async move {
        if finish(&manager, &id, run).await {
            go(&manager, &id, run).await;
        }
    });
}

/// After the verdicts are settled: what is left, the floor from the kept points, then validated.
async fn finish(manager: &Arc<Manager>, id: &str, run: u64) -> bool {
    let Ok(rec) = manager.request(id) else { return false };
    let session = rec.r.session_id.clone();
    let kept = manager.store.read(|data| data.kept(id));
    if kept.is_empty() {
        decline(manager, id, &manager.t("declined_nothing", &[]), "model").await;
        return false;
    }
    manager.settled(&format!("request:{id}"));
    if !manager.is_current(id, run) {
        return false;
    }
    // The tier is the model's: how hard and how far-reaching the work is, never which folders it
    // touches.
    let parts = parts_of(&kept);
    let Some(updated) = manager.store.update_request(id, |rec| {
        rec.r.tier_floor = Tier::Safe;
        rec.r.tier_reasons = vec![manager.t("reason_model", &[("tier", rec.tier_model.as_str()), ("because", &rec.because)])];
        rec.r.parts = parts.clone();
        settle_urgent(rec);
    }) else {
        return false;
    };
    manager.store.set_stage(id, stage::VALIDATED);
    stage_entry(manager, &session, stage::VALIDATED).await;
    let cut = manager.store.read(|data| data.plan(id).map(|plan| plan.plan.points.len()).unwrap_or(0)).saturating_sub(kept.len());
    manager
        .line(&session, &manager.t("line_validated", &[("kept", &kept.len().to_string()), ("cut", &cut.to_string()), ("tier", updated.r.tier.as_str())]))
        .await;
    manager.changed("request", id, &rec.r.repo);
    true
}
