//! A package's job on its runner (docs/specs/01-WORK.md §4-6): dispatch, the beats, the watch
//! rules, and what follows when the builder is done (the gates and the landing, in
//! [`crate::gates`] and below).
//!
//! Each running package has one task of its own that takes its messages in order: the runner's
//! beats, the clock's ticks, a person's decisions. The runner computes the facts; the rules here
//! are plain code over them. The model is asked two things only: after a checkpoint, whether the
//! diff does what the task says; and for a held replan, whether each new task may be kept.
//!
//! The ladder is the same for every rule, per rule and task: steer once, restart once, then a
//! person. A steer is a fixed sentence of `data/texts.toml` with the fact filled in, ending with
//! the step to take next. Nothing is steered while the builder waits on an approval: a stray
//! message there could be read as the answer.

use crate::gates::{self, Outcome, Which};
use crate::inbox::{self, NewAsk};
use crate::packages::{self, Joined};
use crate::service::{Manager, RUNNER_BUILD, RUNNER_CALL, RUNNER_LAND, RUNNER_PREPARE};
use crate::store::AskRec;
use crate::{diff, pipeline, policy};
use cc_protocol::contracts::charter;
use cc_protocol::contracts::manage::{DriftAnswer, DriftParams, Intervention, Package, Point, Repo, Strip, Tier, ValidateAnswer, ValidateParams, WatchBeat, flow, kinds, stage, topics};
use cc_protocol::contracts::now_ms;
use cc_protocol::contracts::runner::{self, Checkpoint, Facts, HeldApproval, HeldReplan, Task};
use cc_protocol::contracts::standard;
use cc_protocol::contracts::train;
use cc_protocol::{RpcError, codes};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// The approval classes a package's plan lists. Plans list none in this version, so every
/// approval question is refused with a sentence saying what to do instead.
const APPROVE: &[&str] = &[];
/// Three missed beats (they come every 20 s).
const LOST_AFTER_MS: u64 = 60_000;
/// The same rule does not fire again on the same task sooner than this unless its fact went away
/// in between: a steer needs a moment to be read.
const COOLDOWN_MS: u64 = 180_000;
/// After a restart, an old "done" or "stopped" is not believed for this long unless the job was
/// seen running in between.
const SETTLE_MS: u64 = 60_000;
/// W1, W2, W3: the thresholds of spec 01 §5.2.
const SAME_FAILURE: u32 = 3;
const SAME_CALL: u32 = 4;
const CALLS_WITHOUT_EDIT: u32 = 10;
const IDLE_MS: u64 = 10 * 60_000;
/// How often a task is started again because its context was full before a person is asked.
const CONTEXT_RESTARTS: u32 = 2;
/// How often a landing is tried again after a rebase before a person is asked.
const LAND_TRIES: u32 = 3;
const RULES: [&str; 8] = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];

pub enum Msg {
    Beat(Box<Facts>),
    /// The runner can be called again.
    Up,
    Tick,
    /// A plugin the gates waited for may be back.
    Retry,
    /// The package may go on (Start now, or the Hold switch went off).
    Resume,
    /// The package was withdrawn: this task ends.
    Cancel,
    Decided {
        ask: Box<AskRec>,
    },
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The package's task, started when there is none.
pub fn ensure(manager: &Arc<Manager>, package: &str) -> mpsc::UnboundedSender<Msg> {
    let mut jobs = lock(&manager.jobs);
    if let Some(sender) = jobs.get(package)
        && !sender.is_closed()
    {
        return sender.clone();
    }
    let (sender, receive) = mpsc::unbounded_channel();
    jobs.insert(package.to_owned(), sender.clone());
    let actor = Actor::new(manager.clone(), package, receive);
    tokio::spawn(actor.run());
    sender
}

pub fn tell(manager: &Arc<Manager>, package: &str, msg: Msg) {
    let _ = ensure(manager, package).send(msg);
}

/// Tell every running job.
pub fn tell_all(manager: &Arc<Manager>, make: impl Fn() -> Msg) {
    for sender in lock(&manager.jobs).values() {
        let _ = sender.send(make());
    }
}

pub fn tick(manager: &Arc<Manager>) {
    tell_all(manager, || Msg::Tick);
}

/// Whether a dispatch of this package is under way right now.
pub fn dispatching(manager: &Manager, package: &str) -> bool {
    lock(&manager.dispatching).contains(package)
}

/// A beat arrived on a link: answer it at once (the answer carries what the manager wants), then
/// hand the facts to the package's task.
pub fn beat(manager: &Arc<Manager>, facts: Facts) {
    let found = manager.store.read(|data| {
        let package = data.packages.values().find(|package| !package.job_id.is_empty() && package.job_id == facts.job_id)?;
        Some((package.clone(), data.hold, data.repos.get(&package.repo).map(|rec| Manager::node_of(package, &rec.repo)).unwrap_or_default()))
    });
    let Some((package, hold, node)) = found else { return };
    let live = matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING);
    let want = if package.stopped || hold || !live { "stop" } else { "continue" };
    let answer = runner::BeatAnswer { job_id: facts.job_id.clone(), attempt: facts.attempt, want: want.into() };
    {
        let manager = manager.clone();
        tokio::spawn(async move {
            if let Err(error) = manager.runner::<Value>(&node, runner::methods::BEAT_ANSWER, json!(answer), Duration::from_secs(20)).await {
                tracing::debug!(job = %answer.job_id, error = %error.message, "a beat was not answered");
            }
        });
    }
    if live && !package.session_id.is_empty() {
        tell(manager, &package.id, Msg::Beat(Box::new(facts)));
    }
}

/// A runner is back: its jobs ask for their facts, and packages that waited for it may start.
pub fn runner_up(manager: &Arc<Manager>, node: &str) {
    let packages: Vec<String> = manager.store.read(|data| {
        data.packages
            .values()
            .filter(|package| matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING) && !package.session_id.is_empty())
            .filter(|package| data.repos.get(&package.repo).is_some_and(|rec| Manager::node_of(package, &rec.repo) == node))
            .map(|package| package.id.clone())
            .collect()
    });
    for id in packages {
        tell(manager, &id, Msg::Up);
    }
    manager.look_now();
}

/// Move a package and its requests to a stage, with the stage's line in every thread.
pub async fn set_stage(manager: &Manager, package: &str, stage_now: &str) {
    let Some(updated) = manager.store.update_package(package, |package| package.stage = stage_now.to_owned()) else { return };
    let text = manager.t(&format!("stage_{stage_now}"), &[("package", package)]);
    for request in &updated.requests {
        let before = manager.store.read(|data| data.requests.get(request).map(|rec| rec.r.stage.clone()));
        if before.as_deref() == Some(stage_now) {
            continue;
        }
        if let Some(rec) = manager.store.set_stage(request, stage_now) {
            manager.entry(&rec.r.session_id, kinds::STAGE, json!({"stage": stage_now, "text": text})).await;
            manager.changed("request", request, &updated.repo);
        }
    }
    manager.changed("package", package, &updated.repo);
    manager.changed("inbox", "", &updated.repo);
}

// ---- dispatch ----------------------------------------------------------------------------------

/// Why a dispatch did not happen.
enum Fail {
    /// Something is not available: the package stays held and is tried again.
    Wait(&'static str, RpcError),
    /// A person has to look.
    Ask(String),
}

/// Whether an error says "cannot be reached" rather than "refused".
pub fn away(error: &RpcError) -> bool {
    matches!(error.code.as_str(), codes::UNAVAILABLE | codes::CONNECTION_CLOSED | codes::TIMEOUT | codes::DRAINING | codes::BUSY)
}

/// Start a package's job in the background. The clock already marked it as building.
pub fn dispatch(manager: &Arc<Manager>, id: &str) {
    lock(&manager.dispatching).insert(id.to_owned());
    let (manager, id) = (manager.clone(), id.to_owned());
    tokio::spawn(async move {
        let outcome = start(&manager, &id).await;
        lock(&manager.dispatching).remove(&id);
        let key = format!("runner:{id}");
        match outcome {
            Ok(()) => {
                manager.settled(&key);
                ensure(&manager, &id);
            }
            Err(fail) => {
                set_stage(&manager, &id, stage::HELD).await;
                let Ok(package) = manager.package(&id) else { return };
                let sessions = manager.store.read(|data| data.sessions_of(&package));
                match fail {
                    Fail::Wait(method, error) => {
                        // Not at every tick: the package is looked at again in a while.
                        manager.store.update_package(&id, |package| package.hold_until_ms = now_ms() + 60_000);
                        manager.unavailable(&key, method, &sessions, &error).await;
                    }
                    Fail::Ask(text) => {
                        manager.settled(&key);
                        let title = manager.store.read(|data| package.requests.first().and_then(|id| data.requests.get(id)).map(|rec| rec.r.title.clone()).unwrap_or_default());
                        inbox::raise(
                            &manager,
                            NewAsk {
                                kind: "stuck".into(),
                                repo: package.repo.clone(),
                                package: id.clone(),
                                request: package.requests.first().cloned().unwrap_or_default(),
                                subject: manager.t("subject_not_started", &[("title", &title)]),
                                title,
                                detail: text,
                                actions: inbox::actions(&["rerun", "take_over", "decline"]),
                                attempt: package.attempt,
                                resume: "dispatch".into(),
                                ..NewAsk::default()
                            },
                        )
                        .await;
                    }
                }
            }
        }
    });
}

/// `Copy button on command output` -> `copy-button-on-command-output` (at most 40 characters).
fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() { "change".into() } else { out }
}

/// The app's id on the runner (its proving home is `~/.cortex-proving/<app>`).
pub fn app_of(repo: &Repo) -> String {
    pipeline::member_of(repo)
}

/// Whether the package may change the standard's files (`AGENTS.md` too for a charter change).
pub fn special(manager: &Manager, package: &Package) -> (bool, bool) {
    manager.store.read(|data| {
        let kinds: Vec<&str> = package.requests.iter().filter_map(|id| data.requests.get(id)).map(|rec| rec.r.kind.as_str()).collect();
        (kinds.contains(&"standard"), kinds.contains(&"charter"))
    })
}

async fn start(manager: &Arc<Manager>, id: &str) -> Result<(), Fail> {
    let package = manager.package(id).map_err(|error| Fail::Ask(error.message))?;
    let repo = manager.repo(&package.repo).map_err(|error| Fail::Ask(error.message))?.repo;
    // The computer it builds on: the one it already has (a restart goes on where its worktree
    // is), the repository's, or any free CortexCode. None free, or one just started: wait.
    let node = if !package.runner.is_empty() {
        package.runner.clone()
    } else {
        let (node, name) = match manager.pick_runner(&repo, true).await {
            Ok(picked) => picked,
            Err(error) if error.code == codes::UNAVAILABLE => return Err(Fail::Wait("runner.start", error)),
            Err(error) => return Err(Fail::Ask(error.message)),
        };
        manager.store.update_package(id, |package| {
            package.runner = node.clone();
            package.runner_name = name.clone();
        });
        node
    };
    let on_runner = |error: RpcError| if away(&error) { Fail::Wait("runner.prepare", error) } else { Fail::Ask(error.message) };
    let (points, first_title, goal, approach) = manager.store.read(|data| {
        let first = package.requests.first().cloned().unwrap_or_default();
        let title = data.requests.get(&first).map(|rec| rec.r.title.clone()).unwrap_or_default();
        let plans: Vec<&cc_protocol::contracts::manage::Plan> = package.requests.iter().filter_map(|id| data.plan(id)).map(|rec| &rec.plan).collect();
        let goal = plans.iter().map(|plan| plan.goal.trim()).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n");
        let approach = plans.iter().map(|plan| plan.approach.trim()).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n");
        (packages::joined(data, &package), title, goal, approach)
    });
    if points.is_empty() {
        return Err(Fail::Ask(manager.t("declined_nothing", &[])));
    }
    let (standard_change, charter_change) = special(manager, &package);
    // What no task may change: the standard's files, `AGENTS.md` and the lock.
    let protected = if standard_change || charter_change {
        Vec::new()
    } else {
        #[derive(serde::Deserialize)]
        struct Paths {
            #[serde(default)]
            paths: Vec<String>,
        }
        let answer: Paths = manager
            .peer(standard::methods::PATHS, json!({"member": pipeline::member_of(&repo)}))
            .await
            .map_err(|error| Fail::Wait(standard::methods::PATHS, error))?;
        let mut paths = answer.paths;
        for own in ["AGENTS.md", "standard.lock"] {
            if !paths.iter().any(|path| path == own) {
                paths.push(own.to_owned());
            }
        }
        paths
    };
    let charter = pipeline::charter_of(manager, &repo.id).await;
    // The builder gets tasks, never walks: those stay with the manager.
    let mut tasks: Vec<Task> = Vec::with_capacity(points.len());
    for joined in &points {
        let point = &joined.point;
        let mut known = point.known.clone();
        for rule in pipeline::rules_for(manager, &repo.id, &point.touches).await {
            if !known.contains(&rule) {
                known.push(rule);
            }
        }
        tasks.push(Task {
            n: point.n,
            title: point.title.clone(),
            brief: point.brief.clone(),
            known,
            rules: point.rules.clone(),
            depends_on: point.depends_on.clone(),
            touches: point.touches.clone(),
            checks: point.checks.clone(),
            request: point.request.clone(),
        });
    }
    let mut trailers = vec![format!("Package: {}", package.id)];
    if !charter.version.is_empty() {
        let digest: String = charter.digest.chars().take(4).collect();
        trailers.push(format!("Charter: {} {} ({digest}…)", repo.id, charter.version));
    }
    manager.links.ensure(manager.ctx().map_err(|error| Fail::Wait("runner.prepare", error))?, &node);

    // A job the runner already knows (it was stopped for a new hold) keeps its workspace and
    // starts as the next attempt; a new one gets its worktree first.
    let job_id = if package.job_id.is_empty() { package.id.clone() } else { package.job_id.clone() };
    let known: Option<Facts> = match manager.runner::<Facts>(&node, runner::methods::GET, json!({"job_id": job_id}), RUNNER_CALL).await {
        Ok(facts) if !facts.project_id.is_empty() => Some(facts),
        Ok(_) => None,
        Err(error) if away(&error) => return Err(Fail::Wait("runner.get", error)),
        Err(_) => None,
    };
    let branch = if package.branch.is_empty() { format!("cm/{}-{}", package.id, slug(&first_title)) } else { package.branch.clone() };
    let (project_id, base, attempt) = match known {
        Some(facts) => (facts.project_id, package.base.clone(), facts.attempt.max(package.attempt) + 1),
        None => {
            let prepare = runner::PrepareParams { job_id: job_id.clone(), repo: repo.id.clone(), url: repo.source.url.clone(), main: repo.source.main.clone(), branch: branch.clone(), base: String::new(), commit_name: repo.source.commit_name.clone(), commit_email: repo.source.commit_email.clone() };
            let prepared: runner::Prepared = manager.runner(&node, runner::methods::PREPARE, json!(prepare), RUNNER_PREPARE).await.map_err(on_runner)?;
            (prepared.project_id, prepared.base, 1)
        }
    };
    let title = match package.requests.len() {
        0 | 1 => first_title.clone(),
        count => manager.t("title_bundle", &[("title", &first_title), ("more", &(count - 1).to_string())]),
    };
    let params = runner::StartParams {
        job_id: job_id.clone(),
        attempt,
        project_id,
        title,
        goal,
        approach,
        tasks,
        protected,
        standard_change,
        approve: APPROVE.iter().map(|class| (*class).to_owned()).collect(),
        trailers,
    };
    let started: runner::Started = manager.runner(&node, runner::methods::START, json!(params), RUNNER_CALL).await.map_err(|error| if away(&error) { Fail::Wait("runner.start", error) } else { Fail::Ask(error.message) })?;
    let Some(updated) = manager.store.update_package(id, |package| {
        package.job_id = job_id.clone();
        package.attempt = attempt;
        package.branch = branch.clone();
        package.base = base.clone();
        package.session_id = started.session_id.clone();
        package.stage = stage::BUILDING.into();
    }) else {
        return Ok(());
    };
    // The requests move only now that the runner has the plan: a start that failed leaves them held.
    for request in &updated.requests {
        manager.store.set_stage(request, stage::BUILDING);
    }
    let text = manager.t("stage_building", &[("package", id)]);
    let sessions = manager.store.read(|data| data.sessions_of(&updated));
    manager.entry_all(&sessions, kinds::PACKAGE, json!({"package": id, "text": text, "hold_until_ms": updated.hold_until_ms, "joined": updated.requests, "urgent": updated.urgent})).await;
    manager.entry_all(&sessions, kinds::STAGE, json!({"stage": stage::BUILDING, "text": text})).await;
    for request in &updated.requests {
        manager.changed("request", request, &updated.repo);
    }
    manager.changed("package", id, &updated.repo);
    manager.changed("inbox", "", &updated.repo);
    Ok(())
}

// ---- after landing -----------------------------------------------------------------------------

/// Tell the release train that a package landed (again when the train was not there).
pub async fn tell_train(manager: &Arc<Manager>, id: &str) {
    let Ok(package) = manager.package(id) else { return };
    let Ok(repo) = manager.repo(&package.repo) else { return };
    let requests: Vec<(String, String, String)> = manager.store.read(|data| {
        package.requests.iter().filter_map(|id| data.requests.get(id)).map(|rec| (rec.r.id.clone(), rec.r.title.clone(), rec.r.tier.as_str().to_owned())).collect()
    });
    let params = train::LandedParams {
        package: package.id.clone(),
        repo: package.repo.clone(),
        requests,
        parts: package.parts.clone(),
        tier: package.tier.as_str().into(),
        urgent: package.urgent,
        commit: package.landed.clone(),
        soak_min: policy::soak_min(&repo.repo.policy, package.tier, package.urgent),
    };
    let key = format!("train:{id}");
    match manager.peer::<Value>(train::methods::LANDED, json!(params)).await {
        Ok(_) => manager.settled(&key),
        Err(error) => {
            let sessions = manager.store.read(|data| data.sessions_of(&package));
            manager.unavailable(&key, train::methods::LANDED, &sessions, &error).await;
        }
    }
}

/// A train ran: its packages' requests are released, and with that they end.
pub async fn released(manager: &Arc<Manager>, release: &train::Release) {
    if release.state != "ran" {
        return;
    }
    let moved: Vec<String> = release.parts.iter().map(|part| format!("{} {}", part.part, part.to)).collect();
    for id in &release.packages {
        let Ok(package) = manager.package(id) else { continue };
        if package.stage == stage::RELEASED {
            continue;
        }
        manager.store.update_package(id, |package| package.stage = stage::RELEASED.into());
        let text = manager.t("stage_released", &[("package", id), ("parts", &moved.join(", "))]);
        for request in &package.requests {
            manager.store.set_stage(request, stage::RELEASED);
            let Some(rec) = manager.store.update_request(request, |rec| {
                rec.r.ended_as = cc_protocol::contracts::manage::ended::RELEASED.into();
                rec.r.ended_reason = moved.join(", ");
            }) else {
                continue;
            };
            manager.entry(&rec.r.session_id, kinds::STAGE, json!({"stage": stage::RELEASED, "text": text})).await;
            manager.changed("request", request, &package.repo);
        }
        manager.changed("package", id, &package.repo);
    }
    manager.changed("inbox", "", &release.repo);
}

// ---- the package's task ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The builder works; the watch rules apply.
    Building,
    /// The gates run, or the package lands.
    Proving,
    /// The gates wait for a plugin that is not running.
    Pending,
    /// A person is asked.
    Waiting,
}

struct Actor {
    m: Arc<Manager>,
    id: String,
    rx: mpsc::UnboundedReceiver<Msg>,
    /// Messages set aside while old beats were dropped.
    backlog: VecDeque<Msg>,
    phase: Phase,
    done: bool,
    points: Vec<Joined>,
    /// Tasks of an accepted replan, by their number.
    replanned: Vec<Task>,
    facts: Option<Facts>,
    last_beat_ms: u64,
    lost: bool,
    /// The rung reached per rule and task: 1 steered, 2 restarted.
    rungs: HashMap<(String, u32), u8>,
    /// When a rule last acted on a task, and whether its fact went away since.
    acted: HashMap<(String, u32), (u64, bool)>,
    refusals: HashMap<u32, u32>,
    context_restarts: HashMap<u32, u32>,
    seen_checkpoints: Option<usize>,
    approvals: BTreeSet<String>,
    replan_seen: String,
    stop_seen: String,
    fired: BTreeSet<String>,
    steered: u32,
    restarted: u32,
    /// After a restart: when, until the job is seen running again.
    settling_ms: u64,
    /// Tasks still to start again in this rework round, with what was found.
    rework: VecDeque<(u32, String)>,
    conflict_restarted: bool,
    /// The person said yes to landing with walks that could not be performed.
    blocked_ok: bool,
    /// The person said yes to landing a major package.
    landing_ok: bool,
}

impl Actor {
    fn new(m: Arc<Manager>, id: &str, rx: mpsc::UnboundedReceiver<Msg>) -> Self {
        Self {
            m,
            id: id.to_owned(),
            rx,
            backlog: VecDeque::new(),
            phase: Phase::Building,
            done: false,
            points: Vec::new(),
            replanned: Vec::new(),
            facts: None,
            last_beat_ms: now_ms(),
            lost: false,
            rungs: HashMap::new(),
            acted: HashMap::new(),
            refusals: HashMap::new(),
            context_restarts: HashMap::new(),
            seen_checkpoints: None,
            approvals: BTreeSet::new(),
            replan_seen: String::new(),
            stop_seen: String::new(),
            fired: BTreeSet::new(),
            steered: 0,
            restarted: 0,
            settling_ms: 0,
            rework: VecDeque::new(),
            conflict_restarted: false,
            blocked_ok: false,
            landing_ok: false,
        }
    }

    fn package(&self) -> Option<Package> {
        self.m.store.read(|data| data.packages.get(&self.id).cloned())
    }

    fn repo(&self, package: &Package) -> Option<Repo> {
        self.m.store.read(|data| data.repos.get(&package.repo).map(|rec| rec.repo.clone()))
    }

    async fn run(mut self) {
        self.init().await;
        while !self.done {
            let msg = match self.backlog.pop_front() {
                Some(msg) => msg,
                None => tokio::select! {
                    _ = self.m.stop.cancelled() => break,
                    msg = self.rx.recv() => match msg {
                        Some(msg) => msg,
                        None => break,
                    },
                },
            };
            match msg {
                Msg::Cancel => break,
                Msg::Beat(facts) => self.on_beat(*facts).await,
                Msg::Up => {
                    if self.lost || self.facts.is_none() {
                        self.refresh().await;
                    }
                    if self.phase == Phase::Pending {
                        self.prove().await;
                    }
                }
                Msg::Tick => self.on_tick().await,
                Msg::Retry => {
                    if self.phase == Phase::Pending {
                        self.prove().await;
                    }
                }
                Msg::Resume => self.resume().await,
                Msg::Decided { ask } => self.decided(&ask).await,
            }
        }
        // Close the channel before the entry goes, so a message sent meanwhile starts a new task.
        self.rx.close();
        let mut jobs = lock(&self.m.jobs);
        if jobs.get(&self.id).is_some_and(|sender| sender.is_closed()) {
            jobs.remove(&self.id);
        }
    }

    /// Take the job up: what was already done to it in this attempt, whether a person is being
    /// asked, and the runner's facts as they are now.
    async fn init(&mut self) {
        let Some(package) = self.package() else {
            self.done = true;
            return;
        };
        if !matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING) || package.session_id.is_empty() {
            self.done = true;
            return;
        }
        let (points, interventions, asked) = self.m.store.read(|data| {
            let interventions: Vec<(Intervention, u32)> = data.interventions.get(&self.id).map(|list| list.iter().map(|rec| (rec.i.clone(), rec.attempt)).collect()).unwrap_or_default();
            (packages::joined(data, &package), interventions, !data.asks_of(&package).is_empty())
        });
        self.points = points;
        for (intervention, attempt) in interventions {
            if attempt != package.attempt {
                continue;
            }
            let key = (intervention.rule.clone(), intervention.task);
            match intervention.action.as_str() {
                "steer" => {
                    self.steered += 1;
                    let rung = self.rungs.entry(key).or_insert(0);
                    *rung = (*rung).max(1);
                }
                "restart" => {
                    self.restarted += 1;
                    self.rungs.insert(key, 2);
                }
                _ => {}
            }
            if RULES.contains(&intervention.rule.as_str()) {
                self.fired.insert(intervention.rule);
            }
        }
        self.phase = if asked { Phase::Waiting } else { Phase::Building };
        self.refresh().await;
    }

    /// Ask the runner for the job's facts and treat them as a beat.
    async fn refresh(&mut self) {
        let Some(package) = self.package() else { return };
        let Some(repo) = self.repo(&package) else { return };
        match self.m.runner::<Facts>(&Manager::node_of(&package, &repo), runner::methods::GET, json!({"job_id": package.job_id}), RUNNER_CALL).await {
            Ok(facts) => self.on_beat(facts).await,
            Err(error) => tracing::info!(package = %self.id, error = %error.message, "the runner did not give the job's facts; waiting for it"),
        }
    }

    /// Old beats that queued up while the manager was busy say nothing about now.
    fn drop_old_beats(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            if !matches!(msg, Msg::Beat(_)) {
                self.backlog.push_back(msg);
            }
        }
    }

    // ---- who and what ------------------------------------------------------------------------

    fn point(&self, task: u32) -> Option<Point> {
        if let Some(replanned) = self.replanned.iter().find(|replanned| replanned.n == task) {
            return Some(point_of(replanned));
        }
        self.points.iter().find(|joined| joined.point.n == task).map(|joined| joined.point.clone())
    }

    fn title(&self, task: u32) -> String {
        self.point(task).map(|point| point.title).unwrap_or_default()
    }

    /// The thread of the request a task belongs to (the package's first when unknown).
    fn session(&self, task: u32) -> String {
        let request = self.point(task).map(|point| point.request).unwrap_or_default();
        self.m.store.read(|data| {
            data.requests
                .get(&request)
                .map(|rec| rec.r.session_id.clone())
                .or_else(|| data.packages.get(&self.id).and_then(|package| data.sessions_of(package).into_iter().next()))
                .unwrap_or_default()
        })
    }

    fn task_now(&self) -> u32 {
        self.facts.as_ref().map(|facts| facts.task).unwrap_or(0).max(1)
    }

    // ---- what is recorded and shown ----------------------------------------------------------

    /// Record that the manager stepped in, in the graph and in the thread.
    async fn intervene(&mut self, rule: &str, fact: &str, action: &str, text: &str, task: u32) {
        let attempt = self.package().map(|package| package.attempt).unwrap_or(0);
        let intervention = Intervention { rule: rule.into(), fact: fact.into(), action: action.into(), text: text.into(), task, ms: now_ms() };
        self.m.store.add_intervention(&self.id, intervention.clone(), attempt);
        if RULES.contains(&rule) {
            self.fired.insert(rule.to_owned());
        }
        match action {
            "steer" => self.steered += 1,
            "restart" => self.restarted += 1,
            _ => {}
        }
        self.m.entry(&self.session(task), kinds::WATCH, json!(intervention)).await;
        self.show();
        if let Some(package) = self.package() {
            self.m.changed("package", &self.id, &package.repo);
        }
    }

    fn strip(&self, package: &Package) -> Option<Strip> {
        let facts = self.facts.as_ref()?;
        let policy = self.m.store.read(|data| data.repos.get(&package.repo).map(|rec| rec.repo.policy.clone())).unwrap_or_default();
        // The one guard every job has: its longest time (size has no limit).
        let budget = policy::budget(&policy, Tier::Major);
        Some(Strip {
            task: facts.task,
            tasks: facts.tasks,
            attempt: package.attempt,
            checkpoint_ms: facts.checkpoints.iter().map(|checkpoint| checkpoint.ms).max().unwrap_or(0),
            files: facts.total_files,
            lines: facts.total_lines,
            limit_lines: 0,
            in_scope: facts.out_of_scope.is_empty(),
            minutes: facts.minutes,
            limit_minutes: budget[0],
            steered: self.steered,
            restarted: self.restarted,
            now: facts.now.clone(),
            rules: RULES.iter().map(|rule| ((*rule).to_owned(), self.fired.contains(*rule))).collect(),
        })
    }

    /// Publish the strip when it changed.
    fn show(&self) {
        let Some(package) = self.package() else { return };
        let Some(strip) = self.strip(&package) else { return };
        let (changed, digest) = self.m.store.write(|data| {
            let changed = data.strips.get(&self.id) != Some(&strip);
            if changed {
                data.strips.insert(self.id.clone(), strip.clone());
            }
            (changed, data.digests.get(&self.id).cloned().unwrap_or_default())
        });
        if changed && let Ok(ctx) = self.m.ctx() {
            let state = self.facts.as_ref().map(|facts| facts.state.clone()).unwrap_or_default();
            let beat = WatchBeat { package: self.id.clone(), strip, digest, extra: json!({"state": if self.lost { "lost" } else { state.as_str() }}) };
            ctx.publish(topics::WATCH, json!(beat));
        }
    }

    /// Ask for the watch's two lines again (after a checkpoint, a steer, a restart). Nothing is
    /// decided from them, so the job does not wait for the answer.
    fn digest(&self, event: &str, task: u32) {
        let Some(package) = self.package() else { return };
        let facts = self.facts.clone().unwrap_or_default();
        let facts = json!({
            "package": self.id, "event": event, "task": task, "tasks": facts.tasks, "title": self.title(task), "now": facts.now,
            "files": facts.total_files, "lines": facts.total_lines, "minutes": facts.minutes, "tokens": facts.tokens,
            "checkpoints": facts.checkpoints.len(), "out_of_scope": facts.out_of_scope, "steered": self.steered, "restarted": self.restarted,
            "attempt": package.attempt,
        });
        let (manager, id, session) = (self.m.clone(), self.id.clone(), self.session(task));
        let strip = self.strip(&package);
        tokio::spawn(async move {
            let text = match manager.peer::<Value>(flow::DIGEST, json!({"facts": facts})).await {
                Ok(answer) => answer.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_owned(),
                Err(error) => {
                    tracing::debug!(package = %id, code = %error.code, "no digest: the chat flow did not answer");
                    return;
                }
            };
            if text.is_empty() {
                return;
            }
            manager.store.set_digest(&id, &text);
            manager.entry(&session, kinds::DIGEST, json!({"text": text})).await;
            if let (Some(strip), Ok(ctx)) = (strip, manager.ctx()) {
                ctx.publish(topics::WATCH, json!(WatchBeat { package: id.clone(), strip, digest: text, extra: Value::Null }));
            }
        });
    }

    /// Stop the work and ask a person.
    async fn ask(&mut self, kind: &str, subject_key: &str, detail: &str, actions: &[&str], resume: &str) {
        self.phase = Phase::Waiting;
        let Some(package) = self.package() else { return };
        let title = self.m.store.read(|data| package.requests.first().and_then(|id| data.requests.get(id)).map(|rec| rec.r.title.clone()).unwrap_or_default());
        inbox::raise(
            &self.m,
            NewAsk {
                kind: kind.into(),
                repo: package.repo.clone(),
                package: self.id.clone(),
                request: package.requests.first().cloned().unwrap_or_default(),
                subject: self.m.t(subject_key, &[("title", &title)]),
                title,
                detail: detail.to_owned(),
                actions: inbox::actions(actions),
                attempt: package.attempt,
                resume: resume.into(),
                ..NewAsk::default()
            },
        )
        .await;
    }

    // ---- calls to the runner -----------------------------------------------------------------

    async fn call<T: serde::de::DeserializeOwned>(&self, method: &str, params: Value, limit: Duration) -> Result<T, RpcError> {
        let package = self.package().ok_or_else(|| RpcError::not_found("the package is gone"))?;
        let repo = self.repo(&package).ok_or_else(|| RpcError::not_found("the repository is gone"))?;
        self.m.runner(&Manager::node_of(&package, &repo), method, params, limit).await
    }

    async fn stop_job(&self) {
        let Some(package) = self.package() else { return };
        if let Err(error) = self.call::<Value>(runner::methods::STOP, json!({"job_id": package.job_id, "attempt": package.attempt}), RUNNER_CALL).await {
            tracing::warn!(package = %self.id, error = %error.message, "the runner did not stop the job");
        }
    }

    /// Start a task again from the last checkpoint, in a new session, with one line on what was
    /// tried and what stopped it. False when the runner refused (a person is asked then).
    async fn restart(&mut self, rule: &str, task: u32, fact: &str, note: &str) -> bool {
        let Some(package) = self.package() else { return false };
        let params = runner::RestartParams { job_id: package.job_id.clone(), attempt: package.attempt, task, note: note.to_owned() };
        match self.call::<runner::Started>(runner::methods::RESTART, json!(params), RUNNER_CALL).await {
            Ok(started) => {
                self.m.store.update_package(&self.id, |package| package.session_id = started.session_id.clone());
                self.settling_ms = now_ms();
                self.stop_seen.clear();
                self.phase = Phase::Building;
                self.intervene(rule, fact, "restart", note, task).await;
                self.digest("restart", task);
                self.drop_old_beats();
                true
            }
            Err(error) => {
                tracing::warn!(package = %self.id, task, error = %error.message, "the runner did not start the task again");
                let detail = self.m.t("detail_restart_failed", &[("task", &self.title(task)), ("fact", fact), ("why", &error.message)]);
                self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], &format!("task:{task}")).await;
                false
            }
        }
    }

    /// The last rung: the job stops and a person looks.
    async fn stop_and_ask(&mut self, rule: &str, task: u32, fact: &str) {
        self.stop_job().await;
        self.intervene(rule, fact, "stop", "", task).await;
        let detail = self.m.t("detail_stuck", &[("task", &self.title(task)), ("fact", fact)]);
        self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], &format!("task:{task}")).await;
    }

    /// One step up the ladder for a rule on a task: steer (when there is a sentence and nothing
    /// was done yet), restart, then a person.
    async fn climb(&mut self, rule: &str, task: u32, fact: &str, steer: Option<String>) {
        let key = (rule.to_owned(), task);
        let rung = self.rungs.get(&key).copied().unwrap_or(0);
        self.acted.insert(key.clone(), (now_ms(), false));
        match (rung, steer) {
            (0, Some(text)) => {
                self.rungs.insert(key, 1);
                let Some(package) = self.package() else { return };
                let params = runner::SteerParams { job_id: package.job_id.clone(), attempt: package.attempt, text: text.clone() };
                match self.call::<Value>(runner::methods::STEER, json!(params), RUNNER_CALL).await {
                    Ok(_) => {
                        self.intervene(rule, fact, "steer", &text, task).await;
                        self.digest("steer", task);
                    }
                    // Refused (the builder waits on an approval, or went on): written down, not sent again.
                    Err(error) => self.intervene(rule, fact, "note", &self.m.t("steer_not_delivered", &[("why", &error.message)]), task).await,
                }
            }
            (0, None) | (1, _) => {
                self.rungs.insert(key, 2);
                let note = self.m.t("restart_note", &[("fact", fact)]);
                self.restart(rule, task, fact, &note).await;
            }
            _ => self.stop_and_ask(rule, task, fact).await,
        }
    }

    // ---- beats -------------------------------------------------------------------------------

    async fn on_beat(&mut self, facts: Facts) {
        let Some(package) = self.package() else {
            self.done = true;
            return;
        };
        if facts.job_id != package.job_id || facts.attempt < package.attempt {
            return;
        }
        if facts.attempt > package.attempt {
            // The runner counts the attempts; the manager follows.
            self.m.store.update_package(&self.id, |package| package.attempt = facts.attempt);
        }
        self.last_beat_ms = now_ms();
        if self.lost {
            self.lost = false;
            self.intervene("W8", &self.m.t("fact_w8_back", &[]), "note", "", facts.task).await;
        }
        let working = matches!(facts.state.as_str(), "running" | "waiting");
        if self.settling_ms != 0 {
            if working || now_ms().saturating_sub(self.settling_ms) > SETTLE_MS {
                self.settling_ms = 0;
            } else {
                // A beat from before the restart.
                return;
            }
        }
        self.facts = Some(facts.clone());
        self.show();
        if self.phase != Phase::Building {
            return;
        }
        // Checkpoints: each new one is looked at once. A restart may have taken some back.
        let count = facts.checkpoints.len();
        match self.seen_checkpoints {
            None => self.seen_checkpoints = Some(count),
            Some(seen) if count > seen => {
                self.seen_checkpoints = Some(count);
                for checkpoint in &facts.checkpoints[seen..] {
                    self.checkpoint(checkpoint).await;
                    if self.phase != Phase::Building || self.settling_ms != 0 {
                        return;
                    }
                }
            }
            Some(_) => self.seen_checkpoints = Some(count),
        }
        if let Some(approval) = &facts.approval {
            // While the builder waits on an approval nothing else is said to it.
            self.approval(&facts, approval).await;
            return;
        }
        if let Some(replan) = &facts.replan {
            self.replan(&facts, replan).await;
            if self.phase != Phase::Building || self.done {
                return;
            }
        }
        match facts.state.as_str() {
            "running" => self.rules(&facts).await,
            "done" => self.finished().await,
            "failed" | "stopped" | "refused" => self.stopped(&facts).await,
            _ => {}
        }
    }

    /// W8: three beats missed. The job is marked lost and waited for; on the runner's return its
    /// facts are asked and the work goes on from the last checkpoint.
    async fn on_tick(&mut self) {
        let working = self.facts.as_ref().is_some_and(|facts| matches!(facts.state.as_str(), "running" | "waiting"));
        if self.phase == Phase::Building && working && !self.lost && now_ms().saturating_sub(self.last_beat_ms) > LOST_AFTER_MS {
            self.lost = true;
            self.intervene("W8", &self.m.t("fact_w8", &[]), "note", "", self.task_now()).await;
        }
    }

    /// The package may go on: a job that was stopped for the hold starts its task again.
    async fn resume(&mut self) {
        if self.phase != Phase::Building {
            return;
        }
        self.refresh().await;
        let stopped = self.facts.as_ref().is_some_and(|facts| facts.state == "stopped");
        let held = self.m.store.read(|data| data.hold || data.packages.get(&self.id).is_none_or(|package| package.stopped));
        if stopped && !held {
            let task = self.task_now();
            let fact = self.m.t("fact_resumed", &[]);
            self.restart("resume", task, &fact, "").await;
        }
    }

    // ---- the watch rules ---------------------------------------------------------------------

    async fn rules(&mut self, facts: &Facts) {
        let Some(package) = self.package() else { return };
        let Some(repo) = self.repo(&package) else { return };
        let (standard_change, charter_change) = special(&self.m, &package);
        let task = facts.task.max(1);
        let next = self.title(task);
        // W7: a runaway guard, the same for every job: the longest time and the most tokens the
        // policy allows any job. How many lines a change has says nothing about how safe it is,
        // so size stops nothing and changes no rating.
        let limit = policy::budget(&repo.policy, Tier::Major);
        if facts.minutes > limit[0] || facts.tokens > limit[1] {
            let fact = self.m.t(
                "fact_w7",
                &[("minutes", &facts.minutes.to_string()), ("limit_minutes", &limit[0].to_string()), ("tokens", &facts.tokens.to_string()), ("limit_tokens", &limit[1].to_string())],
            );
            self.stop_job().await;
            self.intervene("W7", &fact, "stop", "", task).await;
            let detail = self.m.t("detail_budget", &[("task", &self.title(task)), ("fact", &fact)]);
            self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], &format!("task:{task}")).await;
            return;
        }
        // W5: a protected file changed in a job that may not change it. No steer: the checkpoint
        // is restored by starting the task again, and a second time a person looks.
        if !standard_change && !charter_change && !facts.protected_changed.is_empty() {
            let fact = self.m.t("fact_w5", &[("file", &facts.protected_changed.join(", "))]);
            if self.may_fire("W5", task) {
                self.climb("W5", task, &fact, None).await;
            }
            return;
        }
        let file = facts.out_of_scope.first().cloned().unwrap_or_default();
        let minutes = (facts.idle_ms / 60_000).to_string();
        let checks: [(&str, bool, String, String); 4] = [
            ("W4", !facts.out_of_scope.is_empty(), self.m.t("fact_w4", &[("file", &facts.out_of_scope.join(", "))]), self.m.t("steer_w4", &[("file", &file), ("next", &next)])),
            (
                "W1",
                facts.same_failure >= SAME_FAILURE,
                self.m.t("fact_w1", &[("command", &facts.failing_command), ("count", &facts.same_failure.to_string())]),
                self.m.t("steer_w1", &[("command", &facts.failing_command), ("count", &facts.same_failure.to_string()), ("next", &next)]),
            ),
            (
                "W2",
                facts.same_call >= SAME_CALL,
                self.m.t("fact_w2", &[("count", &facts.same_call.to_string())]),
                self.m.t("steer_w2", &[("count", &facts.same_call.to_string()), ("next", &next)]),
            ),
            (
                "W3",
                facts.calls_since_edit >= CALLS_WITHOUT_EDIT || facts.idle_ms >= IDLE_MS,
                self.m.t("fact_w3", &[("calls", &facts.calls_since_edit.to_string()), ("minutes", &minutes)]),
                self.m.t("steer_w3", &[("calls", &facts.calls_since_edit.to_string()), ("minutes", &minutes), ("next", &next)]),
            ),
        ];
        for (rule, on, fact, steer) in checks {
            if !on {
                if let Some(acted) = self.acted.get_mut(&(rule.to_owned(), task)) {
                    acted.1 = true;
                }
                continue;
            }
            if self.may_fire(rule, task) {
                // One step per beat: the next beat shows what it did.
                self.climb(rule, task, &fact, Some(steer)).await;
                return;
            }
        }
    }

    /// Whether a rule may act on a task now: not right after it acted, unless its fact went away
    /// in between.
    fn may_fire(&self, rule: &str, task: u32) -> bool {
        match self.acted.get(&(rule.to_owned(), task)) {
            Some((at, cleared)) => *cleared || now_ms().saturating_sub(*at) >= COOLDOWN_MS,
            None => true,
        }
    }

    /// A task got its checkpoint: the charter's checks over its diff (W6), then the one question
    /// "does this diff do what the task says?".
    async fn checkpoint(&mut self, checkpoint: &Checkpoint) {
        let task = checkpoint.task;
        let Some(package) = self.package() else { return };
        let Some(repo) = self.repo(&package) else { return };
        let Some(point) = self.point(task) else { return };
        let session = self.session(task);
        let shown = json!({"task": task, "files": checkpoint.files, "lines": checkpoint.lines, "commit": checkpoint.commit});
        // The runner is asked for this checkpoint's own diff. When it answers nothing (the diff
        // "since the last checkpoint" is empty right after one), the task's part of the whole
        // diff is taken instead.
        let mut answer: runner::DiffAnswer =
            match self.call(runner::methods::DIFF, json!({"job_id": package.job_id, "task": task, "commit": checkpoint.commit}), RUNNER_CALL).await {
                Ok(answer) => answer,
                Err(error) => {
                    self.intervene("drift", &self.m.t("drift_no_diff", &[("why", &error.message)]), "note", "", task).await;
                    return;
                }
            };
        if answer.diff.trim().is_empty()
            && let Ok(whole) = self.call::<runner::DiffAnswer>(runner::methods::DIFF, json!({"job_id": package.job_id, "whole": true}), RUNNER_CALL).await
        {
            let own = diff::only(&whole.diff, &point.touches);
            answer = runner::DiffAnswer { files: diff::files(&own), diff: own, truncated: whole.truncated };
        }
        if answer.diff.trim().is_empty() {
            self.m.entry(&session, kinds::WATCH, json!({"checkpoint": shown})).await;
            self.intervene("drift", &self.m.t("drift_empty", &[]), "note", "", task).await;
            return;
        }
        let (standard_change, _) = special(&self.m, &package);
        // W6: a law's check fails. A charter that is not there is not a failed check.
        let check = charter::CheckParams { repo: repo.id.clone(), diff: answer.diff.clone(), files: answer.files.clone(), standard_change };
        if let Ok(checked) = self.m.peer::<charter::CheckAnswer>(charter::methods::CHECK, json!(check)).await
            && let Some(finding) = checked.findings.first()
        {
            let fact = self.m.t("fact_w6", &[("rule", &finding.rule), ("file", &finding.file), ("text", &finding.text)]);
            self.m.entry(&session, kinds::WATCH, json!({"checkpoint": shown})).await;
            self.climb("W6", task, &fact, None).await;
            return;
        }
        let params = DriftParams { title: point.title.clone(), rules: point.rules.clone(), diff: answer.diff.clone() };
        let drift: DriftAnswer = match self.m.peer(flow::DRIFT, json!(params)).await {
            Ok(drift) => drift,
            Err(error) => {
                // Not asked is not "yes" and not "no": it is written down as not asked.
                self.m.entry(&session, kinds::WATCH, json!({"checkpoint": shown})).await;
                self.intervene("drift", &self.m.t("drift_not_asked", &[("why", &error.message)]), "note", "", task).await;
                return;
            }
        };
        // A yes must show its line; one whose file and line are not in the diff counts as no.
        let shows = diff::has_line(&answer.diff, &drift.file, drift.line);
        let yes = drift.does_it && shows;
        let reason = if drift.does_it && !shows {
            self.m.t("drift_no_line", &[("file", &drift.file), ("line", &drift.line.to_string())])
        } else if drift.reason.trim().is_empty() {
            self.m.t("drift_no_reason", &[])
        } else {
            drift.reason.trim().to_owned()
        };
        self.m
            .entry(&session, kinds::WATCH, json!({"checkpoint": shown, "drift": {"task": task, "does_it": yes, "file": drift.file, "line": drift.line, "reason": reason}}))
            .await;
        if yes {
            self.digest("checkpoint", task);
            return;
        }
        // A no is the task's restart rung: started again with the answer as the note, and a
        // second no waits on a person.
        let key = ("drift".to_owned(), task);
        let fact = self.m.t("fact_drift", &[("task", &point.title), ("reason", &reason)]);
        if self.rungs.get(&key).copied().unwrap_or(0) >= 2 {
            self.stop_and_ask("drift", task, &fact).await;
        } else {
            self.rungs.insert(key, 2);
            let note = self.m.t("restart_note", &[("fact", &reason)]);
            self.restart("drift", task, &fact, &note).await;
        }
    }

    /// The builder asks for an approval. What the plan lists is approved; everything else is
    /// refused with a sentence saying what to do instead. A second refusal on one task waits on
    /// a person. Never answered by a model.
    async fn approval(&mut self, facts: &Facts, approval: &HeldApproval) {
        let key = format!("{}|{}", facts.session_id, approval.seq);
        if self.approvals.contains(&key) {
            return;
        }
        let Some(package) = self.package() else { return };
        let task = facts.task.max(1);
        let approve = !approval.classes.is_empty() && approval.classes.iter().all(|class| APPROVE.contains(&class.as_str()));
        let text = if approve { String::new() } else { self.m.t("approval_refused", &[("tool", &approval.tool), ("classes", &approval.classes.join(", ")), ("next", &self.title(task))]) };
        let answer = runner::ApprovalAnswer { job_id: package.job_id.clone(), attempt: package.attempt, seq: approval.seq, approve, text: text.clone() };
        if let Err(error) = self.call::<Value>(runner::methods::APPROVAL_ANSWER, json!(answer), RUNNER_CALL).await {
            tracing::warn!(package = %self.id, error = %error.message, "the approval was not answered; it is answered at the next beat");
            return;
        }
        self.approvals.insert(key);
        let fact = self.m.t("fact_approval", &[("tool", &approval.tool), ("classes", &approval.classes.join(", ")), ("question", &approval.question)]);
        if approve {
            self.intervene("approval", &fact, "note", "", task).await;
            return;
        }
        self.intervene("approval", &fact, "refuse", &text, task).await;
        let refused = self.refusals.entry(task).or_insert(0);
        *refused += 1;
        if *refused >= 2 {
            self.stop_and_ask("approval", task, &fact).await;
        }
    }

    /// CortexCode wants to replan. Each proposed task goes to the validator as a point of the
    /// request; the replan is accepted when all are kept and the tier's floor does not rise.
    async fn replan(&mut self, facts: &Facts, replan: &HeldReplan) {
        let key = format!("{}|{}|{}", facts.session_id, replan.found, replan.tasks.len());
        if self.replan_seen == key {
            return;
        }
        let Some(package) = self.package() else { return };
        let Some(repo) = self.repo(&package) else { return };
        let charter = pipeline::charter_of(&self.m, &repo.id).await;
        let (standard_change, _) = special(&self.m, &package);
        let mut all_kept = true;
        let mut proposed: Vec<Point> = Vec::new();
        for task in &replan.tasks {
            let mut point = point_of(task);
            if !package.requests.contains(&point.request) {
                point.request = package.requests.first().cloned().unwrap_or_default();
            }
            let (text, session) = self.m.store.read(|data| data.requests.get(&point.request).map(|rec| (rec.r.text.clone(), rec.r.session_id.clone())).unwrap_or_default());
            // Not available is not an answer: the replan stays held and is asked again at the next beat.
            let Ok(touched) = pipeline::standard_touches(&self.m, &repo, &point.touches).await else { return };
            let params = ValidateParams { session_id: session, text, point: point.clone(), identity: charter.identity.clone(), laws: charter.laws.clone(), touches_standard: !standard_change && !touched.is_empty() };
            let Ok(answer) = self.m.peer::<ValidateAnswer>(flow::VALIDATE, json!(params)).await else { return };
            if answer.verdict.trim() != "keep" || answer.reason.trim().is_empty() {
                all_kept = false;
            }
            proposed.push(point);
        }
        // The plan as it would be: what is done already, and the proposed rest.
        let mut whole: Vec<Point> = self.points.iter().map(|joined| joined.point.clone()).filter(|point| point.n < facts.task.max(1)).collect();
        whole.extend(proposed.iter().cloned());
        let mut touches: Vec<String> = whole.iter().flat_map(|point| point.touches.iter().cloned()).collect();
        touches.sort();
        touches.dedup();
        let Ok(standard_paths) = pipeline::standard_touches(&self.m, &repo, &touches).await else { return };
        let floor = policy::floor(&repo.policy, &whole, &standard_paths, &self.m.texts);
        let rises = floor.tier > package.tier;
        let accept = all_kept && !rises;
        let answer = runner::ReplanAnswer { job_id: package.job_id.clone(), attempt: package.attempt, accept };
        if let Err(error) = self.call::<Value>(runner::methods::REPLAN_ANSWER, json!(answer), RUNNER_CALL).await {
            tracing::warn!(package = %self.id, error = %error.message, "the replan was not answered; it is answered at the next beat");
            return;
        }
        self.replan_seen = key;
        let task = facts.task.max(1);
        if accept {
            self.replanned = replan.tasks.clone();
            self.intervene("replan", &replan.found, "note", &self.m.t("replan_accepted", &[("count", &replan.tasks.len().to_string())]), task).await;
            return;
        }
        let why = if rises { self.m.t("replan_refused_tier", &[("tier", floor.tier.as_str())]) } else { self.m.t("replan_refused", &[]) };
        self.intervene("replan", &replan.found, "refuse", &why, task).await;
        if rises {
            self.raise_tier(floor).await;
        }
    }

    /// The work showed that the change is bigger than its tier allows: the package is assessed
    /// upward, the job stops, and the package is held again by the new tier's rule.
    async fn raise_tier(&mut self, floor: policy::Floor) {
        self.stop_job().await;
        let Some(package) = self.package() else { return };
        let Some(repo) = self.repo(&package) else { return };
        let now = now_ms();
        self.m.store.update_package(&self.id, |package| {
            package.tier = floor.tier;
            package.urgent = package.urgent && floor.tier != Tier::Major;
            package.hold_until_ms = policy::hold_until(&repo.policy, floor.tier, package.urgent, now, now);
            package.held_since_ms = now;
        });
        for request in &package.requests {
            self.m.store.update_request(request, |rec| {
                rec.r.tier_floor = rec.r.tier_floor.max(floor.tier);
                for reason in &floor.reasons {
                    if !rec.r.tier_reasons.contains(reason) {
                        rec.r.tier_reasons.push(reason.clone());
                    }
                }
                pipeline::settle_urgent(rec);
            });
        }
        set_stage(&self.m, &self.id, stage::HELD).await;
        let line = self.m.t("pkg_reassessed", &[("package", &self.id), ("tier", floor.tier.as_str())]);
        let sessions = self.m.store.read(|data| data.sessions_of(&package));
        for session in &sessions {
            self.m.line(session, &line).await;
        }
        if floor.tier == Tier::Major {
            let title = self.m.store.read(|data| package.requests.first().and_then(|id| data.requests.get(id)).map(|rec| rec.r.title.clone()).unwrap_or_default());
            inbox::raise(
                &self.m,
                NewAsk {
                    kind: "approve_plan".into(),
                    repo: package.repo.clone(),
                    package: self.id.clone(),
                    request: package.requests.first().cloned().unwrap_or_default(),
                    subject: self.m.t("subject_approve_plan", &[("title", &title)]),
                    title,
                    detail: floor.reasons.join(" "),
                    actions: inbox::actions(&["approve", "send_back", "decline"]),
                    attempt: package.attempt,
                    ..NewAsk::default()
                },
            )
            .await;
        }
        self.m.look_now();
        self.done = true;
    }

    /// The run stopped. Why it stopped decides what happens next, one action per reason.
    async fn stopped(&mut self, facts: &Facts) {
        let key = format!("{}|{}|{}", facts.session_id, facts.state, facts.stopped_because);
        if self.stop_seen == key {
            return;
        }
        self.stop_seen = key;
        let task = facts.task.max(1);
        match facts.stopped_because.as_str() {
            // Waiting for the model's lane loses nothing; cancelled was the manager's own stop;
            // approvals and replans are answered where they are seen.
            "lane" | "cancelled" | "approval" | "replan" | "done" => {}
            "context" => {
                let count = self.context_restarts.entry(task).or_insert(0);
                *count += 1;
                let fact = self.m.t("fact_context", &[]);
                if *count <= CONTEXT_RESTARTS {
                    let note = self.m.t("note_context", &[]);
                    self.restart("context", task, &fact, &note).await;
                } else {
                    self.stop_and_ask("context", task, &fact).await;
                }
            }
            _ => {
                let fact = self.m.t("fact_error", &[("error", if facts.error.is_empty() { &facts.state } else { &facts.error })]);
                self.climb("error", task, &fact, None).await;
            }
        }
    }

    // ---- done: gates and landing -------------------------------------------------------------

    /// The builder is done with its tasks.
    async fn finished(&mut self) {
        if let Some((task, note)) = self.rework.pop_front() {
            // More tasks of this rework round.
            let fact = self.m.t("fact_rework", &[("finding", &note)]);
            self.restart("rework", task, &fact, &note).await;
            return;
        }
        self.prove().await;
    }

    async fn prove(&mut self) {
        self.phase = Phase::Proving;
        set_stage(&self.m, &self.id, stage::PROVING).await;
        if self.facts.is_none() {
            self.refresh_quietly().await;
        }
        let facts = self.facts.clone().unwrap_or_default();
        let outcome = gates::run(&self.m, &self.id, &facts, Which::All).await;
        self.drop_old_beats();
        self.after_gates(outcome).await;
    }

    /// The runner's facts without acting on them (the gates need the scope's fact).
    async fn refresh_quietly(&mut self) {
        let Some(package) = self.package() else { return };
        if let Ok(facts) = self.call::<Facts>(runner::methods::GET, json!({"job_id": package.job_id}), RUNNER_CALL).await {
            self.facts = Some(facts);
        }
    }

    /// The gates wait for something that is not available. True when they do.
    async fn pending(&mut self, outcome: &Outcome) -> bool {
        let Some((method, error)) = &outcome.unavailable else { return false };
        self.phase = Phase::Pending;
        let sessions = self.package().map(|package| self.m.store.read(|data| data.sessions_of(&package))).unwrap_or_default();
        self.m.unavailable(&format!("package:{}", self.id), method, &sessions, error).await;
        true
    }

    async fn after_gates(&mut self, outcome: Outcome) {
        if self.pending(&outcome).await {
            return;
        }
        self.m.settled(&format!("package:{}", self.id));
        if !outcome.failed.is_empty() {
            self.rework_round(outcome.failed).await;
            return;
        }
        let Some(package) = self.package() else { return };
        // A major package lands by a yes; that yes also covers walks nothing could perform.
        if package.tier == Tier::Major && !self.landing_ok {
            let detail = self.m.t("detail_approve_landing", &[("proof", &outcome.summary.join(" "))]);
            self.ask("approve_landing", "subject_approve_landing", &detail, &["approve", "send_back", "decline"], "land").await;
            return;
        }
        if outcome.blocked && !self.blocked_ok && package.tier != Tier::Major {
            let detail = self.m.t("detail_blocked_walk", &[("proof", &outcome.summary.join(" "))]);
            self.ask("blocked_walk", "subject_blocked_walk", &detail, &["approve", "rerun", "decline"], "land").await;
            return;
        }
        self.land().await;
    }

    /// A gate failed: the failing tasks start again with what was found, as many rounds as the
    /// policy gives the tier. Then a person.
    async fn rework_round(&mut self, failed: Vec<(u32, String)>) {
        let Some(package) = self.package() else { return };
        let Some(repo) = self.repo(&package) else { return };
        let summary: Vec<String> = failed.iter().map(|(_, finding)| finding.clone()).collect();
        if package.reworks >= policy::rework(&repo.policy, package.tier) {
            let detail = self.m.t("detail_gates_failed", &[("rounds", &package.reworks.to_string()), ("findings", &summary.join(" "))]);
            self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], "gates").await;
            return;
        }
        self.m.store.update_package(&self.id, |package| package.reworks += 1);
        // One restart per task, with everything found about it, the earliest task first.
        let mut by_task: std::collections::BTreeMap<u32, Vec<String>> = std::collections::BTreeMap::new();
        for (task, finding) in failed {
            by_task.entry(task).or_default().push(finding);
        }
        self.rework = by_task.into_iter().map(|(task, findings)| (task, findings.join(" "))).collect();
        self.blocked_ok = false;
        self.landing_ok = false;
        set_stage(&self.m, &self.id, stage::BUILDING).await;
        self.phase = Phase::Building;
        if let Some((task, note)) = self.rework.pop_front() {
            let fact = self.m.t("fact_rework", &[("finding", &note)]);
            self.restart("rework", task, &fact, &note).await;
        }
    }

    /// Land the package: the runner fetches, rebases, squashes per request, fast-forwards the
    /// main branch and pushes. A rebase means building and walking again before landing again.
    async fn land(&mut self) {
        self.phase = Phase::Proving;
        let mut tries = 0;
        let mut rebased_ok = false;
        loop {
            let Some(package) = self.package() else { return };
            let Some(repo) = self.repo(&package) else { return };
            let requests: Vec<(String, String)> =
                self.m.store.read(|data| package.requests.iter().filter_map(|id| data.requests.get(id)).map(|rec| (rec.r.id.clone(), rec.r.title.clone())).collect());
            // `rebased_ok` is what the runner contract's text asks for on the second call; its
            // `LandParams` has no such field, so it is sent beside the ones it has.
            let mut params = json!(runner::LandParams { job_id: package.job_id.clone(), attempt: package.attempt, requests, changes: repo.source.changes.clone() });
            if rebased_ok {
                params["rebased_ok"] = json!(true);
            }
            let landed: runner::Landed = match self.call(runner::methods::LAND, params, RUNNER_LAND).await {
                Ok(landed) => landed,
                Err(error) => {
                    let detail = self.m.t("detail_land_failed", &[("why", &error.message)]);
                    self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], "land").await;
                    return;
                }
            };
            self.drop_old_beats();
            if !landed.conflicts.is_empty() {
                let files = landed.conflicts.join(", ");
                if self.conflict_restarted {
                    let detail = self.m.t("detail_conflict", &[("files", &files)]);
                    self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], "land").await;
                    return;
                }
                // One task for the builder: resolve. A second failure waits on a person.
                self.conflict_restarted = true;
                let task = gates::task_for_file(&self.points, &landed.conflicts[0]);
                let note = self.m.t("note_resolve", &[("files", &files)]);
                let fact = self.m.t("fact_conflict", &[("files", &files)]);
                self.landing_ok = false;
                set_stage(&self.m, &self.id, stage::BUILDING).await;
                self.restart("land", task, &fact, &note).await;
                return;
            }
            if landed.ok && !landed.commit.is_empty() {
                self.landed(&landed.commit).await;
                return;
            }
            if !landed.rebased {
                let detail = self.m.t("detail_land_failed", &[("why", if landed.text.is_empty() { "the runner gave no reason" } else { &landed.text })]);
                self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], "land").await;
                return;
            }
            // The main branch had moved: the proof was of another base. Build and walk again.
            tries += 1;
            if tries >= LAND_TRIES {
                let detail = self.m.t("detail_land_moving", &[("tries", &tries.to_string())]);
                self.ask("stuck", "subject_stuck", &detail, &["rerun", "take_over", "decline"], "land").await;
                return;
            }
            let facts = self.facts.clone().unwrap_or_default();
            let outcome = gates::run(&self.m, &self.id, &facts, Which::Proof).await;
            self.drop_old_beats();
            if self.pending(&outcome).await {
                return;
            }
            if !outcome.failed.is_empty() {
                self.rework_round(outcome.failed).await;
                return;
            }
            rebased_ok = true;
        }
    }

    /// Landed: the soak build into the owner's apps, the train is told, the worktree goes.
    async fn landed(&mut self, commit: &str) {
        let Some(package) = self.package() else { return };
        let Some(repo) = self.repo(&package) else { return };
        let now = now_ms();
        let soak = policy::soak_min(&repo.policy, package.tier, package.urgent);
        self.m.store.update_package(&self.id, |package| {
            package.landed = commit.to_owned();
            package.landed_ms = now;
            package.soak_until_ms = now + u64::from(soak) * 60_000;
        });
        inbox::close_where(&self.m, "policy", "landed", |ask| ask.ask.package == self.id);
        set_stage(&self.m, &self.id, stage::LANDED).await;
        self.m.notify("done", &self.id, self.m.t("notify_landed", &[("id", &self.id)]), self.m.t("notify_landed_body", &[("commit", commit)]), repo.name.clone()).await;
        // The soak build installs the package's parts into the owner's own apps. A failure is
        // written down; the package has landed all the same.
        let build = runner::BuildParams { job_id: package.job_id.clone(), attempt: package.attempt, app: app_of(&repo), parts: Vec::new(), soak: true };
        let failure = match self.call::<runner::BuildAnswer>(runner::methods::BUILD, json!(build), RUNNER_BUILD).await {
            Ok(answer) if answer.ok => None,
            Ok(answer) => Some(answer.tail.join("\n")),
            Err(error) => Some(error.message),
        };
        if let Some(tail) = failure {
            gates::record(&self.m, &self.id, "build", "failed", vec![cc_protocol::contracts::manage::Finding { title: self.m.t("soak_build_failed", &[]), evidence: tail }]).await;
        }
        tell_train(&self.m, &self.id).await;
        if let Err(error) = self.call::<Value>(runner::methods::REMOVE, json!({"job_id": package.job_id}), RUNNER_CALL).await {
            tracing::warn!(package = %self.id, error = %error.message, "the runner did not remove the worktree");
        }
        set_stage(&self.m, &self.id, stage::SOAKING).await;
        self.m.store.write(|data| data.strips.remove(&self.id));
        self.m.look_now();
        self.done = true;
    }

    // ---- a person decided --------------------------------------------------------------------

    async fn decided(&mut self, ask: &AskRec) {
        let (kind, action, note) = (ask.ask.kind.as_str(), ask.action.as_str(), ask.note.as_str());
        match (kind, action) {
            (_, "decline") => {
                self.stop_job().await;
                let job_id = self.package().map(|package| package.job_id).unwrap_or_default();
                if let Err(error) = self.call::<Value>(runner::methods::REMOVE, json!({"job_id": job_id}), RUNNER_CALL).await {
                    tracing::warn!(package = %self.id, error = %error.message, "the runner did not remove the worktree");
                }
                let reason = if note.is_empty() { self.m.t("declined_by_you", &[]) } else { self.m.t("declined_by_you_note", &[("note", note)]) };
                packages::end(&self.m, &self.id, &reason).await;
                self.done = true;
            }
            (_, "take_over") => {
                // The person works on it themselves: the job stops and the package is theirs
                // until they press Start now.
                self.stop_job().await;
                self.m.store.update_package(&self.id, |package| package.stopped = true);
                self.phase = Phase::Building;
                let sessions = self.package().map(|package| self.m.store.read(|data| data.sessions_of(&package))).unwrap_or_default();
                for session in &sessions {
                    self.m.line(session, &self.m.t("line_taken_over", &[])).await;
                }
            }
            ("blocked_walk", "approve") => {
                self.blocked_ok = true;
                self.land().await;
            }
            ("approve_landing", "approve") => {
                self.landing_ok = true;
                self.blocked_ok = true;
                self.land().await;
            }
            ("approve_landing", "send_back") => {
                // The person's remark goes to the builder: the last task starts again with it.
                let task = self.points.len().max(1) as u32;
                self.landing_ok = false;
                set_stage(&self.m, &self.id, stage::BUILDING).await;
                let fact = self.m.t("fact_sent_back", &[("note", note)]);
                self.restart("person", task, &fact, note).await;
            }
            (_, "rerun") => match ask.resume.as_str() {
                "land" if kind == "stuck" => self.land().await,
                resume => match resume.strip_prefix("task:").and_then(|n| n.parse::<u32>().ok()) {
                    Some(task) => {
                        // A fresh ladder for the task: the person looked and said go on.
                        self.rungs.retain(|(_, of), _| *of != task);
                        self.acted.retain(|(_, of), _| *of != task);
                        self.refusals.remove(&task);
                        self.context_restarts.remove(&task);
                        set_stage(&self.m, &self.id, stage::BUILDING).await;
                        let fact = self.m.t("fact_rerun", &[]);
                        self.restart("person", task, &fact, note).await;
                    }
                    None => self.prove().await,
                },
            },
            _ => tracing::warn!(package = %self.id, kind, action, "an answer this job does not know; nothing was done"),
        }
    }
}

/// A runner task as a plan point (for the validator and the floor).
fn point_of(task: &Task) -> Point {
    Point {
        n: task.n,
        title: task.title.clone(),
        brief: task.brief.clone(),
        known: task.known.clone(),
        rules: task.rules.clone(),
        depends_on: task.depends_on.clone(),
        touches: task.touches.clone(),
        checks: task.checks.clone(),
        request: task.request.clone(),
        ..Point::default()
    }
}

/// Stop and remove a withdrawn package's job, and end its task.
pub fn cancel(manager: &Arc<Manager>, package: &Package) {
    if let Some(sender) = lock(&manager.jobs).get(&package.id) {
        let _ = sender.send(Msg::Cancel);
    }
    let (manager, package) = (manager.clone(), package.clone());
    tokio::spawn(async move {
        let Ok(repo) = manager.repo(&package.repo) else { return };
        let node = Manager::node_of(&package, &repo.repo);
        if package.job_id.is_empty() || node.is_empty() {
            return;
        }
        for (method, params) in [(runner::methods::STOP, json!({"job_id": package.job_id, "attempt": package.attempt})), (runner::methods::REMOVE, json!({"job_id": package.job_id}))] {
            if let Err(error) = manager.runner::<Value>(&node, method, params, RUNNER_CALL).await {
                tracing::warn!(package = %package.id, method, error = %error.message, "the runner did not take the withdrawal");
            }
        }
    });
}
