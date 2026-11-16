//! Soak and the release train (`train@1`, `docs/specs/02-HOLD-PACKAGES-RELEASE.md` §6-7).
//!
//! 1. A landed package soaks until its time passes. A report that names one of its parts holds it
//!    back; without one it becomes ready.
//! 2. The clock looks at the wall clock at least every 30 s (the monotonic clock stops while a Mac
//!    sleeps). At a repository's train time it writes the day into the graph first, so the train
//!    leaves once per day whatever happens after, and then runs the train when soaks are ready.
//!    Nobody presses: the train goes by itself for every tier.
//! 3. A train asks the runner what would be released, releases it, and records the release with
//!    its part moves, its note and the script's last lines. A failed release leaves the soaks
//!    ready for the next day's train.
//!
//! The records are in the graph (`records.rs`); what is in memory here is read from it at start.

use crate::data::Texts;
use crate::graph::{self, Graph};
use crate::records::{self, Clock, Kept, release_state, soak_state};
use crate::{bump, when};
use cc_protocol::contracts::lifecycle::{ProbeCheck, ProbeReport};
use cc_protocol::contracts::manage::{From as Origin, RequestCreateParams, methods as manage};
use cc_protocol::contracts::runner::{ReleaseAnswer, ReleasedPart};
use cc_protocol::contracts::train::{BumpParams, LandedParams, PartMove, Release, ReportParams, Soak, TrainView, methods, topics};
use cc_protocol::host::Host;
use cc_protocol::{Ctx, RpcError, codes, parse_params, to_value};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{Notify, OnceCell};
use tokio_util::sync::CancellationToken;

/// The longest the clock sleeps between looks at the time.
const LOOK_EVERY: Duration = Duration::from_secs(30);
/// How long a plan read from the runner fills in the versions of the next train.
const PLAN_FRESH_MS: u64 = 10 * 60 * 1000;
/// How long a repository's train time is kept before it is asked for again.
const TRAIN_AT_FRESH_MS: u64 = 5 * 60 * 1000;
/// How many releases `train.get` answers.
const SHOWN_RELEASES: usize = 20;
/// How many releases `train.list` answers at most.
const LISTED_RELEASES: usize = 500;
/// The repository id the probe's soak lives under. Nothing else uses it.
const PROBE_REPO: &str = "probe.release-train";
/// How long the probe waits for the memory plugin.
const PROBE_WAIT: Duration = Duration::from_secs(20);

/// What the runner said a release would move, and when.
struct Plan {
    ms: u64,
    parts: Vec<ReleasedPart>,
}

#[derive(Default)]
struct State {
    /// By package.
    soaks: BTreeMap<String, Kept>,
    /// Oldest first.
    releases: Vec<Release>,
    /// By repository.
    clocks: BTreeMap<String, Clock>,
    /// The newest plan per repository. Not a record: it is asked for again when it is old.
    plans: HashMap<String, Plan>,
    /// When a plan was last asked for, so a view does not ask again and again.
    plan_asked: HashMap<String, u64>,
    /// Each repository's train time and when it was read.
    train_at: HashMap<String, (u64, (u32, u32))>,
    /// Repositories whose train is running now.
    running: HashSet<String>,
}

pub struct Train {
    ctx: OnceLock<Ctx>,
    graph: Graph,
    texts: Texts,
    loaded: OnceCell<()>,
    state: Mutex<State>,
    /// Wakes the clock when a soak was added.
    wake: Notify,
}

/// Takes a repository out of `running` when its train ends, however it ends.
struct Running<'a> {
    train: &'a Train,
    repo: String,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.train.with(|state| state.running.remove(&self.repo));
    }
}

/// Whether a part a release moved is a part a package names. The release script may say
/// `plugin cleanup` where a plan says `cleanup`.
fn same_part(moved: &str, named: &str) -> bool {
    let bare = |part: &str| part.trim().trim_start_matches("plugins/").trim_start_matches("plugin ").trim().to_owned();
    bare(moved) == bare(named)
}

fn names_part(soak: &Soak, part: &str) -> bool {
    soak.parts.iter().any(|named| same_part(part, named))
}

/// A request as the release's parts name it: its title, or its id when it has none.
fn carried_as(request: &(String, String, String)) -> String {
    if request.1.trim().is_empty() { request.0.clone() } else { request.1.clone() }
}

impl Train {
    pub fn new(live_dir: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            ctx: OnceLock::new(),
            graph: Graph::new(),
            texts: Texts::new(live_dir),
            loaded: OnceCell::new(),
            state: Mutex::new(State::default()),
            wake: Notify::new(),
        })
    }

    /// Remember the frame connection (the first one) and start writing to the graph.
    pub fn attach(&self, ctx: &Ctx) {
        if self.ctx.set(ctx.clone()).is_ok() {
            self.graph.start(graph::ctx_caller(ctx));
        }
    }

    fn ctx(&self) -> Result<Ctx, RpcError> {
        self.ctx.get().cloned().ok_or_else(|| RpcError::new(codes::UNAVAILABLE, "not connected to the frame yet"))
    }

    fn with<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        change(&mut self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    fn publish_changed(&self, repo: &str) {
        if let Some(ctx) = self.ctx.get() {
            ctx.publish(topics::CHANGED, json!({"repo": repo}));
        }
    }

    // ---- the records ---------------------------------------------------------------------------

    /// Read the records once. A failed read is tried again by the next call.
    async fn ensure_loaded(&self) -> Result<(), RpcError> {
        self.loaded.get_or_try_init(|| self.load()).await.map(|_| ())
    }

    async fn load(&self) -> Result<(), RpcError> {
        let all = |label: &str, name: &str| format!("MATCH ({name}:{label}) WHERE {name}.ended_ms IS NULL RETURN {name}");
        let soaks = self.graph.nodes(&all(records::SOAK, "s"), Map::new(), 5000).await?;
        let releases = self.graph.nodes(&all(records::RELEASE, "r"), Map::new(), 5000).await?;
        let moves = self.graph.nodes(&all(records::PART_MOVE, "m"), Map::new(), 5000).await?;
        let clocks = self.graph.nodes(&all(records::CLOCK, "c"), Map::new(), 5000).await?;

        let mut by_release: HashMap<String, Vec<(u64, PartMove)>> = HashMap::new();
        for props in &moves {
            let (release, n, part) = records::move_from(props);
            by_release.entry(release).or_default().push((n, part));
        }
        let mut releases: Vec<Release> = releases.iter().map(records::release_from).filter(|release| !release.id.is_empty()).collect();
        for release in &mut releases {
            let mut parts = by_release.remove(&release.id).unwrap_or_default();
            parts.sort_by_key(|(n, _)| *n);
            release.parts = parts.into_iter().map(|(_, part)| part).collect();
        }
        releases.sort_by_key(|release| release.ms);
        self.with(|state| {
            // A probe that was cut off may have left its soak; it is never a real one.
            state.soaks = soaks.iter().map(records::soak_from).filter(|kept| !kept.soak.package.is_empty() && kept.soak.repo != PROBE_REPO).map(|kept| (kept.soak.package.clone(), kept)).collect();
            state.releases = releases;
            state.clocks = clocks.iter().map(records::clock_from).filter(|(repo, _)| !repo.is_empty()).collect();
        });
        Ok(())
    }

    fn put_soaks(&self, soaks: &[Kept]) {
        // Nothing is kept unconnected: every soak hangs on this computer's node.
        let edges = soaks.iter().map(|kept| graph::edge((records::SOAK, &kept.soak.package), "ON", graph::machine())).collect();
        self.graph.put(soaks.iter().map(records::soak_node).collect(), edges);
    }

    fn put_clock(&self, repo: &str, clock: &Clock) {
        let node = records::clock_node(repo, clock);
        let edge = graph::edge((&node.label, &node.key), "ON", graph::machine());
        self.graph.put(vec![node], vec![edge]);
    }

    // ---- methods -------------------------------------------------------------------------------

    pub async fn handle(self: &Arc<Self>, method: &str, params: Value) -> Result<Value, RpcError> {
        if method == methods::BUMP {
            let params: BumpParams = parse_params(params)?;
            let version = bump::next(&params.version, &params.tier, params.breaking).map_err(RpcError::invalid_params)?;
            return Ok(json!({"version": version}));
        }
        self.ensure_loaded().await?;
        match method {
            methods::GET => to_value(&self.view(&self.named(&params, "repo")?).await),
            methods::LIST => {
                let repo = self.named(&params, "repo")?;
                let releases: Vec<Release> = self.with(|state| state.releases.iter().rev().filter(|release| release.repo == repo).take(LISTED_RELEASES).cloned().collect());
                Ok(json!({"releases": releases}))
            }
            methods::LANDED => {
                self.landed(parse_params(params)?)?;
                Ok(json!({"ok": true}))
            }
            methods::REPORT => {
                self.report(parse_params(params)?);
                Ok(json!({"ok": true}))
            }
            methods::SKIP => {
                self.skip(&self.named(&params, "repo")?).await;
                Ok(json!({"ok": true}))
            }
            methods::RUN => {
                let repo = self.named(&params, "repo")?;
                // In a task of its own: a caller that goes away must not cut a release in half.
                let this = self.clone();
                let release = tokio::spawn(async move { this.run_train(&repo, None, false).await }).await.map_err(|error| RpcError::internal(format!("the train stopped: {error}")))??;
                to_value(&release)
            }
            methods::TAKE_BACK => {
                let package = self.named(&params, "package")?;
                let this = self.clone();
                let request = tokio::spawn(async move { this.take_back(&package).await }).await.map_err(|error| RpcError::internal(format!("taking back stopped: {error}")))??;
                Ok(json!({"request": request}))
            }
            other => Err(RpcError::unknown_method(other)),
        }
    }

    /// A text parameter that must be there.
    fn named(&self, params: &Value, name: &str) -> Result<String, RpcError> {
        match params.get(name).and_then(Value::as_str).map(str::trim) {
            Some(value) if !value.is_empty() => Ok(value.to_owned()),
            _ if name == "repo" => Err(RpcError::invalid_params(self.texts.get("no_repo", &[("repo", name)]))),
            _ => Err(RpcError::invalid_params(format!("{name} is missing"))),
        }
    }

    /// `train.landed`: the package starts to soak. An urgent one leaves at once on a train of its own.
    fn landed(self: &Arc<Self>, params: LandedParams) -> Result<(), RpcError> {
        if params.package.trim().is_empty() || params.repo.trim().is_empty() {
            return Err(RpcError::invalid_params(self.texts.get("landed_incomplete", &[])));
        }
        let now = when::now_ms();
        let at_once = params.urgent || params.soak_min == 0;
        let kept = Kept {
            soak: Soak {
                package: params.package.clone(),
                repo: params.repo.clone(),
                requests: params.requests,
                parts: params.parts,
                tier: params.tier,
                urgent: params.urgent,
                commit: params.commit,
                landed_ms: now,
                until_ms: if at_once { now } else { now + u64::from(params.soak_min) * 60_000 },
                reports: Vec::new(),
                state: if at_once { soak_state::READY } else { soak_state::SOAKING }.to_owned(),
            },
            ..Kept::default()
        };
        self.with(|state| state.soaks.insert(kept.soak.package.clone(), kept.clone()));
        self.put_soaks(&[kept]);
        self.publish_changed(&params.repo);
        self.wake.notify_one();
        if params.urgent {
            let this = self.clone();
            let (repo, package) = (params.repo, params.package);
            tokio::spawn(async move {
                if let Err(error) = this.run_train(&repo, Some(&package), true).await {
                    // The soak stays ready, so the daily train carries it.
                    tracing::warn!(repo, package, error = %error.message, "the hotfix train did not leave");
                }
            });
        }
        Ok(())
    }

    /// `train.report`: every package of the repository still in its soak that names the part is
    /// held back from the train.
    fn report(&self, params: ReportParams) {
        let held: Vec<Kept> = self.with(|state| {
            let mut held = Vec::new();
            for kept in state.soaks.values_mut() {
                let soak = &mut kept.soak;
                let in_soak = soak.state == soak_state::SOAKING || soak.state == soak_state::HELD_BACK;
                if soak.repo == params.repo && in_soak && names_part(soak, &params.part) {
                    soak.reports.push(params.text.clone());
                    soak.state = soak_state::HELD_BACK.to_owned();
                    held.push(kept.clone());
                }
            }
            held
        });
        if !held.is_empty() {
            self.put_soaks(&held);
            self.publish_changed(&params.repo);
        }
    }

    /// `train.skip`: the next train of the repository does not run.
    async fn skip(&self, repo: &str) {
        let at = self.train_at(repo).await;
        let (day, _) = when::next(when::now_ms(), at, "");
        let clock = self.with(|state| {
            let clock = state.clocks.entry(repo.to_owned()).or_default();
            clock.skip_day = day;
            clock.clone()
        });
        self.put_clock(repo, &clock);
        self.publish_changed(repo);
    }

    /// `train.take_back`: open the urgent request that reverts a released package.
    async fn take_back(&self, package: &str) -> Result<String, RpcError> {
        let vars = [("package", package)];
        // The state is changed first, so a second press while the request is being opened is refused.
        let claimed = self.with(|state| match state.soaks.get_mut(package) {
            None => Err(RpcError::not_found(self.texts.get("no_package", &vars))),
            Some(kept) if kept.soak.state == soak_state::TAKEN_BACK => Err(RpcError::new(codes::CONFLICT, self.texts.get("already_taken_back", &vars))),
            Some(kept) if kept.soak.state != soak_state::RELEASED => Err(RpcError::invalid_params(self.texts.get("not_released", &vars))),
            Some(kept) => {
                kept.soak.state = soak_state::TAKEN_BACK.to_owned();
                Ok(kept.soak.clone())
            }
        })?;
        let requests = claimed.requests.iter().map(|request| format!("{} {}", request.0, request.1).trim().to_owned()).collect::<Vec<_>>().join("; ");
        let parts = claimed.parts.join(", ");
        let text_vars = [("package", package), ("commit", claimed.commit.as_str()), ("parts", parts.as_str()), ("requests", requests.as_str())];
        let params = RequestCreateParams {
            repo: claimed.repo.clone(),
            text: self.texts.get("take_back_text", &text_vars).trim().to_owned(),
            title: self.texts.get("take_back_title", &vars),
            // The person pressed Take back on the Releases page; the kind is left to the assessment.
            from: Origin { kind: "person".into(), name: "Releases".into(), node: String::new() },
            urgent: Some(true),
        };
        let opened = match self.ctx() {
            Ok(ctx) => Host::new(ctx).effect(manage::REQUEST_CREATE, to_value(&params)?).await,
            Err(error) => Err(error),
        };
        let request = opened.and_then(|view| {
            view.get("id").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| RpcError::new(codes::CONTRACT_VIOLATION, "manage.request.create answered no request id"))
        });
        let kept = self.with(|state| {
            let kept = state.soaks.get_mut(package)?;
            match &request {
                Ok(id) => kept.taken_back_request = id.clone(),
                // Nothing was opened: the package is released as before.
                Err(_) => kept.soak.state = soak_state::RELEASED.to_owned(),
            }
            Some(kept.clone())
        });
        let request = request?;
        if let Some(kept) = kept {
            self.put_soaks(&[kept]);
        }
        self.publish_changed(&claimed.repo);
        Ok(request)
    }

    // ---- the view ------------------------------------------------------------------------------

    /// The repository's train time (`manage.repo.get` → `policy.train_at`), `03:00` when it cannot
    /// be read.
    async fn train_at(&self, repo: &str) -> (u32, u32) {
        let now = when::now_ms();
        if let Some(at) = self.with(|state| state.train_at.get(repo).filter(|(read_ms, _)| now.saturating_sub(*read_ms) < TRAIN_AT_FRESH_MS).map(|(_, at)| *at)) {
            return at;
        }
        let answer = match self.ctx() {
            Ok(ctx) => ctx.call(manage::REPO_GET, json!({"id": repo})).await,
            Err(error) => Err(error),
        };
        match answer {
            Ok(found) => {
                let at = when::parse_at(found.pointer("/policy/train_at").and_then(Value::as_str).unwrap_or_default());
                self.with(|state| state.train_at.insert(repo.to_owned(), (now, at)));
                at
            }
            Err(error) => {
                tracing::debug!(repo, error = %error.message, "the repository's train time could not be read");
                // The last one read is better than the default while the manage plugin is away.
                self.with(|state| state.train_at.get(repo).map(|(_, at)| *at)).unwrap_or_else(|| when::parse_at(""))
            }
        }
    }

    /// `train.get`.
    async fn view(self: &Arc<Self>, repo: &str) -> TrainView {
        let at = self.train_at(repo).await;
        let now = when::now_ms();
        let (view, ask_plan) = self.with(|state| {
            let skip_day = state.clocks.get(repo).map(|clock| clock.skip_day.clone()).unwrap_or_default();
            let (_, next_ms) = when::next(now, at, &skip_day);
            let mut released: Vec<Release> = state.releases.iter().rev().filter(|release| release.repo == repo).take(SHOWN_RELEASES).cloned().collect();
            released.sort_by_key(|release| std::cmp::Reverse(release.ms));
            let shown: HashSet<&str> = released.iter().map(|release| release.id.as_str()).collect();
            // What has not left yet, and the packages of the shown releases that were taken back
            // (the view has no other place that says a package is taken back).
            let mut soaking: Vec<Soak> = state
                .soaks
                .values()
                .filter(|kept| kept.soak.repo == repo)
                .filter(|kept| match kept.soak.state.as_str() {
                    soak_state::SOAKING | soak_state::READY | soak_state::HELD_BACK => true,
                    soak_state::TAKEN_BACK => shown.contains(kept.release.as_str()),
                    _ => false,
                })
                .map(|kept| kept.soak.clone())
                .collect();
            soaking.sort_by_key(|soak| soak.landed_ms);

            let ready: Vec<&Soak> = soaking.iter().filter(|soak| soak.state == soak_state::READY).collect();
            let plan = state.plans.get(repo).filter(|plan| now.saturating_sub(plan.ms) < PLAN_FRESH_MS);
            let mut next: Vec<PartMove> = Vec::new();
            for soak in &ready {
                for part in &soak.parts {
                    let carries = soak.requests.iter().map(carried_as);
                    match next.iter_mut().find(|listed| same_part(&listed.part, part)) {
                        Some(listed) => listed.carries.extend(carries),
                        None => next.push(PartMove { part: part.clone(), from: String::new(), to: String::new(), carries: carries.collect() }),
                    }
                }
            }
            if let Some(plan) = plan.filter(|_| !ready.is_empty()) {
                for planned in &plan.parts {
                    match next.iter_mut().find(|listed| same_part(&planned.part, &listed.part)) {
                        Some(listed) => {
                            listed.from = planned.from.clone();
                            listed.to = planned.to.clone();
                        }
                        // The release script would move it too, though no ready package names it.
                        None => next.push(PartMove { part: planned.part.clone(), from: planned.from.clone(), to: planned.to.clone(), carries: Vec::new() }),
                    }
                }
            }
            let asked = state.plan_asked.get(repo).copied().unwrap_or(0);
            let ask_plan = !ready.is_empty() && plan.is_none() && now.saturating_sub(asked) >= PLAN_FRESH_MS && !state.running.contains(repo);
            if ask_plan {
                state.plan_asked.insert(repo.to_owned(), now);
            }
            (TrainView { repo: repo.to_owned(), next_ms, next, soaking, released }, ask_plan)
        });
        if ask_plan {
            // The versions come from the runner, which may be slow: the view answers without them
            // and is asked for again when they are here.
            let (this, repo) = (self.clone(), repo.to_owned());
            tokio::spawn(async move {
                match this.ask_release(&repo, true).await {
                    Ok(answer) if answer.ok => {
                        this.with(|state| state.plans.insert(repo.clone(), Plan { ms: when::now_ms(), parts: answer.parts }));
                        this.publish_changed(&repo);
                    }
                    Ok(_) => tracing::debug!(repo, "the runner could not plan a release"),
                    Err(error) => tracing::debug!(repo, error = %error.message, "the runner could not plan a release"),
                }
            });
        }
        view
    }

    // ---- the train -----------------------------------------------------------------------------

    /// `manage.runner.release` on the repository's runner. The plan is a read and is asked again
    /// after an interruption; the release itself is never replayed.
    async fn ask_release(&self, repo: &str, plan_only: bool) -> Result<ReleaseAnswer, RpcError> {
        let host = Host::new(self.ctx()?);
        let params = json!({"repo": repo, "plan_only": plan_only});
        let answer = if plan_only { host.call(manage::RUNNER_RELEASE, params).await } else { host.effect(manage::RUNNER_RELEASE, params).await }?;
        parse_params(answer)
    }

    /// Run the repository's train now: every ready package, or only `only` (a hotfix train).
    async fn run_train(&self, repo: &str, only: Option<&str>, hotfix: bool) -> Result<Release, RpcError> {
        self.ensure_loaded().await?;
        if !self.with(|state| state.running.insert(repo.to_owned())) {
            return Err(RpcError::new(codes::BUSY, self.texts.get("busy", &[("repo", repo)])));
        }
        let _running = Running { train: self, repo: repo.to_owned() };
        let carried: Vec<Soak> = self.with(|state| {
            let ready = state.soaks.values().map(|kept| &kept.soak).filter(|soak| soak.repo == repo && soak.state == soak_state::READY);
            ready.filter(|soak| only.is_none_or(|package| soak.package == package)).cloned().collect()
        });
        let mut release = Release {
            id: format!("rel-{}", uuid::Uuid::new_v4().simple()),
            repo: repo.to_owned(),
            ms: when::now_ms(),
            packages: carried.iter().map(|soak| soak.package.clone()).collect(),
            hotfix,
            ..Release::default()
        };

        // 1. What would move.
        let plan = match self.ask_release(repo, true).await {
            Ok(plan) if plan.ok => plan,
            Ok(plan) => return Ok(self.record_failed(release, "note_plan_failed", plan.tail)),
            Err(error) => return Ok(self.record_failed(release, "note_plan_failed", vec![error.message])),
        };
        self.with(|state| state.plans.insert(repo.to_owned(), Plan { ms: when::now_ms(), parts: plan.parts.clone() }));
        if plan.parts.is_empty() {
            // Nothing to build: what the packages changed needs no part release, so they are done.
            release.state = release_state::NOTHING.to_owned();
            release.note = self.texts.get("note_nothing", &[]);
            release.tail = plan.tail;
            return Ok(self.record(release, &carried));
        }

        // 2. Release it.
        let ran = match self.ask_release(repo, false).await {
            Ok(ran) if ran.ok => ran,
            Ok(ran) => return Ok(self.record_failed(release, "note_failed", ran.tail)),
            Err(error) => return Ok(self.record_failed(release, "note_failed", vec![error.message])),
        };

        // 3. Record it.
        let moved = if ran.parts.is_empty() { plan.parts } else { ran.parts };
        release.parts = moved
            .iter()
            .map(|part| PartMove {
                part: part.part.clone(),
                from: part.from.clone(),
                to: part.to.clone(),
                carries: carried.iter().filter(|soak| names_part(soak, &part.part)).flat_map(|soak| soak.requests.iter().map(carried_as)).collect(),
            })
            .collect();
        release.note = self.note(repo, &release.parts, &carried);
        release.tail = ran.tail;
        release.state = release_state::RAN.to_owned();
        // The versions moved: the plan no longer says what the next train would do.
        self.with(|state| state.plans.remove(repo));
        Ok(self.record(release, &carried))
    }

    /// The release note: one line per request, grouped by part.
    fn note(&self, repo: &str, parts: &[PartMove], carried: &[Soak]) -> String {
        let line = |request: &(String, String, String)| format!("  {} {} ({})", request.0, request.1, request.2);
        let mut lines = Vec::new();
        for part in parts {
            lines.push(if part.from.is_empty() { format!("{} {}", part.part, part.to) } else { format!("{} {} → {}", part.part, part.from, part.to) });
            lines.extend(carried.iter().filter(|soak| names_part(soak, &part.part)).flat_map(|soak| soak.requests.iter().map(line)));
        }
        // A request whose package names no part that moved still gets its line.
        let apart: Vec<String> = carried.iter().filter(|soak| !parts.iter().any(|part| names_part(soak, &part.part))).flat_map(|soak| soak.requests.iter().map(line)).collect();
        if !apart.is_empty() {
            lines.push(self.texts.get("note_no_part", &[]));
            lines.extend(apart);
        }
        // The release script releases what is on the main line, so a moved part may hold the
        // changes of a package that is still in its soak. The note says so.
        let unsoaked: Vec<(String, String)> = self.with(|state| {
            let waiting = state.soaks.values().map(|kept| &kept.soak).filter(|soak| soak.repo == repo && (soak.state == soak_state::SOAKING || soak.state == soak_state::HELD_BACK));
            waiting.flat_map(|soak| parts.iter().filter(|part| names_part(soak, &part.part)).map(|part| (part.part.clone(), soak.package.clone()))).collect()
        });
        for (part, package) in unsoaked {
            lines.push(self.texts.get("note_unsoaked", &[("part", &part), ("package", &package)]));
        }
        lines.join("\n")
    }

    /// Write a release that shipped `carried`, mark those soaks released and tell everyone.
    fn record(&self, release: Release, carried: &[Soak]) -> Release {
        let shipped: Vec<Kept> = self.with(|state| {
            let mut shipped = Vec::new();
            for soak in carried {
                // Only what is still ready: a package that landed again meanwhile soaks anew.
                if let Some(kept) = state.soaks.get_mut(&soak.package).filter(|kept| kept.soak.state == soak_state::READY) {
                    kept.soak.state = soak_state::RELEASED.to_owned();
                    kept.release = release.id.clone();
                    shipped.push(kept.clone());
                }
            }
            state.releases.push(release.clone());
            shipped
        });
        let packages: Vec<String> = shipped.iter().map(|kept| kept.soak.package.clone()).collect();
        let (mut nodes, mut edges) = records::release_nodes(&release, &packages);
        edges.push(graph::edge((records::RELEASE, &release.id), "ON", graph::machine()));
        nodes.extend(shipped.iter().map(records::soak_node));
        self.graph.put(nodes, edges);
        if let Some(ctx) = self.ctx.get() {
            ctx.publish(topics::RELEASED, json!(release));
        }
        self.publish_changed(&release.repo);
        release
    }

    /// Write a release that shipped nothing (`failed` or `skipped`); the soaks stay ready.
    fn record_unshipped(&self, release: Release) -> Release {
        self.with(|state| state.releases.push(release.clone()));
        let (nodes, mut edges) = records::release_nodes(&release, &[]);
        edges.push(graph::edge((records::RELEASE, &release.id), "ON", graph::machine()));
        self.graph.put(nodes, edges);
        self.publish_changed(&release.repo);
        release
    }

    fn record_failed(&self, mut release: Release, note: &str, tail: Vec<String>) -> Release {
        release.state = release_state::FAILED.to_owned();
        release.note = self.texts.get(note, &[]);
        release.tail = if tail.is_empty() { vec![self.texts.get("no_tail", &[])] } else { tail };
        tracing::warn!(repo = release.repo, tail = ?release.tail.last(), "the release train failed");
        self.record_unshipped(release)
    }

    // ---- the clock -----------------------------------------------------------------------------

    /// Look at the wall clock at least every 30 s until `stop`.
    pub fn spawn_clock(self: &Arc<Self>, stop: CancellationToken) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let soonest = this.tick().await;
                let wait = soonest.map(|until| Duration::from_millis(until.saturating_sub(when::now_ms()).max(200))).unwrap_or(LOOK_EVERY).min(LOOK_EVERY);
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = this.wake.notified() => {}
                    _ = stop.cancelled() => return,
                }
            }
        });
    }

    /// One look: soaks whose time passed become ready, due trains leave. Answers when the next
    /// soak ends.
    async fn tick(self: &Arc<Self>) -> Option<u64> {
        if let Err(error) = self.ensure_loaded().await {
            tracing::debug!(error = %error.message, "the train's records are not read yet");
            return None;
        }
        let now = when::now_ms();
        let (ripened, waiting) = self.with(|state| {
            let mut ripened = Vec::new();
            for kept in state.soaks.values_mut() {
                let soak = &mut kept.soak;
                if soak.state == soak_state::SOAKING && soak.until_ms <= now && soak.reports.is_empty() {
                    soak.state = soak_state::READY.to_owned();
                    ripened.push(kept.clone());
                }
            }
            // Every repository with a package that has not left yet has a train to look at.
            let waiting: BTreeSet<String> = state
                .soaks
                .values()
                .map(|kept| &kept.soak)
                .filter(|soak| matches!(soak.state.as_str(), soak_state::SOAKING | soak_state::READY | soak_state::HELD_BACK))
                .map(|soak| soak.repo.clone())
                .collect();
            (ripened, waiting)
        });
        if !ripened.is_empty() {
            self.put_soaks(&ripened);
            for repo in ripened.iter().map(|kept| kept.soak.repo.as_str()).collect::<BTreeSet<_>>() {
                self.publish_changed(repo);
            }
        }

        let mut leaving = Vec::new();
        for repo in waiting {
            let at = self.train_at(&repo).await;
            let (today, leaves) = when::today(now, at);
            if now < leaves {
                continue;
            }
            // The day is written before anything runs: one train per day, also after a restart or
            // a failure. A package that becomes ready later in the day waits for tomorrow's train.
            let claimed = self.with(|state| {
                let clock = state.clocks.entry(repo.clone()).or_default();
                if clock.ran_day == today {
                    return None;
                }
                clock.ran_day = today.clone();
                let has_ready = state.soaks.values().any(|kept| kept.soak.repo == repo && kept.soak.state == soak_state::READY);
                Some((state.clocks.get(&repo).cloned().unwrap_or_default(), has_ready))
            });
            let Some((clock, has_ready)) = claimed else { continue };
            self.put_clock(&repo, &clock);
            if !has_ready {
                continue;
            }
            if clock.skip_day == today {
                let packages = self.with(|state| state.soaks.values().filter(|kept| kept.soak.repo == repo && kept.soak.state == soak_state::READY).map(|kept| kept.soak.package.clone()).collect());
                self.record_unshipped(Release {
                    id: format!("rel-{}", uuid::Uuid::new_v4().simple()),
                    repo: repo.clone(),
                    ms: now,
                    packages,
                    note: self.texts.get("note_skipped", &[]),
                    state: release_state::SKIPPED.to_owned(),
                    ..Release::default()
                });
                continue;
            }
            leaving.push(repo);
        }
        if !leaving.is_empty() {
            // One after the other (the release machine is one computer), and apart from the clock,
            // which keeps looking while a release builds.
            let this = self.clone();
            tokio::spawn(async move {
                for repo in leaving {
                    if let Err(error) = this.run_train(&repo, None, false).await {
                        tracing::warn!(repo, error = %error.message, "the daily train did not leave");
                    }
                }
            });
        }
        self.with(|state| state.soaks.values().map(|kept| &kept.soak).filter(|soak| soak.state == soak_state::SOAKING && soak.reports.is_empty()).map(|soak| soak.until_ms).min())
    }

    // ---- probe ---------------------------------------------------------------------------------

    /// The bump cases, and a soak written to the graph, read back and retired.
    pub async fn probe(&self) -> ProbeReport {
        let mut checks = Vec::new();
        let started = std::time::Instant::now();
        let cases: [(&str, &str, bool, Option<&str>); 12] = [
            ("0.4.12", "safe", false, Some("0.4.13")),
            ("0.3.99", "safe", false, Some("0.4.0")),
            ("0.99.99", "safe", false, Some("1.0.0")),
            ("0.4.12", "minor", false, Some("0.5.0")),
            ("0.99.3", "minor", false, Some("1.0.0")),
            ("0.4.12", "major", false, Some("0.5.0")),
            ("0.4.12", "major", true, Some("1.0.0")),
            ("99.99.99", "safe", false, None),
            ("99.4.12", "major", true, None),
            ("0.4", "safe", false, None),
            ("0.4.100", "safe", false, None),
            ("0.4.12", "urgent", false, None),
        ];
        let wrong: Vec<String> = cases
            .iter()
            .filter(|(version, tier, breaking, expected)| bump::next(version, tier, *breaking).ok().as_deref() != *expected)
            .map(|(version, tier, breaking, _)| format!("{version} {tier}{}", if *breaking { " breaking" } else { "" }))
            .collect();
        checks.push(ProbeCheck { name: "bump".into(), ok: wrong.is_empty(), detail: if wrong.is_empty() { format!("{} cases", cases.len()) } else { format!("wrong: {}", wrong.join(", ")) }, ms: started.elapsed().as_millis() as u64 });

        let started = std::time::Instant::now();
        let (ok, detail) = self.probe_soak().await;
        checks.push(ProbeCheck { name: "soak in the graph".into(), ok, detail, ms: started.elapsed().as_millis() as u64 });
        ProbeReport::from_checks(checks)
    }

    async fn probe_soak(&self) -> (bool, String) {
        let now = when::now_ms();
        let kept = Kept {
            soak: Soak {
                package: format!("probe-{}", uuid::Uuid::new_v4().simple()),
                repo: PROBE_REPO.into(),
                requests: vec![("PR-0001".into(), "A probe request".into(), "safe".into()), ("PR-0002".into(), "Another one".into(), "minor".into())],
                parts: vec!["frame".into(), "probe".into()],
                tier: "minor".into(),
                urgent: false,
                commit: "0000000".into(),
                landed_ms: now,
                until_ms: now + 60_000,
                reports: vec!["a probe report".into()],
                state: soak_state::HELD_BACK.into(),
            },
            ..Kept::default()
        };
        let own = graph::at(records::SOAK, &kept.soak.package);
        let find = |live_only: bool| format!("MATCH (s:{}) WHERE s.package = $package{} RETURN s", records::SOAK, if live_only { " AND s.ended_ms IS NULL" } else { "" });
        let params: Map<String, Value> = graph::props([("package", json!(kept.soak.package))]);

        self.put_soaks(std::slice::from_ref(&kept));
        let round_trip = async {
            if !self.graph.flush().await {
                return Err("the memory plugin refused the soak".to_owned());
            }
            let found = self.graph.nodes(&find(false), params.clone(), 5).await.map_err(|error| error.message)?;
            match found.first().map(records::soak_from) {
                Some(read) if read == kept => Ok(()),
                Some(_) => Err("the soak read back differs from the one written".to_owned()),
                None => Err("the soak written was not found".to_owned()),
            }
        };
        let written = tokio::time::timeout(PROBE_WAIT, round_trip).await;
        // Retired whatever happened: queued after the put, so it also follows a put that is late.
        self.graph.retire(vec![own], "probe");
        match written {
            // The memory plugin starts on its own time. The writes wait in order and the train's
            // methods wait for it too, so this is not a fault of this plugin.
            Err(_) => return (true, "the memory plugin does not serve yet; the soak was not read back".to_owned()),
            Ok(Err(detail)) => return (false, detail),
            Ok(Ok(())) => {}
        }
        let retired = async {
            self.graph.flush().await;
            self.graph.nodes(&find(true), params.clone(), 5).await.map(|left| left.is_empty()).map_err(|error| error.message)
        };
        match tokio::time::timeout(PROBE_WAIT, retired).await {
            Ok(Ok(true)) => (true, "written, read back the same, retired".to_owned()),
            Ok(Ok(false)) => (false, "the probe's soak is still there after it was retired".to_owned()),
            Ok(Err(detail)) => (false, detail),
            Err(_) => (true, "written and read back the same; the memory plugin did not answer the retire in time".to_owned()),
        }
    }
}
