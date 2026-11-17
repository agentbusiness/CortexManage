//! The manager: what every part shares (the frame connection, the records, the texts, the runner
//! links, the running jobs), the `manage.*` methods, the events it listens to, and the clock.
//!
//! Calls to the chat flow, the charter, the standard and the train can fail because that plugin
//! is not running. That is "not available", never an answer: the request or package stays where
//! it is with a plain note in its thread ([`Manager::unavailable`]) and is tried again when the
//! plugin's `plugin.state` / `plugin.swapped` event says it is active, and every ten minutes.

use crate::link::{LinkEvent, Links};
use crate::store::{Data, RepoRec, ReqRec, Store};
use crate::texts::Texts;
use crate::{graph, inbox, job, packages, pipeline, policy, words};
use cc_protocol::contracts::frame::{self as fc, SessionCreateParams, SessionKind, SessionUpdateParams};
use cc_protocol::contracts::lifecycle::{ProbeCheck, ProbeReport};
use cc_protocol::contracts::manage::{
    Changed, DecideParams, DiscoverParams, From, Package, PackageEditParams, PackageView, Point, Policy, Repo, RepoCreateParams, RepoEditParams, Request, RequestCreateParams,
    RequestEditParams, RequestListParams, RequestView, SayParams, Tier, ended, methods, stage, topics,
};
use cc_protocol::contracts::memory::{GraphLayer, GraphPutParams, GraphQueryParams, GraphQueryResult, GraphRetireParams, MachineInfo, methods as memory_methods};
use cc_protocol::contracts::runner::{self, Facts};
use cc_protocol::contracts::service::{Audience, Caller, Share, ShareList, SharedMethod};
use cc_protocol::contracts::train;
use cc_protocol::contracts::{category_of, now_ms};
use cc_protocol::host::Host;
use cc_protocol::{Ctx, RpcError, codes, parse_params, to_value};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{Notify, mpsc, watch};
use tokio_util::sync::CancellationToken;

/// How often the clock looks at the wall time (holds that ended, beats that stopped).
const TICK: Duration = Duration::from_secs(15);
/// Something that waits for a plugin is tried again after this long, whatever events say.
const RETRY_AFTER_MS: u64 = 10 * 60_000;
/// How long a method waits for the records to be read after a start.
const READY_WAIT: Duration = Duration::from_secs(30);
/// Calls to a runner: an ordinary one, and the long ones.
pub const RUNNER_CALL: Duration = Duration::from_secs(120);
pub const RUNNER_PREPARE: Duration = Duration::from_secs(20 * 60);
pub const RUNNER_BUILD: Duration = Duration::from_secs(2 * 60 * 60);
pub const RUNNER_LAND: Duration = Duration::from_secs(30 * 60);
/// The flow every request's session belongs to.
const FLOW: &str = "manage";
/// The shared service: requests from the other apps of the mesh.
const SERVICE: &str = "manage.request";

/// Something that could not go on because a plugin did not answer.
struct Waiting {
    category: String,
    tried_ms: u64,
}

pub struct Manager {
    ctx: OnceLock<Ctx>,
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub store: Store,
    pub texts: Texts,
    pub links: Links,
    link_events: Mutex<Option<mpsc::UnboundedReceiver<LinkEvent>>>,
    /// The running jobs, by package.
    pub jobs: Mutex<HashMap<String, mpsc::UnboundedSender<job::Msg>>>,
    /// Packages whose job is being started right now.
    pub dispatching: Mutex<HashSet<String>>,
    /// The newest pipeline run of each request; an older one stops at its next step.
    runs: Mutex<HashMap<String, u64>>,
    waiting: Mutex<BTreeMap<String, Waiting>>,
    loaded: watch::Sender<bool>,
    load_error: Mutex<String>,
    kick: Notify,
    pub stop: CancellationToken,
    started: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Manager {
    pub fn new(home: PathBuf, data_dir: PathBuf, live_dir: Option<PathBuf>) -> Arc<Self> {
        let (links, link_events) = Links::new();
        Arc::new(Self {
            ctx: OnceLock::new(),
            home,
            data_dir,
            store: Store::new(),
            texts: Texts::new(live_dir),
            links,
            link_events: Mutex::new(Some(link_events)),
            jobs: Mutex::new(HashMap::new()),
            dispatching: Mutex::new(HashSet::new()),
            runs: Mutex::new(HashMap::new()),
            waiting: Mutex::new(BTreeMap::new()),
            loaded: watch::channel(false).0,
            load_error: Mutex::new(String::new()),
            kick: Notify::new(),
            stop: CancellationToken::new(),
            started: AtomicBool::new(false),
        })
    }

    /// Take the frame connection (once) and start writing records through it.
    pub fn attach(&self, ctx: &Ctx) {
        if self.ctx.set(ctx.clone()).is_ok() {
            self.store.graph.start(graph::ctx_caller(ctx));
        }
    }

    pub fn ctx(&self) -> Result<&Ctx, RpcError> {
        self.ctx.get().ok_or_else(|| RpcError::new(codes::UNAVAILABLE, "not connected to the frame yet"))
    }

    pub fn host(&self) -> Result<Host, RpcError> {
        Ok(Host::new(self.ctx()?.clone()))
    }

    /// A text of `data/texts.toml` with its `{name}`s filled in.
    pub fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
        self.texts.get(key, vars)
    }

    /// Call another plugin once and read its answer as `T`. No retries here: a plugin that does
    /// not serve is "not available", handled by the caller.
    pub async fn peer<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RpcError> {
        let value = self.ctx()?.call(method, params).await?;
        serde_json::from_value(value).map_err(|error| RpcError::new(codes::CONTRACT_VIOLATION, format!("{method}: unexpected answer: {error}")))
    }

    /// Call the runner at `node`.
    pub async fn runner<T: DeserializeOwned>(&self, node: &str, method: &str, params: Value, limit: Duration) -> Result<T, RpcError> {
        self.links.call(self.ctx()?, node, method, params, limit).await
    }

    pub fn changed(&self, what: &str, id: &str, repo: &str) {
        if let Some(ctx) = self.ctx.get() {
            ctx.publish(topics::CHANGED, json!(Changed { what: what.into(), id: id.into(), repo: repo.into() }));
        }
    }

    /// Tell the person through the notify plugin (macOS notification, or the window while it is
    /// open). Best effort: a notification that cannot be sent changes nothing else.
    pub async fn notify(&self, kind: &str, key: &str, title: String, body: String, subtitle: String) {
        let notification = json!({"kind": kind, "title": title, "body": body, "key": key, "subtitle": subtitle, "target": {"open": "manage", "id": key}, "actions": []});
        if let Err(error) = self.peer::<Value>("notify.send", notification).await {
            tracing::debug!(error = %error.message, "the notification was not sent");
        }
    }

    /// Append an entry to a request's thread. 0 when the frame refused it (logged).
    pub async fn entry(&self, session: &str, kind: &str, data: Value) -> u64 {
        if session.is_empty() {
            return 0;
        }
        let Ok(host) = self.host() else { return 0 };
        match host.journal_append(session, kind, data).await {
            Ok(seq) => seq,
            Err(error) => {
                tracing::warn!(session, kind, error = %error.message, "the frame refused a thread entry");
                0
            }
        }
    }

    pub async fn entry_all(&self, sessions: &[String], kind: &str, data: Value) {
        for session in sessions {
            self.entry(session, kind, data.clone()).await;
        }
    }

    /// One of the manager's short lines in a thread.
    pub async fn line(&self, session: &str, text: &str) {
        if !text.is_empty() {
            // `status`: a line about what the manager did, not an answer to the person.
            self.entry(session, fc::kinds::ASSISTANT_MESSAGE, json!({"text": text, "streaming": false, "status": true})).await;
        }
    }

    pub async fn note(&self, session: &str, text: &str) {
        self.entry(session, fc::kinds::NOTE, json!({"text": text})).await;
    }

    // ---- not available -------------------------------------------------------------------------

    /// A call failed: the thing named by `key` (`request:<id>`, `package:<id>`, `train:<id>`,
    /// `runner:<id>`) waits for the plugin of `method`. The thread gets one note per wait.
    pub async fn unavailable(&self, key: &str, method: &str, sessions: &[String], error: &RpcError) {
        let category = category_of(method).to_owned();
        let new = {
            let mut waiting = lock(&self.waiting);
            match waiting.get_mut(key) {
                Some(entry) => {
                    entry.tried_ms = now_ms();
                    entry.category = category.clone();
                    false
                }
                None => {
                    waiting.insert(key.to_owned(), Waiting { category: category.clone(), tried_ms: now_ms() });
                    true
                }
            }
        };
        tracing::warn!(key, method, code = %error.code, error = %error.message, "not available; it waits");
        if new {
            let what = match category.as_str() {
                "charter" => self.t("name_charter", &[]),
                "standard" => self.t("name_standard", &[]),
                "train" => self.t("name_train", &[]),
                "runner" => self.t("name_runner", &[]),
                _ => self.t("name_flow", &[]),
            };
            let text = self.t("not_available", &[("what", &what)]);
            for session in sessions {
                self.note(session, &text).await;
            }
        }
    }

    pub fn settled(&self, key: &str) {
        lock(&self.waiting).remove(key);
    }

    /// Try again what waits for `category` (or, with None, what has waited ten minutes).
    pub fn retry(self: &Arc<Self>, category: Option<&str>) {
        let now = now_ms();
        let due: Vec<String> = {
            let waiting = lock(&self.waiting);
            waiting
                .iter()
                .filter(|(_, entry)| match category {
                    Some(category) => entry.category == category,
                    None => now.saturating_sub(entry.tried_ms) >= RETRY_AFTER_MS,
                })
                .map(|(key, _)| key.clone())
                .collect()
        };
        for key in due {
            // The entry stays until the work gets past the call, so the thread is not told twice.
            if let Some(entry) = lock(&self.waiting).get_mut(&key) {
                entry.tried_ms = now;
            }
            match key.split_once(':') {
                Some(("request", id)) => pipeline::spawn(self, id),
                Some(("package", id)) => job::tell(self, id, job::Msg::Retry),
                Some(("train", id)) => {
                    let (manager, id) = (self.clone(), id.to_owned());
                    tokio::spawn(async move { job::tell_train(&manager, &id).await });
                }
                // A package whose runner was away is looked at by the clock.
                _ => self.kick.notify_one(),
            }
        }
    }

    // ---- pipeline runs -------------------------------------------------------------------------

    /// Begin a new pipeline run of a request; an older run stops at its next step.
    pub fn next_run(&self, request: &str) -> u64 {
        let mut runs = lock(&self.runs);
        let run = runs.entry(request.to_owned()).or_insert(0);
        *run += 1;
        *run
    }

    pub fn is_current(&self, request: &str, run: u64) -> bool {
        lock(&self.runs).get(request).copied() == Some(run)
    }

    /// Look at the holds now, not at the next tick.
    pub fn look_now(&self) {
        self.kick.notify_one();
    }

    // ---- start ---------------------------------------------------------------------------------

    /// Read the records, then go on with everything that was under way. Called once.
    pub fn activate(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let manager = self.clone();
        tokio::spawn(async move {
            loop {
                match manager.store.load().await {
                    Ok(()) => break,
                    Err(error) => {
                        tracing::warn!(error = %error.message, "the records cannot be read from the graph yet; trying again");
                        *lock(&manager.load_error) = error.message;
                    }
                }
                tokio::select! {
                    _ = manager.stop.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                }
            }
            // `send` changes nothing while nobody listens; this always sets the value.
            manager.loaded.send_replace(true);
            manager.go_on();
            manager.clock().await;
        });
        if let Some(mut events) = lock(&self.link_events).take() {
            let manager = self.clone();
            tokio::spawn(async move {
                loop {
                    let event = tokio::select! {
                        _ = manager.stop.cancelled() => return,
                        event = events.recv() => event,
                    };
                    match event {
                        Some(LinkEvent::Beat(facts)) => job::beat(&manager, *facts),
                        Some(LinkEvent::Up(node)) => job::runner_up(&manager, &node),
                        None => return,
                    }
                }
            });
        }
    }

    /// After a start: open the runner links, take up the requests that were being assessed,
    /// planned or validated, and the jobs that were running.
    fn go_on(self: &Arc<Self>) {
        let (nodes, requests, packages) = self.store.read(|data| {
            let nodes: Vec<String> = data.repos.values().filter(|rec| rec.repo.managed && !rec.repo.runner.is_empty()).map(|rec| rec.repo.runner.clone()).collect();
            let requests: Vec<String> = data
                .requests
                .values()
                .filter(|rec| rec.r.ended_as.is_empty() && matches!(rec.r.stage.as_str(), stage::ARRIVED | stage::ASSESSED | stage::PLANNED | stage::VALIDATED))
                .map(|rec| rec.r.id.clone())
                .collect();
            let packages: Vec<String> = data
                .packages
                .values()
                .filter(|package| matches!(package.stage.as_str(), stage::BUILDING | stage::PROVING) && !package.session_id.is_empty())
                .map(|package| package.id.clone())
                .collect();
            (nodes, requests, packages)
        });
        if let Ok(ctx) = self.ctx() {
            for node in nodes {
                self.links.ensure(ctx, &node);
            }
        }
        for id in requests {
            pipeline::spawn(self, &id);
        }
        for id in packages {
            job::ensure(self, &id);
        }
    }

    /// The clock: every [`TICK`] (or when asked) look at the wall time and do what is due. It
    /// never sleeps towards a far moment, so a computer that slept does not start things late.
    async fn clock(self: &Arc<Self>) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = self.stop.cancelled() => return,
                _ = tick.tick() => {}
                _ = self.kick.notified() => {}
            }
            packages::tick(self);
            job::tick(self);
            self.retry(None);
        }
    }

    async fn ready(&self) -> Result<(), RpcError> {
        let mut loaded = self.loaded.subscribe();
        match tokio::time::timeout(READY_WAIT, loaded.wait_for(|done| *done)).await {
            Ok(Ok(_)) => Ok(()),
            _ => {
                let why = lock(&self.load_error).clone();
                Err(RpcError::new(codes::UNAVAILABLE, format!("the records are still being read from the graph ({why})")).retryable())
            }
        }
    }

    pub fn shutdown(&self) {
        self.stop.cancel();
        self.links.stop();
    }

    // ---- events --------------------------------------------------------------------------------

    pub async fn event(self: &Arc<Self>, topic: &str, data: Value) {
        if topic == fc::topics::PLUGIN_STATE {
            let state = data.get("state").and_then(Value::as_str).unwrap_or_default();
            if state == "active"
                && let Some(category) = data.get("category").and_then(Value::as_str)
            {
                self.retry(Some(category));
            }
        } else if topic == fc::topics::PLUGIN_SWAPPED {
            if let Some(category) = data.get("category").and_then(Value::as_str) {
                self.retry(Some(category));
            }
        } else if topic == train::topics::RELEASED {
            match serde_json::from_value::<train::Release>(data) {
                Ok(release) => job::released(self, &release).await,
                Err(error) => tracing::warn!(%error, "a `train.released` event that is not a release; ignored"),
            }
        }
    }

    // ---- lookups -------------------------------------------------------------------------------

    pub fn repo(&self, id: &str) -> Result<RepoRec, RpcError> {
        self.store.read(|data| data.repos.get(id).cloned()).ok_or_else(|| RpcError::not_found(format!("no repository `{id}`")))
    }

    pub fn request(&self, id: &str) -> Result<ReqRec, RpcError> {
        self.store.read(|data| data.requests.get(id).cloned()).ok_or_else(|| RpcError::not_found(format!("no request `{id}`")))
    }

    pub fn package(&self, id: &str) -> Result<Package, RpcError> {
        self.store.read(|data| data.packages.get(id).cloned()).ok_or_else(|| RpcError::not_found(format!("no package `{id}`")))
    }

    fn request_view_of(&self, id: &str) -> Result<RequestView, RpcError> {
        self.store.read(|data| data.requests.get(id).map(|rec| request_view(data, rec))).ok_or_else(|| RpcError::not_found(format!("no request `{id}`")))
    }

    pub fn package_view_of(&self, id: &str) -> Result<PackageView, RpcError> {
        self.store.read(|data| data.packages.get(id).map(|package| package_view(data, package))).ok_or_else(|| RpcError::not_found(format!("no package `{id}`")))
    }

    /// The repository's runner node, or a plain error when it has none.
    fn runner_of(&self, repo: &Repo) -> Result<String, RpcError> {
        if repo.runner.is_empty() {
            return Err(RpcError::new(codes::UNAVAILABLE, self.t("no_runner", &[])));
        }
        Ok(repo.runner.clone())
    }

    /// The frame project a repository's sessions live under (`<home>/repos/<id>`), opened when
    /// first wanted.
    async fn project(&self, rec: &RepoRec) -> Result<String, RpcError> {
        let root = self.home.join("repos").join(&rec.repo.id);
        tokio::fs::create_dir_all(&root).await.map_err(|error| RpcError::internal(format!("cannot create {}: {error}", root.display())))?;
        let project: fc::Project = parse_params(self.host()?.effect(fc::methods::PROJECT_OPEN, json!({"root": root.to_string_lossy()})).await?)?;
        if project.id != rec.project_id {
            self.store.update_repo(&rec.repo.id, |rec| rec.project_id = project.id.clone());
        }
        Ok(project.id)
    }

    // ---- methods -------------------------------------------------------------------------------

    pub async fn handle(self: &Arc<Self>, method: &str, params: Value) -> Result<Value, RpcError> {
        if method == methods::SHARES {
            return to_value(&shares());
        }
        self.ready().await?;
        match method {
            methods::REPO_LIST => Ok(json!({"repos": self.store.read(|data| data.repos.values().map(|rec| rec.repo.clone()).collect::<Vec<_>>())})),
            methods::REPO_GET => to_value(&self.repo(&id_of(&params)?)?.repo),
            methods::REPO_CREATE => self.repo_create(parse_params(params)?).await,
            methods::REPO_EDIT => self.repo_edit(parse_params(params)?),
            methods::REQUEST_CREATE => self.request_create(params).await,
            methods::REQUEST_GET => to_value(&self.request_view_of(&id_of(&params)?)?),
            methods::REQUEST_LIST => self.request_list(parse_params(params)?),
            methods::REQUEST_EDIT => self.request_edit(parse_params(params)?).await,
            methods::REQUEST_SAY => self.request_say(parse_params(params)?).await,
            methods::PACKAGE_GET => to_value(&self.package_view_of(&id_of(&params)?)?),
            methods::PACKAGE_LIST => {
                let repo = params.get("repo").and_then(Value::as_str).unwrap_or_default().to_owned();
                let packages: Vec<PackageView> =
                    self.store.read(|data| data.packages.values().filter(|package| repo.is_empty() || package.repo == repo).map(|package| package_view(data, package)).collect());
                Ok(json!({"packages": packages}))
            }
            methods::PACKAGE_EDIT => {
                let params: PackageEditParams = parse_params(params)?;
                packages::edit(self, &params).await?;
                to_value(&self.package_view_of(&params.id)?)
            }
            methods::DECIDE => {
                let params: DecideParams = parse_params(params)?;
                inbox::decide(self, &params).await?;
                Ok(json!({"ok": true}))
            }
            methods::INBOX_LIST => to_value(&inbox::list(self)),
            methods::HOLD_GET => Ok(json!({"on": self.store.read(|data| data.hold)})),
            methods::HOLD_EDIT => {
                let on = params.get("on").and_then(Value::as_bool).ok_or_else(|| RpcError::invalid_params("`on` (true or false) is needed"))?;
                self.store.set_hold(on);
                self.changed("hold", "", "");
                self.changed("inbox", "", "");
                if !on {
                    // Jobs that stopped for the switch go on, and holds that ended meanwhile start.
                    job::tell_all(self, || job::Msg::Resume);
                    self.look_now();
                }
                Ok(json!({"on": on}))
            }
            methods::EVIDENCE_READ => self.evidence_read(&id_of(&params)?).await,
            methods::DISCOVER_READ | methods::DISCOVER_SEARCH | methods::DISCOVER_TREE => self.discover(method, parse_params(params)?).await,
            methods::RUNNER_RELEASE => self.runner_release(params).await,
            methods::RUNNER_GET => self.runner_get(params).await,
            other => Err(RpcError::unknown_method(other)),
        }
    }

    async fn repo_create(&self, params: RepoCreateParams) -> Result<Value, RpcError> {
        let name = params.name.trim().to_owned();
        if name.is_empty() {
            return Err(RpcError::invalid_params("a repository needs a name"));
        }
        let (ids, prefixes) = self.store.read(|data| {
            (data.repos.keys().cloned().collect::<Vec<_>>(), data.repos.values().map(|rec| rec.repo.prefix.clone()).collect::<Vec<_>>())
        });
        let base = repo_id(&name);
        if base.is_empty() {
            return Err(RpcError::invalid_params("a repository's name needs at least one letter or digit"));
        }
        let mut id = base.clone();
        let mut number = 2;
        while ids.contains(&id) {
            id = format!("{base}-{number}");
            number += 1;
        }
        let given: String = params.prefix.trim().chars().filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_uppercase()).collect();
        let prefix = if !given.is_empty() {
            if !(2..=3).contains(&given.len()) {
                return Err(RpcError::invalid_params("the prefix of request ids is two or three letters"));
            }
            if prefixes.contains(&given) {
                return Err(RpcError::invalid_params(format!("another repository already uses the prefix {given}")));
            }
            given
        } else {
            // No letters are taken from the name: the next free number, `R1`, `R2`, ...
            (1u32..).map(|number| format!("R{number}")).find(|candidate| !prefixes.contains(candidate)).unwrap_or_default()
        };
        let mut source = params.source;
        if source.kind.is_empty() {
            source.kind = "git".into();
        }
        if source.main.is_empty() {
            source.main = "main".into();
        }
        if source.changes.is_empty() {
            source.changes = "none".into();
        }
        let repo = Repo {
            id: id.clone(),
            name,
            prefix,
            software: params.software.trim().to_owned(),
            source,
            runner: params.runner.trim().to_owned(),
            runner_name: params.runner_name,
            policy: Policy::default(),
            managed: params.managed,
            created_ms: now_ms(),
        };
        let rec = RepoRec { repo: repo.clone(), project_id: String::new(), request_seq: 0 };
        self.store.put_repo(rec.clone());
        // The project is opened now when the frame allows it, and at the first request otherwise.
        if let Err(error) = self.project(&rec).await {
            tracing::info!(repo = %id, error = %error.message, "the repository's project is opened with its first request");
        }
        if !repo.runner.is_empty() && repo.managed {
            self.links.ensure(self.ctx()?, &repo.runner);
        }
        self.changed("repo", &id, &id);
        to_value(&repo)
    }

    fn repo_edit(&self, params: RepoEditParams) -> Result<Value, RpcError> {
        if let Some(name) = &params.name
            && name.trim().is_empty()
        {
            return Err(RpcError::invalid_params("a repository needs a name"));
        }
        let rec = self
            .store
            .update_repo(&params.id, |rec| {
                let repo = &mut rec.repo;
                if let Some(name) = &params.name {
                    repo.name = name.trim().to_owned();
                }
                if let Some(source) = &params.source {
                    repo.source = source.clone();
                }
                if let Some(runner) = &params.runner {
                    repo.runner = runner.trim().to_owned();
                }
                if let Some(runner_name) = &params.runner_name {
                    repo.runner_name = runner_name.clone();
                }
                if let Some(policy) = &params.policy {
                    repo.policy = policy.clone();
                }
                if let Some(managed) = params.managed {
                    repo.managed = managed;
                }
            })
            .ok_or_else(|| RpcError::not_found(format!("no repository `{}`", params.id)))?;
        if !rec.repo.runner.is_empty() && rec.repo.managed {
            self.links.ensure(self.ctx()?, &rec.repo.runner);
        }
        self.changed("repo", &params.id, &params.id);
        // A runner that was just set lets a waiting package start.
        self.look_now();
        to_value(&rec.repo)
    }

    async fn request_create(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        // A call through the mesh carries who asked (stamped by the mesh): that is who it is from.
        let caller: Option<Caller> = params.get("caller").filter(|caller| !caller.is_null()).and_then(|caller| serde_json::from_value(caller.clone()).ok());
        let params: RequestCreateParams = parse_params(params)?;
        let text = params.text.trim().to_owned();
        if text.is_empty() {
            return Err(RpcError::invalid_params("a request needs a text: what should change"));
        }
        if params.repo == "standard" {
            return Err(RpcError::invalid_params("the standard charter is changed through a standard request in a repository"));
        }
        let repo = self.repo(&params.repo)?;
        if !repo.repo.managed {
            return Err(RpcError::invalid_params(format!("{} is listed but left out for now; it takes no requests", repo.repo.name)));
        }
        let project_id = self.project(&repo).await?;
        let title = match params.title.trim() {
            "" => first_line(&text),
            title => title.to_owned(),
        };
        let session = self
            .host()?
            .session_create(&SessionCreateParams {
                project_id,
                kind: SessionKind::Chat,
                title: title.clone(),
                agent: None,
                parent: None,
                workspace: None,
                task_id: None,
                flow: Some(FLOW.into()),
            })
            .await?;
        let id = self.store.next_request_id(&repo.repo.id).ok_or_else(|| RpcError::not_found(format!("no repository `{}`", params.repo)))?;
        let from = match caller {
            Some(caller) => From { kind: "app".into(), name: if caller.name.is_empty() { caller.app } else { format!("{} on {}", caller.app, caller.name) }, node: caller.node },
            None if params.from.kind.is_empty() => From { kind: "person".into(), ..params.from },
            None => params.from,
        };
        let now = now_ms();
        let rec = ReqRec {
            r: Request {
                id: id.clone(),
                repo: repo.repo.id.clone(),
                session_id: session.id.clone(),
                title,
                text: text.clone(),
                from,
                stage: stage::ARRIVED.into(),
                created_ms: now,
                updated_ms: now,
                ..Request::default()
            },
            stages: vec![(stage::ARRIVED.into(), now)],
            urgent_person: params.urgent,
            title_auto: params.title.trim().is_empty(),
            ..ReqRec::default()
        };
        self.store.put_request(rec);
        self.entry(&session.id, fc::kinds::USER_MESSAGE, json!({"text": text})).await;
        self.changed("request", &id, &repo.repo.id);
        pipeline::spawn(self, &id);
        to_value(&self.request_view_of(&id)?)
    }

    fn request_list(&self, params: RequestListParams) -> Result<Value, RpcError> {
        let requests: Vec<RequestView> = self.store.read(|data| {
            data.requests
                .values()
                .filter(|rec| params.repo.as_ref().is_none_or(|repo| &rec.r.repo == repo))
                .filter(|rec| params.stage.as_ref().is_none_or(|stage| &rec.r.stage == stage))
                .filter(|rec| params.with_ended || rec.r.ended_as.is_empty())
                .map(|rec| request_view(data, rec))
                .collect()
        });
        Ok(json!({"requests": requests}))
    }

    async fn request_edit(self: &Arc<Self>, params: RequestEditParams) -> Result<Value, RpcError> {
        let rec = self.request(&params.id)?;
        if !rec.r.ended_as.is_empty() {
            return Err(RpcError::new(codes::CONFLICT, format!("{} is closed ({})", rec.r.id, rec.r.ended_as)));
        }
        let building = matches!(rec.r.stage.as_str(), stage::BUILDING | stage::PROVING);
        let landed = matches!(rec.r.stage.as_str(), stage::LANDED | stage::SOAKING | stage::RELEASED);
        if params.withdraw {
            if landed {
                return Err(RpcError::new(codes::CONFLICT, self.t("withdraw_landed", &[])));
            }
            if building {
                let alone = self.store.read(|data| data.packages.get(&rec.r.package).is_none_or(|package| package.requests.len() <= 1));
                if !alone {
                    return Err(RpcError::new(codes::CONFLICT, self.t("withdraw_bundled", &[])));
                }
                if let Some(package) = self.store.update_package(&rec.r.package, |package| package.stage = ended::WITHDRAWN.into()) {
                    job::cancel(self, &package);
                }
            } else {
                packages::take_out(self, &rec.r.id);
            }
            self.next_run(&rec.r.id);
            inbox::close_where(self, "policy", "withdrawn", |ask| ask.ask.request == rec.r.id || ask.other_request == rec.r.id || (building && ask.ask.package == rec.r.package));
            self.store.update_request(&rec.r.id, |rec| {
                rec.r.ended_as = ended::WITHDRAWN.into();
                rec.r.ended_reason = self.t("withdrawn_reason", &[]);
            });
            self.entry(&rec.r.session_id, cc_protocol::contracts::manage::kinds::STAGE, json!({"stage": ended::WITHDRAWN, "text": self.t("stage_withdrawn", &[])})).await;
            self.changed("request", &rec.r.id, &rec.r.repo);
            self.changed("inbox", "", "");
            return to_value(&self.request_view_of(&params.id)?);
        }
        if let Some(urgent) = params.urgent {
            let updated = self.store.update_request(&rec.r.id, |rec| {
                rec.urgent_person = Some(urgent);
                pipeline::settle_urgent(rec);
            });
            // A held request changes where it belongs: an urgent one is never bundled.
            if let Some(updated) = updated
                && updated.r.stage == stage::HELD
                && updated.r.urgent != rec.r.urgent
                && packages::take_out(self, &rec.r.id)
            {
                self.store.set_stage(&rec.r.id, stage::VALIDATED);
                pipeline::spawn(self, &rec.r.id);
            }
            self.changed("request", &rec.r.id, &rec.r.repo);
        }
        if !params.answers.is_empty() {
            if rec.r.stage != stage::CLARIFYING {
                return Err(RpcError::new(codes::CONFLICT, format!("{} has no open questions", rec.r.id)));
            }
            let updated = self.store.update_request(&rec.r.id, |rec| {
                for (id, answer) in &params.answers {
                    if let Some(question) = rec.r.questions.iter_mut().find(|question| &question.id == id) {
                        question.answer = answer.clone();
                    }
                }
            });
            let answered = updated.as_ref().is_some_and(|rec| rec.r.questions.iter().all(|question| !question.answer.is_empty()));
            if answered && let Some(updated) = updated {
                let shown: Vec<String> = updated.r.questions.iter().map(|question| format!("{} {}", question.text, question.answer.join(", "))).collect();
                self.entry(&rec.r.session_id, fc::kinds::USER_MESSAGE, json!({"text": shown.join("\n")})).await;
                pipeline::answered(self, &updated).await;
            }
            self.changed("request", &rec.r.id, &rec.r.repo);
        }
        if let Some(text) = params.add_text.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            if building || landed {
                return Err(RpcError::new(codes::CONFLICT, self.t("add_text_late", &[])));
            }
            self.entry(&rec.r.session_id, fc::kinds::USER_MESSAGE, json!({"text": text})).await;
            pipeline::again(self, &rec.r.id, text, true);
        }
        to_value(&self.request_view_of(&params.id)?)
    }

    /// The person wrote in a request's thread.
    async fn request_say(self: &Arc<Self>, params: SayParams) -> Result<Value, RpcError> {
        let rec = self.request(&params.request)?;
        let text = params.text.trim().to_owned();
        if text.is_empty() {
            return Err(RpcError::invalid_params("nothing was said"));
        }
        // The chat flow's turn wrote the person's message into the thread before it called this.
        let answer = if !rec.r.ended_as.is_empty() {
            self.t("say_closed", &[("as", &rec.r.ended_as)])
        } else {
            match rec.r.stage.as_str() {
                stage::BUILDING | stage::PROVING => {
                    // While it is built the text goes to the builder as it was written.
                    let package = self.package(&rec.r.package)?;
                    let node = self.repo(&rec.r.repo)?.repo.runner;
                    let steer = runner::SteerParams { job_id: package.job_id.clone(), attempt: package.attempt, text: text.clone() };
                    match self.runner::<Value>(&node, runner::methods::STEER, json!(steer), RUNNER_CALL).await {
                        Ok(_) => self.t("say_steered", &[]),
                        Err(error) => self.t("say_not_steered", &[("why", &error.message)]),
                    }
                }
                stage::LANDED | stage::SOAKING | stage::RELEASED => self.t("say_landed", &[]),
                _ => {
                    pipeline::again(self, &rec.r.id, &text, true);
                    self.t("say_again", &[])
                }
            }
        };
        // The chat flow shows the answer in the thread: the model's own, or this line.
        Ok(json!({"text": answer}))
    }

    async fn evidence_read(&self, id: &str) -> Result<Value, RpcError> {
        let kind = self
            .store
            .read(|data| data.evidence.values().flatten().find(|evidence| evidence.id == id).map(|evidence| evidence.kind.clone()))
            .ok_or_else(|| RpcError::not_found(format!("no evidence `{id}`")))?;
        // Ids are the plugin's own (`ev-00042`), so the name is a file of the evidence folder.
        let path = self.data_dir.join("evidence").join(format!("{id}.log"));
        let bytes = tokio::fs::read(&path).await.map_err(|_| RpcError::not_found(format!("evidence `{id}` left no bytes behind")))?;
        Ok(json!({"kind": kind, "mime": "text/plain; charset=utf-8", "base64": base64(&bytes)}))
    }

    async fn discover(&self, method: &str, params: DiscoverParams) -> Result<Value, RpcError> {
        let repo = self.repo(&params.repo)?.repo;
        let node = self.runner_of(&repo)?;
        let (method, params) = match method {
            methods::DISCOVER_READ => (runner::methods::READ, json!(runner::ReadParams { repo: repo.id.clone(), path: params.path, from: params.from, lines: params.lines })),
            methods::DISCOVER_SEARCH => (runner::methods::SEARCH, json!(runner::SearchParams { repo: repo.id.clone(), text: params.text, under: params.under, limit: params.limit })),
            _ => (runner::methods::TREE, json!({"repo": repo.id, "path": params.path})),
        };
        self.runner(&node, method, params, RUNNER_CALL).await
    }

    async fn runner_release(&self, params: Value) -> Result<Value, RpcError> {
        let repo = self.repo(params.get("repo").and_then(Value::as_str).unwrap_or_default())?.repo;
        let node = self.runner_of(&repo)?;
        let plan_only = params.get("plan_only").and_then(Value::as_bool).unwrap_or(false);
        let release = runner::ReleaseParams { repo: repo.id.clone(), url: repo.source.url.clone(), main: repo.source.main.clone(), plan_only };
        let answer: runner::ReleaseAnswer = self.runner(&node, runner::methods::RELEASE, json!(release), RUNNER_BUILD).await?;
        to_value(&answer)
    }

    async fn runner_get(&self, params: Value) -> Result<Value, RpcError> {
        let repo = self.repo(params.get("repo").and_then(Value::as_str).unwrap_or_default())?.repo;
        if repo.runner.is_empty() {
            return Ok(json!({"reachable": false, "node": "", "name": "", "jobs": []}));
        }
        #[derive(serde::Deserialize)]
        struct Jobs {
            #[serde(default)]
            jobs: Vec<Facts>,
        }
        match self.runner::<Jobs>(&repo.runner, runner::methods::LIST, json!({}), Duration::from_secs(10)).await {
            Ok(list) => Ok(json!({"reachable": true, "node": repo.runner, "name": repo.runner_name, "jobs": list.jobs})),
            Err(error) => Ok(json!({"reachable": false, "node": repo.runner, "name": repo.runner_name, "jobs": [], "error": error.message})),
        }
    }

    // ---- probe ---------------------------------------------------------------------------------

    /// Real work without a runner or a model: the policy's floor, the ids, the texts, and one
    /// record written to the graph, read back and retired.
    pub async fn probe(&self) -> ProbeReport {
        let mut checks = Vec::new();
        let mut check = |name: &str, ok: bool, detail: String, started: std::time::Instant| {
            checks.push(ProbeCheck { name: name.into(), ok, detail, ms: started.elapsed().as_millis() as u64 });
        };

        let started = std::time::Instant::now();
        let globs = |paths: &[&str]| paths.iter().map(|glob| (*glob).to_owned()).collect::<Vec<_>>();
        let policy = Policy {
            major_paths: globs(&["protocol/**", "frame/**", "**/pairing*", "AGENTS.md"]),
            minor_paths: globs(&["plugins/*/ui/**", "plugins/*/backend/data/**", "**/texts.toml"]),
            ..Policy::default()
        };
        let point = |touches: &[&str], lines: u32| Point { n: 1, title: "probe".into(), touches: touches.iter().map(|touch| (*touch).to_owned()).collect(), limit_lines: lines, ..Point::default() };
        let cases: [(&str, Vec<Point>, Vec<String>, Tier); 7] = [
            ("a plugin's backend source is safe", vec![point(&["plugins/mesh/backend/src/net.rs"], 40)], vec![], Tier::Safe),
            ("a plugin's UI is minor", vec![point(&["plugins/updater/ui/band/src/index.tsx"], 40)], vec![], Tier::Minor),
            ("a texts file is minor", vec![point(&["plugins/scheduler/backend/data/texts.toml"], 10)], vec![], Tier::Minor),
            ("the protocol is major", vec![point(&["protocol/src/contracts/mesh.rs"], 40)], vec![], Tier::Major),
            ("a pairing file is major wherever it is", vec![point(&["plugins/mesh/backend/src/pairing.rs"], 40)], vec![], Tier::Major),
            ("a standard path is major", vec![point(&["plugins/theme-cortex/ui/theme/src/x.css"], 10)], vec!["plugins/theme-cortex/ui/theme/src/x.css".into()], Tier::Major),
            ("more than 2500 lines is major", vec![point(&["plugins/mesh/backend/src/a.rs"], 1500), point(&["plugins/mesh/backend/src/b.rs"], 1200)], vec![], Tier::Major),
        ];
        let wrong: Vec<String> = cases
            .iter()
            .filter_map(|(name, points, standard, want)| {
                let got = policy::floor(&policy, points, standard, &self.texts);
                (got.tier != *want || got.reasons.is_empty()).then(|| format!("{name}: got {}", got.tier.as_str()))
            })
            .collect();
        let many: Vec<Point> = (0..13).map(|n| Point { n, ..point(&["plugins/mesh/backend/src/a.rs"], 10) }).collect();
        let many_ok = policy::floor(&policy, &many, &[], &self.texts).tier == Tier::Major;
        check("policy.floor", wrong.is_empty() && many_ok, if wrong.is_empty() { format!("{} cases", cases.len() + 1) } else { wrong.join("; ") }, started);

        let started = std::time::Instant::now();
        let hold_ok = policy::hold_until(&policy, Tier::Safe, false, 1_000, 1_000) == 1_000 + 120 * 60_000
            && policy::hold_until(&policy, Tier::Minor, false, 1_000 + 23 * 3_600_000, 1_000) == 1_000 + 24 * 3_600_000
            && policy::hold_until(&policy, Tier::Major, false, 1_000, 1_000) == 0
            && policy::hold_until(&policy, Tier::Minor, true, 5_000, 1_000) == 5_000;
        check("policy.hold", hold_ok, "safe by the clock, capped at the longest hold, major without a clock, urgent at once".into(), started);

        let started = std::time::Instant::now();
        let ids_ok = crate::store::request_id("CT", 142) == "CT-0142" && repo_id("Cortex Term") == "cortexterm";
        let version = env!("CARGO_PKG_VERSION");
        check("ids", ids_ok && !version.is_empty(), format!("version {version}, request ids like CT-0142"), started);

        let started = std::time::Instant::now();
        let steer = self.t("steer_w4", &[("file", "a/b.rs"), ("next", "Probe")]);
        let texts_ok = steer.contains("a/b.rs") && steer.contains("Probe") && !self.t("count_held", &[("count", "1")]).is_empty();
        check("texts", texts_ok, "the steer sentences fill in their facts".into(), started);

        let started = std::time::Instant::now();
        let (ok, detail) = match self.probe_graph().await {
            Ok(detail) => (true, detail),
            Err(error) => (false, format!("{}: {}", error.code, error.message)),
        };
        check("graph.round_trip", ok, detail, started);
        ProbeReport::from_checks(checks)
    }

    /// Put a repository record under a probe-only id, read it back, retire it.
    async fn probe_graph(&self) -> Result<String, RpcError> {
        let ctx = self.ctx()?;
        let call = |method: &'static str, params: Value| async move {
            // The memory plugin may still be starting: a refusal is tried again for a short while.
            let started = std::time::Instant::now();
            loop {
                match ctx.call(method, params.clone()).await {
                    Err(error) if cc_protocol::host::refused(&error) && started.elapsed() < Duration::from_secs(20) => tokio::time::sleep(Duration::from_millis(500)).await,
                    outcome => return outcome,
                }
            }
        };
        let key = format!("probe-{}", uuid::Uuid::new_v4().simple());
        let machine: MachineInfo = parse_params(call(memory_methods::GRAPH_MACHINE, json!({})).await?)?;
        let node = graph::node("Repo", &key, graph::props([("of", json!("manage-probe")), ("name", json!("probe")), ("major_paths", json!(["protocol/**"])), ("hold_safe_min", json!(120))]));
        let edge = graph::edge(("Repo", &key), "ON", graph::at("Machine", &machine.key));
        call(memory_methods::GRAPH_PUT, json!(GraphPutParams { nodes: vec![node], edges: vec![edge], layer: GraphLayer::Workspace, at_ms: None })).await?;
        let query = GraphQueryParams { cypher: "MATCH (n:Repo) WHERE n.key = $key RETURN n".into(), params: graph::props([("key", json!(key))]), limit: Some(1) };
        let found: GraphQueryResult = parse_params(call(memory_methods::GRAPH_QUERY, json!(query)).await?)?;
        let props = found.rows.first().and_then(|row| row.first()).and_then(|node| node.get("properties")).cloned().unwrap_or(Value::Null);
        let read_ok = props.get("name").and_then(Value::as_str) == Some("probe")
            && props.get("hold_safe_min").and_then(Value::as_u64) == Some(120)
            && props.get("major_paths").and_then(Value::as_array).map(Vec::len) == Some(1);
        call(memory_methods::GRAPH_RETIRE, json!(GraphRetireParams { refs: vec![graph::at("Repo", &key)], reason: Some("probe".into()), at_ms: None })).await?;
        if !read_ok {
            return Err(RpcError::new(codes::CONTRACT_VIOLATION, format!("the record did not come back as it was written: {props}")));
        }
        Ok("a record was written, read back and retired".into())
    }
}

fn id_of(params: &Value) -> Result<String, RpcError> {
    params.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned).ok_or_else(|| RpcError::invalid_params("`id` is needed"))
}

/// `Cortex Term` -> `cortexterm`.
pub fn repo_id(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// A title from a text: its first line, cut at a word.
fn first_line(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().trim();
    if line.chars().count() <= 72 {
        return line.to_owned();
    }
    let cut: String = line.chars().take(72).collect();
    match cut.rsplit_once(' ') {
        Some((head, _)) if head.chars().count() > 30 => format!("{head}…"),
        _ => format!("{cut}…"),
    }
}

pub fn request_view(data: &Data, rec: &ReqRec) -> RequestView {
    RequestView { request: rec.r.clone(), word: words::request(data, rec), plan: data.plan(&rec.r.id).map(|plan| plan.plan.clone()), stages: rec.stages.clone() }
}

pub fn package_view(data: &Data, package: &Package) -> PackageView {
    let policy = data.repos.get(&package.repo).map(|rec| rec.repo.policy.clone()).unwrap_or_default();
    PackageView {
        package: package.clone(),
        word: words::package(data, package),
        points: packages::joined(data, package).into_iter().map(|joined| joined.point).collect(),
        gates: data.gates.get(&package.id).cloned().unwrap_or_default(),
        interventions: data.interventions.get(&package.id).map(|list| list.iter().map(|rec| rec.i.clone()).collect()).unwrap_or_default(),
        evidence: data.evidence.get(&package.id).cloned().unwrap_or_default(),
        strip: data.strips.get(&package.id).cloned(),
        digest: data.digests.get(&package.id).cloned().unwrap_or_default(),
        budget: policy::budget(&policy, package.tier),
    }
}

/// What the plugin shares with the other apps of the mesh: making a request and looking at
/// requests. For apps' own code, never a model tool.
pub fn shares() -> ShareList {
    let method = |name: &str, description: &str, params: Value| SharedMethod { method: name.into(), description: description.into(), params };
    let share = Share {
        service: SERVICE.into(),
        version: 1,
        title: "Change requests".into(),
        description: "Ask CortexManage for a change to one of the repositories it manages, and see how a request is doing.".into(),
        methods: vec![
            method(
                methods::REQUEST_CREATE,
                "Open a request: one thing that should change in a repository. It is assessed, planned, held and built by CortexManage. Answers the request with its id.",
                json!({
                    "type": "object",
                    "properties": {
                        "repo": {"type": "string", "description": "The repository's id, as `manage.request.list` shows it (for example `cortexterm`)."},
                        "text": {"type": "string", "description": "What should change, in plain words; for a fault: what happens now and what should happen."},
                        "title": {"type": "string", "description": "A short title (made from the text when absent)."}
                    },
                    "required": ["repo", "text"]
                }),
            ),
            method(
                methods::REQUEST_GET,
                "One request with its stage, tier and the word that says how it is doing.",
                json!({"type": "object", "properties": {"id": {"type": "string", "description": "The request's id (for example `CT-0142`)."}}, "required": ["id"]}),
            ),
            method(
                methods::REQUEST_LIST,
                "The open requests, of one repository or of all.",
                json!({
                    "type": "object",
                    "properties": {
                        "repo": {"type": "string", "description": "Only this repository's."},
                        "stage": {"type": "string", "description": "Only the ones at this stage (arrived, held, building, landed ...)."},
                        "with_ended": {"type": "boolean", "description": "Also the ones that ended."}
                    }
                }),
            ),
        ],
        audience: Audience::Apps,
    };
    ShareList { shares: vec![share] }
}

/// Standard base64, for the bytes of a piece of evidence.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// The title of a session, set again when the assessment gave a better one.
pub async fn retitle(manager: &Manager, session: &str, title: &str) {
    let Ok(host) = manager.host() else { return };
    let update = SessionUpdateParams { id: session.to_owned(), title: Some(title.to_owned()), status: None, task_id: None, model: None };
    if let Err(error) = host.session_update(&update).await {
        tracing::debug!(session, error = %error.message, "the session's title was not changed");
    }
}
