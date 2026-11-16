//! The standard's service: the members, the lock, the last check, the handshake.
//!
//! The record is the graph (WORKSPACE layer, written in order through [`Graph`]):
//!
//! - `(:StandardMember {key: id, name, priority, without, deferred, root, names})-[:ON]->(:Machine)`
//! - `(:StandardFolder {key: "<member>/<part>", member, part, folder})-[:OF]->(:StandardMember)`
//! - `(:StandardLock {key: "current", version, from, written_ms, files})-[:ON]->(:Machine)`
//! - `(:StandardFile {key: "<part>/<path>", part, path, sha256, names})-[:IN]->(:StandardLock)`
//! - `(:StandardCheck {key: "last", version, checked_ms, parts, drift})-[:ON]->(:Machine)`
//! - `(:StandardCell {key: "<member>/<part>", member, part, state, files, count})-[:IN]->(:StandardCheck)`
//!
//! What is read back at the start is kept in memory; every method answers from there. File reads
//! (hashing a checkout) run in `spawn_blocking`.

use crate::graph::{self, Graph};
use crate::lock;
use crate::parts::{Part, Parts, Skip, Source};
use cc_protocol::contracts::lifecycle::{ProbeCheck, ProbeReport};
use cc_protocol::contracts::mesh::{self, CallParams, Card, Peer, PeersResult};
use cc_protocol::contracts::standard::{Cell, DiffParams, Handshake, HandshakeRow, Lock, LockFile, LockWriteParams, Member, Report, methods, topics};
use cc_protocol::{Ctx, RpcError, codes, parse_params, to_value};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const MEMBER: &str = "StandardMember";
const FOLDER: &str = "StandardFolder";
const LOCK: &str = "StandardLock";
const FILE: &str = "StandardFile";
const CHECK: &str = "StandardCheck";
const CELL: &str = "StandardCell";
/// The one lock and the one last check.
const CURRENT: &str = "current";
const LAST: &str = "last";
/// Nodes per put (each goes with one edge; the memory plugin takes 5000 items a put).
const BATCH: usize = 2000;
/// The lock's own file in every repository: written by CortexManage only.
const LOCK_FILE: &str = "standard.lock";
/// How long one member has to answer the handshake.
const ANSWER: Duration = Duration::from_secs(20);

#[derive(Default)]
struct State {
    /// By priority, the highest first.
    members: Vec<Member>,
    lock: Lock,
    report: Option<Report>,
}

pub struct Standard {
    data_dir: PathBuf,
    parts: Source,
    graph: Graph,
    ctx: OnceLock<Ctx>,
    state: Mutex<State>,
    /// True once what the graph keeps was read back (or could not be, and the defaults stand in).
    loaded: watch::Sender<bool>,
}

fn now_ms() -> u64 {
    cc_protocol::contracts::now_ms()
}

/// A list property: absent when empty (an empty list has no type to keep).
fn list(items: &[String]) -> Value {
    if items.is_empty() { Value::Null } else { json!(items) }
}

fn sort(members: &mut [Member]) {
    members.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
}

/// The folders of an app built on the frame.
fn framed() -> BTreeMap<String, String> {
    [
        ("protocol", "protocol"),
        ("mesh", "plugins/mesh/backend/src"),
        ("service-kit", "service-kit"),
        ("sandbox-kit", "sandbox-kit"),
        ("model-kit", "model-kit"),
        ("shared-models", "shared-models"),
        ("memory", "plugins/memory-irongraph/backend/src"),
        ("theme", "plugins/theme-cortex/ui/theme"),
    ]
    .into_iter()
    .map(|(part, folder)| (part.to_owned(), folder.to_owned()))
    .collect()
}

/// The members of the standard as docs/specs/04-STANDARD.md §1 lists them, for the first start.
/// A checkout is named only when its folder is on this computer (`~/DEV/<name>`); any other
/// folder, and a member without a frame, are set in the Standard view. Reads the disk.
fn defaults() -> Vec<Member> {
    let home = std::env::var("HOME").unwrap_or_default();
    let here = |name: &str| {
        let root = Path::new(&home).join("DEV").join(name);
        if !home.is_empty() && root.is_dir() { root.to_string_lossy().into_owned() } else { String::new() }
    };
    let member = |id: &str, name: &str, priority: u32| Member {
        id: id.to_owned(),
        name: name.to_owned(),
        priority,
        folders: framed(),
        without: Vec::new(),
        deferred: false,
        root: here(name),
        names: vec![name.to_owned(), id.to_owned()],
    };
    // CortexAgent is left out for now (the owner's decision of 2026-11-16): listed, never checked.
    let mut agent = member("cortexagent", "CortexAgent", 20);
    agent.root = String::new();
    agent.deferred = true;
    vec![
        member("cortexmodelserver", "CortexModelServer", 50),
        member("cortexcode", "CortexCode", 30),
        agent,
        member("cortexterm", "CortexTerm", 10),
        member("cortexmanage", "CortexManage", 5),
    ]
}

/// A blocking piece of work (file reads) off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, RpcError> {
    tokio::task::spawn_blocking(work).await.map_err(|error| RpcError::internal(format!("the file work stopped: {error}")))
}

fn folder_key(member: &str, part: &str) -> String {
    format!("{member}/{part}")
}

fn file_key(file: &LockFile) -> String {
    format!("{}/{}", file.part, file.path)
}

impl Standard {
    pub fn new(data_dir: PathBuf, live_dir: Option<PathBuf>) -> Arc<Standard> {
        Arc::new(Standard {
            data_dir,
            parts: Source::new(live_dir),
            graph: Graph::new(),
            ctx: OnceLock::new(),
            state: Mutex::new(State::default()),
            loaded: watch::channel(false).0,
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ctx(&self) -> Result<Ctx, RpcError> {
        self.ctx.get().cloned().ok_or_else(|| RpcError::new(codes::UNAVAILABLE, "not connected to the frame yet"))
    }

    /// The context arrives with the first request; then what the graph keeps is read back.
    pub fn attach(self: &Arc<Self>, ctx: &Ctx) {
        if self.ctx.set(ctx.clone()).is_ok() {
            self.graph.start(graph::ctx_caller(ctx));
            let standard = self.clone();
            tokio::spawn(async move {
                standard.load().await;
                standard.loaded.send_replace(true);
            });
        }
    }

    /// Wait until the records were read back, so no method answers from an empty memory.
    async fn ready(&self) {
        let mut loaded = self.loaded.subscribe();
        let _ = loaded.wait_for(|loaded| *loaded).await;
    }

    async fn active(&self, label: &str) -> Result<Vec<Map<String, Value>>, RpcError> {
        self.graph.nodes(&format!("MATCH (n:{label}) WHERE n.ended_ms IS NULL RETURN n"), Map::new(), 5000).await
    }

    /// Read the members, the lock and the last check back. With no member kept, this is the first
    /// start: the defaults are written. When the graph cannot be read, the defaults stand in for
    /// this run and nothing is written over what the graph may hold.
    async fn load(&self) {
        let kept = match self.active(MEMBER).await {
            Ok(kept) => kept,
            Err(error) => {
                tracing::warn!(error = %error.message, "the standard's members were not read from the graph; the defaults are used for this run");
                let mut members = blocking(defaults).await.unwrap_or_default();
                sort(&mut members);
                self.state().members = members;
                return;
            }
        };
        let mut members: Vec<Member> = if kept.is_empty() {
            let members = blocking(defaults).await.unwrap_or_default();
            for member in &members {
                self.put_member(member, None);
            }
            tracing::info!(members = members.len(), "the standard's members were written for the first time");
            members
        } else {
            let mut folders: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
            match self.active(FOLDER).await {
                Ok(nodes) => {
                    for props in nodes {
                        folders.entry(graph::text(&props, "member")).or_default().insert(graph::text(&props, "part"), graph::text(&props, "folder"));
                    }
                }
                Err(error) => tracing::warn!(error = %error.message, "the members' folders were not read from the graph"),
            }
            kept.iter()
                .map(|props| {
                    let id = graph::text(props, "key");
                    Member {
                        name: graph::text(props, "name"),
                        priority: graph::number(props, "priority").unwrap_or_default() as u32,
                        folders: folders.remove(&id).unwrap_or_default(),
                        without: graph::texts(props, "without"),
                        deferred: graph::flag(props, "deferred"),
                        root: graph::text(props, "root"),
                        names: graph::texts(props, "names"),
                        id,
                    }
                })
                .filter(|member| !member.id.is_empty())
                .collect()
        };
        sort(&mut members);

        let parts = self.read_parts().await;
        let mut kept_lock = Lock::default();
        match self.active(LOCK).await {
            Ok(nodes) => {
                if let Some(props) = nodes.iter().find(|props| graph::text(props, "key") == CURRENT) {
                    kept_lock.version = graph::text(props, "version");
                    kept_lock.from = graph::text(props, "from");
                    kept_lock.written_ms = graph::number(props, "written_ms").unwrap_or_default();
                }
            }
            Err(error) => tracing::warn!(error = %error.message, "the lock was not read from the graph"),
        }
        if !kept_lock.version.is_empty() {
            match self.active(FILE).await {
                Ok(nodes) => {
                    kept_lock.files = nodes
                        .iter()
                        .map(|props| LockFile { part: graph::text(props, "part"), path: graph::text(props, "path"), sha256: graph::text(props, "sha256"), names: graph::flag(props, "names") })
                        .filter(|file| !file.part.is_empty() && !file.path.is_empty())
                        .collect();
                    kept_lock.files.sort_by(|a, b| parts.order(&a.part).cmp(&parts.order(&b.part)).then(a.part.cmp(&b.part)).then(a.path.cmp(&b.path)));
                }
                Err(error) => tracing::warn!(error = %error.message, "the lock's files were not read from the graph"),
            }
        }

        let mut report = None;
        match self.active(CHECK).await {
            Ok(nodes) => {
                if let Some(props) = nodes.iter().find(|props| graph::text(props, "key") == LAST) {
                    let cells = match self.active(CELL).await {
                        Ok(cells) => cells,
                        Err(error) => {
                            tracing::warn!(error = %error.message, "the last check's cells were not read from the graph");
                            Vec::new()
                        }
                    };
                    let ids = graph::texts(props, "parts");
                    let place = |cell: &Cell| (members.iter().position(|member| member.id == cell.member).unwrap_or(usize::MAX), ids.iter().position(|part| *part == cell.part).unwrap_or(usize::MAX));
                    let mut cells: Vec<Cell> = cells
                        .iter()
                        .map(|cell| Cell {
                            member: graph::text(cell, "member"),
                            part: graph::text(cell, "part"),
                            state: graph::text(cell, "state"),
                            files: graph::texts(cell, "files"),
                            count: graph::number(cell, "count").unwrap_or_default() as u32,
                        })
                        .collect();
                    cells.sort_by_key(|cell| place(cell));
                    report = Some(Report {
                        version: graph::text(props, "version"),
                        parts: ids.clone(),
                        // The members as they are now: the check keeps what it found, not a copy of them.
                        members: members.clone(),
                        cells,
                        drift: graph::texts(props, "drift"),
                        checked_ms: graph::number(props, "checked_ms").unwrap_or_default(),
                    });
                }
            }
            Err(error) => tracing::warn!(error = %error.message, "the last check was not read from the graph"),
        }

        let mut state = self.state();
        state.members = members;
        state.lock = kept_lock;
        state.report = report;
    }

    async fn read_parts(&self) -> Parts {
        let source = self.parts.clone();
        blocking(move || source.read()).await.unwrap_or_default()
    }

    /// Write a member and its folders; the folders `before` had and it no longer has are retired.
    fn put_member(&self, member: &Member, before: Option<&Member>) {
        let props = graph::props([
            ("name", json!(member.name)),
            ("priority", json!(member.priority)),
            ("without", list(&member.without)),
            ("deferred", json!(member.deferred)),
            ("root", json!(member.root)),
            ("names", list(&member.names)),
        ]);
        let mut nodes = vec![graph::node(MEMBER, &member.id, props)];
        let mut edges = vec![graph::edge((MEMBER, &member.id), "ON", graph::machine())];
        for (part, folder) in &member.folders {
            let key = folder_key(&member.id, part);
            nodes.push(graph::node(FOLDER, &key, graph::props([("member", json!(member.id)), ("part", json!(part)), ("folder", json!(folder))])));
            edges.push(graph::edge((FOLDER, &key), "OF", graph::at(MEMBER, &member.id)));
        }
        self.graph.put(nodes, edges);
        if let Some(before) = before {
            let gone = before.folders.keys().filter(|part| !member.folders.contains_key(*part)).map(|part| graph::at(FOLDER, &folder_key(&member.id, part))).collect();
            self.graph.retire(gone, "no longer the member's folder");
        }
    }

    /// Write the lock; the files `before` had and it no longer has are retired.
    fn put_lock(&self, lock: &Lock, before: &Lock) {
        let head = || {
            let props = graph::props([("version", json!(lock.version)), ("from", json!(lock.from)), ("written_ms", json!(lock.written_ms)), ("files", json!(lock.files.len()))]);
            (graph::node(LOCK, CURRENT, props), graph::edge((LOCK, CURRENT), "ON", graph::machine()))
        };
        // Every batch carries the lock's own node, so its files always hang from something.
        for batch in lock.files.chunks(BATCH).chain(lock.files.is_empty().then_some(&[][..])) {
            let (node, edge) = head();
            let (mut nodes, mut edges) = (vec![node], vec![edge]);
            for file in batch {
                let key = file_key(file);
                nodes.push(graph::node(FILE, &key, graph::props([("part", json!(file.part)), ("path", json!(file.path)), ("sha256", json!(file.sha256)), ("names", json!(file.names))])));
                edges.push(graph::edge((FILE, &key), "IN", graph::at(LOCK, CURRENT)));
            }
            self.graph.put(nodes, edges);
        }
        let kept: BTreeSet<String> = lock.files.iter().map(file_key).collect();
        let gone: Vec<_> = before.files.iter().map(file_key).filter(|key| !kept.contains(key)).map(|key| graph::at(FILE, &key)).collect();
        for batch in gone.chunks(BATCH) {
            self.graph.retire(batch.to_vec(), "not in the new lock");
        }
    }

    /// Write a check: one node for it, one per cell; cells `before` had and it has not are retired.
    fn put_report(&self, report: &Report, before: Option<&Report>) {
        let head = || {
            let props = graph::props([("version", json!(report.version)), ("checked_ms", json!(report.checked_ms)), ("parts", list(&report.parts)), ("drift", list(&report.drift))]);
            (graph::node(CHECK, LAST, props), graph::edge((CHECK, LAST), "ON", graph::machine()))
        };
        for batch in report.cells.chunks(BATCH).chain(report.cells.is_empty().then_some(&[][..])) {
            let (node, edge) = head();
            let (mut nodes, mut edges) = (vec![node], vec![edge]);
            for cell in batch {
                let key = folder_key(&cell.member, &cell.part);
                let props = graph::props([("member", json!(cell.member)), ("part", json!(cell.part)), ("state", json!(cell.state)), ("files", list(&cell.files)), ("count", json!(cell.count))]);
                nodes.push(graph::node(CELL, &key, props));
                edges.push(graph::edge((CELL, &key), "IN", graph::at(CHECK, LAST)));
            }
            self.graph.put(nodes, edges);
        }
        if let Some(before) = before {
            let kept: BTreeSet<String> = report.cells.iter().map(|cell| folder_key(&cell.member, &cell.part)).collect();
            let gone: Vec<_> = before.cells.iter().map(|cell| folder_key(&cell.member, &cell.part)).filter(|key| !kept.contains(key)).map(|key| graph::at(CELL, &key)).collect();
            for batch in gone.chunks(BATCH) {
                self.graph.retire(batch.to_vec(), "not in the newer check");
            }
        }
    }

    fn changed(&self, version: &str, drift: &[String]) {
        if let Some(ctx) = self.ctx.get() {
            ctx.publish(topics::CHANGED, json!({"version": version, "drift": drift}));
        }
    }

    pub async fn handle(self: &Arc<Self>, method: &str, params: Value) -> Result<Value, RpcError> {
        self.ready().await;
        match method {
            methods::LOCK_GET => to_value(&self.state().lock),
            methods::LOCK_WRITE => to_value(&self.write_lock(parse_params(params)?).await?),
            methods::LOCK_PRINT => {
                let text = {
                    let state = self.state();
                    lock::print(&state.lock, &state.members)
                };
                Ok(json!({"text": text}))
            }
            methods::MEMBER_LIST => Ok(json!({"members": self.state().members})),
            methods::MEMBER_EDIT => {
                self.edit(parse_params(params)?).await?;
                Ok(json!({"members": self.state().members}))
            }
            methods::CHECK => to_value(&self.check().await?),
            methods::GET => to_value(&self.last()),
            methods::TOUCHES => {
                let params: DiffParams = parse_params(params)?;
                Ok(json!({"paths": self.touches(params).await?}))
            }
            methods::PATHS => {
                let member = params.get("member").and_then(Value::as_str).unwrap_or_default().to_owned();
                Ok(json!({"paths": self.paths(&member).await?}))
            }
            methods::HANDSHAKE => to_value(&self.handshake().await?),
            other => Err(RpcError::unknown_method(other)),
        }
    }

    fn member(&self, id: &str) -> Result<Member, RpcError> {
        self.state().members.iter().find(|member| member.id == id).cloned().ok_or_else(|| RpcError::not_found(format!("`{id}` is not a member of the standard")))
    }

    /// The last report; before the first check, the members with nothing compared yet.
    fn last(&self) -> Report {
        let state = self.state();
        state.report.clone().unwrap_or_else(|| Report { version: state.lock.version.clone(), members: state.members.clone(), ..Report::default() })
    }

    /// Hash every standard file of every member with a checkout here against the lock.
    pub async fn check(&self) -> Result<Report, RpcError> {
        let (members, kept) = {
            let state = self.state();
            (state.members.clone(), state.lock.clone())
        };
        let source = self.parts.clone();
        let report = blocking(move || lock::check(&members, &kept, &source.read(), now_ms())).await?;
        let before = self.state().report.replace(report.clone());
        self.put_report(&report, before.as_ref());
        self.changed(&report.version, &report.drift);
        Ok(report)
    }

    /// Make the lock from one member's copies, then check every member against it.
    async fn write_lock(&self, params: LockWriteParams) -> Result<Lock, RpcError> {
        let from = self.member(params.from.trim())?;
        let given = params.version.trim().to_owned();
        if !given.is_empty() && lock::parse_version(&given).is_none() {
            return Err(RpcError::invalid_params(format!("`{given}` is not a version; write it as two numbers, like 1.08")));
        }
        let source = self.parts.clone();
        let files = {
            let from = from.clone();
            blocking(move || lock::files_of(&from, &source.read())).await?.map_err(RpcError::invalid_params)?
        };
        let (written, before) = {
            let mut state = self.state();
            let version = if given.is_empty() { lock::next_version(&state.lock.version) } else { given };
            let written = Lock { version, from: from.id.clone(), files, written_ms: now_ms() };
            let before = std::mem::replace(&mut state.lock, written.clone());
            (written, before)
        };
        self.put_lock(&written, &before);
        // The check publishes `standard.changed` with the new version and what now differs.
        self.check().await?;
        Ok(written)
    }

    /// Add or change a member, then check again (its column may read differently now).
    async fn edit(&self, mut member: Member) -> Result<(), RpcError> {
        member.id = member.id.trim().to_owned();
        if member.id.is_empty() || member.id.contains('/') {
            return Err(RpcError::invalid_params("a member needs its software id (`cortexterm`), without `/`"));
        }
        member.name = member.name.trim().to_owned();
        if member.name.is_empty() {
            member.name = member.id.clone();
        }
        member.root = member.root.trim().trim_end_matches('/').to_owned();
        if !member.root.is_empty() && !Path::new(&member.root).is_absolute() {
            return Err(RpcError::invalid_params("the member's folder is its full path on this computer (it starts with `/`)"));
        }
        let mut folders = BTreeMap::new();
        for (part, folder) in std::mem::take(&mut member.folders) {
            let (part, folder) = (part.trim().to_owned(), folder.trim().trim_matches('/').to_owned());
            if part.is_empty() || part.contains('/') || folder.is_empty() {
                continue;
            }
            if folder.split('/').any(|name| name == "..") {
                return Err(RpcError::invalid_params(format!("the folder of `{part}` stays inside the member's checkout (no `..`)")));
            }
            folders.insert(part, folder);
        }
        member.folders = folders;
        let clean = |items: &mut Vec<String>| {
            let mut seen = BTreeSet::new();
            *items = std::mem::take(items).into_iter().map(|item| item.trim().to_owned()).filter(|item| !item.is_empty() && seen.insert(item.clone())).collect();
        };
        clean(&mut member.without);
        clean(&mut member.names);
        let before = {
            let mut state = self.state();
            let before = state.members.iter().position(|kept| kept.id == member.id).map(|at| state.members.remove(at));
            state.members.push(member.clone());
            sort(&mut state.members);
            before
        };
        if before.as_ref() != Some(&member) {
            self.put_member(&member, before.as_ref());
        }
        self.check().await?;
        Ok(())
    }

    /// The parts a member carries with their folders, in the order of the parts.
    fn carried<'a>(member: &'a Member, parts: &'a Parts) -> impl Iterator<Item = (&'a Part, &'a str)> {
        parts.parts.iter().filter(|part| !member.without.contains(&part.id)).filter_map(|part| member.folders.get(&part.id).map(|folder| (part, folder.trim_matches('/')))).filter(|(_, folder)| !folder.is_empty())
    }

    /// Every standard path of a member, as globs relative to its repository's root.
    async fn paths(&self, member: &str) -> Result<Vec<String>, RpcError> {
        let member = self.member(member.trim())?;
        let parts = self.read_parts().await;
        let mut paths: Vec<String> = Self::carried(&member, &parts).flat_map(|(part, folder)| part.files.iter().map(move |pattern| format!("{folder}/{}", pattern.trim_matches('/')))).collect();
        paths.push(LOCK_FILE.to_owned());
        Ok(paths)
    }

    /// Which of these changed paths are standard files of the member (as they were given).
    async fn touches(&self, params: DiffParams) -> Result<Vec<String>, RpcError> {
        let member = self.member(params.member.trim())?;
        let parts = self.read_parts().await;
        let root = member.root.trim_end_matches('/');
        let touched = params
            .paths
            .into_iter()
            .filter(|given| {
                // Relative to the repository's root, whether it came that way or as a full path.
                let mut path = given.trim();
                if !root.is_empty()
                    && let Some(inside) = path.strip_prefix(root)
                    && inside.starts_with('/')
                {
                    path = inside;
                }
                let path = path.trim_start_matches("./").trim_start_matches('/');
                path == LOCK_FILE || Self::carried(&member, &parts).any(|(part, folder)| path.strip_prefix(folder).and_then(|rest| rest.strip_prefix('/')).is_some_and(|rest| parts.counts(part, rest)))
            })
            .collect();
        Ok(touched)
    }

    /// Ask every member on the mesh what a member answers to another member.
    ///
    /// Of the standard's read methods, a member's mesh answers exactly one to another member:
    /// `mesh.card` (mesh `net.rs`; `mesh.status`, `mesh.peers` and `mesh.services` are a frame's
    /// answers to its own window and are refused over the mesh). So each reachable member is asked
    /// for its card through `mesh.call`, and the answer is checked against the contract's `Card`.
    async fn handshake(&self) -> Result<Handshake, RpcError> {
        let ctx = self.ctx()?;
        let started = Instant::now();
        let peers: PeersResult = parse_params(ctx.call(mesh::methods::PEERS, json!({})).await?)?;
        let mut asked = tokio::task::JoinSet::new();
        for (at, peer) in peers.peers.into_iter().enumerate() {
            let ctx = ctx.clone();
            asked.spawn(async move { (at, ask(&ctx, peer).await) });
        }
        let mut rows = Vec::new();
        while let Some(done) = asked.join_next().await {
            match done {
                Ok(row) => rows.push(row),
                Err(error) => tracing::warn!(%error, "a handshake question stopped"),
            }
        }
        rows.sort_by_key(|(at, _)| *at);
        Ok(Handshake { rows: rows.into_iter().map(|(_, row)| row).collect(), ms: started.elapsed().as_millis() as u64 })
    }

    /// The self-test: a lock written from two real files in a folder made for it, printed and read
    /// back, and checked against a second copy under another name, before and after a change.
    pub async fn probe(&self) -> ProbeReport {
        let mut checks = Vec::new();
        let started = Instant::now();
        let source = self.parts.clone();
        let shipped = blocking(move || source.read()).await.unwrap_or_default();
        checks.push(ProbeCheck { name: "parts".into(), ok: !shipped.parts.is_empty(), detail: format!("{} parts in parts.toml", shipped.parts.len()), ms: started.elapsed().as_millis() as u64 });

        let started = Instant::now();
        let folder = self.data_dir.join(format!("probe-{}-{}", std::process::id(), now_ms()));
        let outcome = blocking(move || {
            let outcome = round_trip(&folder, shipped.skip);
            let _ = std::fs::remove_dir_all(&folder);
            outcome
        })
        .await
        .unwrap_or_else(|error| Err(error.message));
        let (ok, detail) = match outcome {
            Ok(detail) => (true, detail),
            Err(problem) => (false, problem),
        };
        checks.push(ProbeCheck { name: "lock".into(), ok, detail, ms: started.elapsed().as_millis() as u64 });
        ProbeReport::from_checks(checks)
    }
}

/// One member's answer to the handshake.
async fn ask(ctx: &Ctx, peer: Peer) -> HandshakeRow {
    let known = peer.card.as_ref();
    let mut row = HandshakeRow {
        node: peer.member.node.clone(),
        name: known.map(|card| card.name.clone()).filter(|name| !name.is_empty()).unwrap_or_else(|| peer.member.name.clone()),
        software: known.map(|card| card.software.clone()).unwrap_or_default(),
        state: String::new(),
        detail: String::new(),
    };
    let mut end = |state: &str, detail: String| {
        row.state = state.to_owned();
        row.detail = detail;
    };
    if !peer.reachable {
        end("unreachable", "Not connected now.".to_owned());
        return row;
    }
    let call = CallParams { node: peer.member.node.clone(), method: mesh::methods::CARD.to_owned(), params: json!({}) };
    match tokio::time::timeout(ANSWER, ctx.call(mesh::methods::CALL, json!(call))).await {
        Err(_) => end("unreachable", format!("No answer to mesh.card in {} s.", ANSWER.as_secs())),
        Ok(Err(error)) if matches!(error.code.as_str(), codes::UNAVAILABLE | codes::CONNECTION_CLOSED | codes::TIMEOUT) => end("unreachable", error.message),
        Ok(Err(error)) => end("fails", format!("mesh.card was refused: {}", error.message)),
        Ok(Ok(answer)) => match serde_json::from_value::<Card>(answer) {
            Err(error) => end("fails", format!("Its card does not have the standard's shape: {error}.")),
            Ok(card) if card.node != peer.member.node => end("fails", "It answered with another computer's card.".to_owned()),
            Ok(card) if card.software.is_empty() => end("fails", "Its card does not say which app it is.".to_owned()),
            Ok(card) => {
                let detail = format!("mesh.card: {} {}", card.software, card.version);
                row.software = card.software;
                if !card.name.is_empty() {
                    row.name = card.name;
                }
                row.state = "answers".to_owned();
                row.detail = detail.trim_end().to_owned();
            }
        },
    }
    row
}

/// The probe's work in `folder` (made here, removed by the caller). Returns what it found.
fn round_trip(folder: &Path, skip: Skip) -> Result<String, String> {
    let write = |path: PathBuf, text: &str| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
    };
    let parts = Parts { skip, own: crate::parts::Own { priority_after: vec!["PRIORITY: u32 = ".into()] }, parts: vec![Part { id: "kit".into(), files: vec!["**".into()] }] };
    let member = |id: &str, name: &str, priority: u32| Member {
        id: id.to_owned(),
        name: name.to_owned(),
        priority,
        folders: [("kit".to_owned(), "kit".to_owned())].into_iter().collect(),
        root: folder.join(id).to_string_lossy().into_owned(),
        names: vec![name.to_owned(), id.to_owned()],
        ..Member::default()
    };
    let (first, second) = (member("probeone", "ProbeOne", 7), member("probetwo", "ProbeTwo", 3));
    // Two files each: one that carries the member's own name and priority, one that is the same bytes.
    for member in [&first, &second] {
        let kit = Path::new(&member.root).join("kit");
        write(kit.join("lib.rs"), &format!("//! {} keeps this.\npub const SOFTWARE: &str = \"{}\";\npub const PRIORITY: u32 = {};\n", member.name, member.id, member.priority))?;
        write(kit.join("src").join("wire.rs"), "pub const ALPN: &[u8] = b\"probe/1\";\n")?;
        // Build output is never a standard file.
        write(kit.join("target").join("built.txt"), &member.id)?;
    }
    let files = lock::files_of(&first, &parts)?;
    let named: Vec<&str> = files.iter().filter(|file| file.names).map(|file| file.path.as_str()).collect();
    if files.len() != 2 || named != ["lib.rs"] {
        return Err(format!("expected 2 files with lib.rs hashed without the names; got {} files, names in {named:?}", files.len()));
    }
    let written = Lock { version: lock::next_version(""), from: first.id.clone(), files, written_ms: now_ms() };
    let members = [first.clone(), second.clone()];
    let text = lock::print(&written, &members);
    let (version, read_back) = lock::files_of_text(&text).map_err(|error| format!("the printed lock does not read back: {error}"))?;
    if version != written.version || read_back != written.files {
        return Err("the printed lock reads back differently".to_owned());
    }
    let states = |report: &Report| report.cells.iter().map(|cell| cell.state.clone()).collect::<Vec<_>>();
    let same = lock::check(&members, &written, &parts, now_ms());
    if states(&same) != ["names", "names"] || !same.drift.is_empty() {
        return Err(format!("two copies that differ only by name should read `names`; got {:?}", states(&same)));
    }
    write(Path::new(&second.root).join("kit").join("src").join("wire.rs"), "pub const ALPN: &[u8] = b\"probe/2\";\n")?;
    let changed = lock::check(&members, &written, &parts, now_ms());
    let cell = changed.cells.iter().find(|cell| cell.member == second.id);
    if changed.drift != [second.id.clone()] || cell.is_none_or(|cell| cell.state != "differs" || cell.files != ["src/wire.rs"]) {
        return Err(format!("a changed file should read `differs`; got {:?}, drift {:?}", states(&changed), changed.drift));
    }
    Ok(format!("lock {} of {} files written, printed, read back and checked", written.version, written.files.len()))
}
