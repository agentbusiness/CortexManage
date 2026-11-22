//! The records of `manage@1` (docs/specs/05-MANAGE-CONTRACT.md §1), kept in the graph and nowhere
//! else. The plugin works from a copy in memory, read once when it is activated; every change is
//! made to the copy and written through to the graph in the same step, in order.
//!
//! ```text
//! (:Repo)-[:ON]->(:Machine)                (:Repo)-[:HAS_REQUEST]->(:Request)-[:ASKS]->(:Question)
//! (:Request)-[:HAS_PLAN]->(:Plan)-[:HAS_POINT]->(:Point)-[:PROVED_BY]->(:Walk)
//! (:Repo)-[:HAS_PACKAGE]->(:Package)       (:Request)-[:PART_OF]->(:Package)
//! (:Package)-[:HAS_GATE]->(:Gate)          (:Package)-[:WATCHED]->(:Intervention)
//! (:Package)-[:HAS_EVIDENCE]->(:Evidence)  (:Request|:Package|:Repo)-[:DECIDED]->(:Decision)
//! (:Request)-[:MERGED_INTO]->(:Request)    (:ManageState)-[:ON]->(:Machine)
//! ```
//!
//! Nothing is deleted. A request that ended keeps its node (`ended_as`); an emptied package and a
//! replaced clarify question are retired. Every node carries `of: "manage"`, so a label another
//! plugin also uses is never read as ours. The properties the spec marks as keeping their history
//! (stage, tier, kind, urgent, the hold's end, a point's verdict, a gate's result) are also written
//! as temporal properties named `<property>_history`.

use crate::graph::{self, Graph, at, edge, machine, node, props};
use cc_protocol::RpcError;
use cc_protocol::contracts::manage::{Ask, Delivery, DeliveryFact, Evidence, Finding, From, Gate, Intervention, Package, Plan, Point, Policy, Question, Repo, Request, Source, Strip, Tier, Walk};
use cc_protocol::contracts::now_ms;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The mark on every node of this plugin.
pub const OF: &str = "manage";
const REPO: &str = "Repo";
const REQUEST: &str = "Request";
const QUESTION: &str = "Question";
const PLAN: &str = "Plan";
const POINT: &str = "Point";
const WALK: &str = "Walk";
const PACKAGE: &str = "Package";
const GATE: &str = "Gate";
const EVIDENCE: &str = "Evidence";
const INTERVENTION: &str = "Intervention";
const DECISION: &str = "Decision";
const DELIVERY: &str = "Delivery";
const STATE: &str = "ManageState";
const STATE_KEY: &str = "manage";
/// Rows per read.
const PAGE: u32 = 5000;

/// A repository and what the plugin keeps beside the contract's fields.
#[derive(Debug, Clone, Default)]
pub struct RepoRec {
    pub repo: Repo,
    /// The frame project its requests' sessions live under.
    pub project_id: String,
    /// The number of the last request id given.
    pub request_seq: u32,
}

/// A request and what the plugin keeps beside the contract's fields.
#[derive(Debug, Clone, Default)]
pub struct ReqRec {
    pub r: Request,
    /// When each stage was reached.
    pub stages: Vec<(String, u64)>,
    /// What the model proposed; the tier is the stricter of this and the floor.
    pub tier_model: Tier,
    pub urgent_model: bool,
    /// The person's own switch, when they used it.
    pub urgent_person: Option<bool>,
    /// The title was made from the text; the assessment may give a better one.
    pub title_auto: bool,
    /// What the person said since the last plan (the next plan's note).
    pub plan_note: String,
    /// The model's sentence for its tier.
    pub because: String,
    /// What the person said with the board before it had a package: `now` (start it as soon as
    /// it is planned), `wait` (wait until they start it), or empty.
    pub start_wish: String,
    /// The person took it off the board after it landed (its code stays).
    pub hidden: bool,
    /// A person reopened it: it is never merged into other work by itself again.
    pub reopened: bool,
    /// The commit message its work lands with (written when its package passed its gates).
    pub commit_message: String,
}

#[derive(Debug, Clone, Default)]
pub struct PlanRec {
    pub plan: Plan,
    /// The journal entry that shows it (patched when the verdicts are in).
    pub entry_seq: u64,
}

/// A decision wanted from a person, and its answer once given (`Decision` in the graph).
#[derive(Debug, Clone, Default)]
pub struct AskRec {
    pub ask: Ask,
    pub open: bool,
    /// A contradiction's second request.
    pub other_request: String,
    /// Where the work goes on after `rerun`: `dispatch`, `task:<n>`, `gates`, `land`.
    pub resume: String,
    /// person | policy.
    pub by: String,
    pub action: String,
    pub note: String,
    pub answered_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct IntRec {
    pub i: Intervention,
    /// The job's attempt it was made in.
    pub attempt: u32,
}

#[derive(Default)]
pub struct Data {
    pub repos: BTreeMap<String, RepoRec>,
    pub requests: BTreeMap<String, ReqRec>,
    /// Every version of a request's plan, oldest first.
    pub plans: BTreeMap<String, Vec<PlanRec>>,
    pub packages: BTreeMap<String, Package>,
    pub gates: BTreeMap<String, Vec<Gate>>,
    pub interventions: BTreeMap<String, Vec<IntRec>>,
    pub evidence: BTreeMap<String, Vec<Evidence>>,
    pub asks: BTreeMap<String, AskRec>,
    pub digests: BTreeMap<String, String>,
    /// The watch strip of each running job (computed from beats, not a record).
    pub strips: BTreeMap<String, Strip>,
    /// The Hold switch.
    pub hold: bool,
    /// The person's rules for the commits and releases of every repository.
    pub rules: String,
    pub package_seq: u32,
    pub ask_seq: u32,
    pub evidence_seq: u32,
}

impl Data {
    pub fn plan(&self, request: &str) -> Option<&PlanRec> {
        self.plans.get(request).and_then(|plans| plans.last())
    }

    /// The newest plan's kept points.
    pub fn kept(&self, request: &str) -> Vec<Point> {
        self.plan(request).map(|rec| rec.plan.points.iter().filter(|point| point.verdict == "keep").cloned().collect()).unwrap_or_default()
    }

    pub fn open_asks(&self) -> impl Iterator<Item = &AskRec> {
        self.asks.values().filter(|rec| rec.open)
    }

    /// The open asks about a package or one of its requests.
    pub fn asks_of(&self, package: &Package) -> Vec<&AskRec> {
        self.open_asks()
            .filter(|rec| {
                rec.ask.package == package.id
                    || (!rec.ask.request.is_empty() && package.requests.contains(&rec.ask.request))
                    || (!rec.other_request.is_empty() && package.requests.contains(&rec.other_request))
            })
            .collect()
    }

    pub fn sessions_of(&self, package: &Package) -> Vec<String> {
        package.requests.iter().filter_map(|id| self.requests.get(id)).map(|rec| rec.r.session_id.clone()).filter(|id| !id.is_empty()).collect()
    }
}

pub struct Store {
    pub graph: Graph,
    data: Mutex<Data>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

// ---- values ------------------------------------------------------------------------------------

/// A list property; an empty one is left out (null removes it).
fn list<T: serde::Serialize>(items: &[T]) -> Value {
    if items.is_empty() { Value::Null } else { json!(items) }
}

fn numbers(props: &Map<String, Value>, name: &str) -> Vec<u64> {
    props.get(name).and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
}

fn num(props: &Map<String, Value>, name: &str) -> u64 {
    graph::number(props, name).unwrap_or(0)
}

fn tier(props: &Map<String, Value>, name: &str) -> Tier {
    Tier::parse(&graph::text(props, name)).unwrap_or_default()
}

fn triple(props: &Map<String, Value>, name: &str, default: [u64; 3]) -> [u64; 3] {
    let values = numbers(props, name);
    if values.len() == 3 { [values[0], values[1], values[2]] } else { default }
}

fn temporal<const N: usize>(pairs: [(&str, Value); N]) -> Map<String, Value> {
    pairs.into_iter().map(|(name, value)| (name.to_owned(), value)).collect()
}

fn plan_key(request: &str, version: u32) -> String {
    format!("{request}/v{version}")
}

fn point_key(request: &str, version: u32, n: u32) -> String {
    format!("{request}/v{version}/p{n}")
}

// ---- records to nodes --------------------------------------------------------------------------

fn repo_props(rec: &RepoRec) -> Map<String, Value> {
    let (repo, policy) = (&rec.repo, &rec.repo.policy);
    props([
        ("of", json!(OF)),
        ("name", json!(repo.name)),
        ("prefix", json!(repo.prefix)),
        ("software", json!(repo.software)),
        ("source_kind", json!(repo.source.kind)),
        ("source_url", json!(repo.source.url)),
        ("source_main", json!(repo.source.main)),
        ("source_changes", json!(repo.source.changes)),
        ("source_commit_name", json!(repo.source.commit_name)),
        ("source_commit_email", json!(repo.source.commit_email)),
        ("source_github", json!(repo.source.github)),
        ("source_connection", json!(repo.source.connection)),
        ("rules", json!(repo.rules)),
        ("runner", json!(repo.runner)),
        ("runner_name", json!(repo.runner_name)),
        ("managed", json!(repo.managed)),
        ("created_ms", json!(repo.created_ms)),
        ("project_id", json!(rec.project_id)),
        ("request_seq", json!(rec.request_seq)),
        ("hold_safe_min", json!(policy.hold_safe_min)),
        ("hold_minor_min", json!(policy.hold_minor_min)),
        ("hold_max_min", json!(policy.hold_max_min)),
        ("soak_safe_min", json!(policy.soak_safe_min)),
        ("soak_minor_min", json!(policy.soak_minor_min)),
        ("soak_major_min", json!(policy.soak_major_min)),
        ("rework_safe", json!(policy.rework_safe)),
        ("rework_minor", json!(policy.rework_minor)),
        ("rework_major", json!(policy.rework_major)),
        ("budget_safe", json!(policy.budget_safe)),
        ("budget_minor", json!(policy.budget_minor)),
        ("budget_major", json!(policy.budget_major)),
        ("major_paths", list(&policy.major_paths)),
        ("minor_paths", list(&policy.minor_paths)),
        ("train_at", json!(policy.train_at)),
        ("train_every", json!(policy.train_every)),
        ("train_day", json!(policy.train_day)),
    ])
}

fn repo_from(p: &Map<String, Value>) -> RepoRec {
    let default = Policy::default();
    let small = |name: &str, default: u32| graph::number(p, name).map(|n| n as u32).unwrap_or(default);
    let policy = Policy {
        hold_safe_min: small("hold_safe_min", default.hold_safe_min),
        hold_minor_min: small("hold_minor_min", default.hold_minor_min),
        hold_max_min: small("hold_max_min", default.hold_max_min),
        soak_safe_min: small("soak_safe_min", default.soak_safe_min),
        soak_minor_min: small("soak_minor_min", default.soak_minor_min),
        soak_major_min: small("soak_major_min", default.soak_major_min),
        rework_safe: small("rework_safe", default.rework_safe),
        rework_minor: small("rework_minor", default.rework_minor),
        rework_major: small("rework_major", default.rework_major),
        budget_safe: triple(p, "budget_safe", default.budget_safe),
        budget_minor: triple(p, "budget_minor", default.budget_minor),
        budget_major: triple(p, "budget_major", default.budget_major),
        major_paths: graph::texts(p, "major_paths"),
        minor_paths: graph::texts(p, "minor_paths"),
        train_at: graph::text(p, "train_at"),
        train_every: graph::text(p, "train_every"),
        train_day: num(p, "train_day") as u32,
    };
    RepoRec {
        repo: Repo {
            id: graph::text(p, "key"),
            name: graph::text(p, "name"),
            prefix: graph::text(p, "prefix"),
            software: graph::text(p, "software"),
            source: Source {
                kind: graph::text(p, "source_kind"),
                url: graph::text(p, "source_url"),
                main: graph::text(p, "source_main"),
                changes: graph::text(p, "source_changes"),
                commit_name: graph::text(p, "source_commit_name"),
                commit_email: graph::text(p, "source_commit_email"),
                github: graph::text(p, "source_github"),
                connection: graph::text(p, "source_connection"),
            },
            runner: graph::text(p, "runner"),
            runner_name: graph::text(p, "runner_name"),
            policy,
            managed: p.get("managed").and_then(Value::as_bool).unwrap_or(true),
            created_ms: num(p, "created_ms"),
            // Read from its own node after the repositories.
            delivery: Delivery::default(),
            rules: graph::text(p, "rules"),
        },
        project_id: graph::text(p, "project_id"),
        request_seq: num(p, "request_seq") as u32,
    }
}

fn request_props(rec: &ReqRec) -> Map<String, Value> {
    let r = &rec.r;
    let person = match rec.urgent_person {
        Some(true) => "yes",
        Some(false) => "no",
        None => "",
    };
    props([
        ("of", json!(OF)),
        ("repo", json!(r.repo)),
        ("session_id", json!(r.session_id)),
        ("title", json!(r.title)),
        ("title_auto", json!(rec.title_auto)),
        ("text", json!(r.text)),
        ("from_kind", json!(r.from.kind)),
        ("from_name", json!(r.from.name)),
        ("from_node", json!(r.from.node)),
        ("kind", json!(r.kind)),
        ("tier", json!(r.tier.as_str())),
        ("tier_floor", json!(r.tier_floor.as_str())),
        ("tier_model", json!(rec.tier_model.as_str())),
        ("tier_reasons", list(&r.tier_reasons)),
        ("because", json!(rec.because)),
        ("start_wish", json!(rec.start_wish)),
        ("hidden", json!(rec.hidden)),
        ("reopened", json!(rec.reopened)),
        ("commit_message", json!(rec.commit_message)),
        ("follows", json!(r.follows)),
        ("must_ship", json!(r.must_ship)),
        ("repairs_release", json!(r.repairs_release)),
        ("surface", list(&r.surface)),
        ("urgent", json!(r.urgent)),
        ("urgent_because", json!(r.urgent_because)),
        ("urgent_model", json!(rec.urgent_model)),
        ("urgent_person", json!(person)),
        ("stage", json!(r.stage)),
        ("stages", list(&rec.stages.iter().map(|(stage, _)| stage.clone()).collect::<Vec<_>>())),
        ("stage_ms", list(&rec.stages.iter().map(|(_, ms)| *ms).collect::<Vec<_>>())),
        ("ended_as", json!(r.ended_as)),
        ("closed_reason", json!(r.ended_reason)),
        ("merged_into", json!(r.merged_into)),
        ("package", json!(r.package)),
        ("parts", list(&r.parts)),
        ("fingerprint", json!(r.fingerprint)),
        ("seen", json!(r.seen)),
        ("seen_last_ms", json!(r.seen_last_ms)),
        ("plan_note", json!(rec.plan_note)),
        ("created_ms", json!(r.created_ms)),
        ("updated_ms", json!(r.updated_ms)),
    ])
}

fn request_from(p: &Map<String, Value>) -> ReqRec {
    let stages: Vec<(String, u64)> = graph::texts(p, "stages").into_iter().zip(numbers(p, "stage_ms")).collect();
    ReqRec {
        r: Request {
            id: graph::text(p, "key"),
            repo: graph::text(p, "repo"),
            session_id: graph::text(p, "session_id"),
            title: graph::text(p, "title"),
            text: graph::text(p, "text"),
            from: From { kind: graph::text(p, "from_kind"), name: graph::text(p, "from_name"), node: graph::text(p, "from_node") },
            kind: graph::text(p, "kind"),
            tier: tier(p, "tier"),
            tier_floor: tier(p, "tier_floor"),
            tier_reasons: graph::texts(p, "tier_reasons"),
            surface: graph::texts(p, "surface"),
            urgent: graph::flag(p, "urgent"),
            urgent_because: graph::text(p, "urgent_because"),
            stage: graph::text(p, "stage"),
            ended_as: graph::text(p, "ended_as"),
            ended_reason: graph::text(p, "closed_reason"),
            merged_into: graph::text(p, "merged_into"),
            package: graph::text(p, "package"),
            parts: graph::texts(p, "parts"),
            questions: Vec::new(),
            created_ms: num(p, "created_ms"),
            updated_ms: num(p, "updated_ms"),
            fingerprint: graph::text(p, "fingerprint"),
            seen: num(p, "seen"),
            seen_last_ms: num(p, "seen_last_ms"),
            follows: graph::text(p, "follows"),
            must_ship: graph::flag(p, "must_ship"),
            repairs_release: graph::flag(p, "repairs_release"),
        },
        stages,
        tier_model: tier(p, "tier_model"),
        urgent_model: graph::flag(p, "urgent_model"),
        urgent_person: match graph::text(p, "urgent_person").as_str() {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        },
        title_auto: graph::flag(p, "title_auto"),
        plan_note: graph::text(p, "plan_note"),
        because: graph::text(p, "because"),
        start_wish: graph::text(p, "start_wish"),
        hidden: graph::flag(p, "hidden"),
        reopened: graph::flag(p, "reopened"),
        commit_message: graph::text(p, "commit_message"),
    }
}

fn delivery_props(delivery: &Delivery) -> Map<String, Value> {
    let d = delivery;
    props([
        ("of", json!(OF)),
        ("state", json!(d.state)),
        ("error", json!(d.error)),
        ("learned_ms", json!(d.learned_ms)),
        ("reach_main", json!(d.reach_main)),
        ("main_is", json!(d.main_is)),
        ("release_kind", json!(d.release_kind)),
        ("release_steps", list(&d.release_steps)),
        ("plan_step", json!(d.plan_step)),
        ("tag", json!(d.tag)),
        ("version_kind", json!(d.version_kind)),
        ("version_file", json!(d.version_file)),
        ("version_key", json!(d.version_key)),
        ("version_now", json!(d.version_now)),
        ("checks", list(&d.checks)),
        ("ships", json!(d.ships)),
        ("ci", json!(d.ci)),
        ("take_back", json!(d.take_back)),
        ("secrets", list(&d.secrets)),
        ("secret_files", list(&d.secret_files)),
        ("secrets_from", json!(d.secrets_from)),
        // Where each value was read, as four lists of the same length.
        ("fact_fields", list(&d.facts.iter().map(|fact| fact.field.clone()).collect::<Vec<_>>())),
        ("fact_paths", list(&d.facts.iter().map(|fact| fact.path.clone()).collect::<Vec<_>>())),
        ("fact_lines", list(&d.facts.iter().map(|fact| fact.line).collect::<Vec<_>>())),
        ("fact_quotes", list(&d.facts.iter().map(|fact| fact.quote.clone()).collect::<Vec<_>>())),
        ("set_by_person", list(&d.set_by_person)),
        ("failed_releases", json!(d.failed_releases)),
        ("files", list(&d.files)),
        ("hashes", list(&d.hashes)),
    ])
}

fn delivery_from(p: &Map<String, Value>) -> Delivery {
    let (fields, paths, lines, quotes) = (graph::texts(p, "fact_fields"), graph::texts(p, "fact_paths"), numbers(p, "fact_lines"), graph::texts(p, "fact_quotes"));
    let facts = fields
        .into_iter()
        .enumerate()
        .map(|(at, field)| DeliveryFact { field, path: paths.get(at).cloned().unwrap_or_default(), line: lines.get(at).copied().unwrap_or(0) as u32, quote: quotes.get(at).cloned().unwrap_or_default() })
        .collect();
    Delivery {
        state: graph::text(p, "state"),
        error: graph::text(p, "error"),
        learned_ms: num(p, "learned_ms"),
        reach_main: graph::text(p, "reach_main"),
        main_is: graph::text(p, "main_is"),
        release_kind: graph::text(p, "release_kind"),
        release_steps: graph::texts(p, "release_steps"),
        plan_step: graph::text(p, "plan_step"),
        tag: graph::text(p, "tag"),
        version_kind: graph::text(p, "version_kind"),
        version_file: graph::text(p, "version_file"),
        version_key: graph::text(p, "version_key"),
        version_now: graph::text(p, "version_now"),
        checks: graph::texts(p, "checks"),
        ships: graph::text(p, "ships"),
        ci: graph::text(p, "ci"),
        take_back: graph::text(p, "take_back"),
        secrets: graph::texts(p, "secrets"),
        secret_files: graph::texts(p, "secret_files"),
        secrets_from: graph::text(p, "secrets_from"),
        facts,
        set_by_person: graph::texts(p, "set_by_person"),
        failed_releases: num(p, "failed_releases") as u32,
        files: graph::texts(p, "files"),
        hashes: graph::texts(p, "hashes"),
    }
}

fn package_props(package: &Package, digest: &str) -> Map<String, Value> {
    props([
        ("of", json!(OF)),
        ("repo", json!(package.repo)),
        ("requests", list(&package.requests)),
        ("tier", json!(package.tier.as_str())),
        ("urgent", json!(package.urgent)),
        ("stage", json!(package.stage)),
        ("hold_until_ms", json!(package.hold_until_ms)),
        ("held_since_ms", json!(package.held_since_ms)),
        ("branch", json!(package.branch)),
        ("base", json!(package.base)),
        ("attempt", json!(package.attempt)),
        ("reworks", json!(package.reworks)),
        ("parts", list(&package.parts)),
        ("runner", json!(package.runner)),
        ("runner_name", json!(package.runner_name)),
        ("job_id", json!(package.job_id)),
        ("session_id", json!(package.session_id)),
        ("landed", json!(package.landed)),
        ("landed_ms", json!(package.landed_ms)),
        ("soak_until_ms", json!(package.soak_until_ms)),
        ("stopped", json!(package.stopped)),
        ("created_ms", json!(package.created_ms)),
        ("digest", json!(digest)),
    ])
}

fn package_from(p: &Map<String, Value>) -> Package {
    Package {
        id: graph::text(p, "key"),
        repo: graph::text(p, "repo"),
        requests: graph::texts(p, "requests"),
        tier: tier(p, "tier"),
        urgent: graph::flag(p, "urgent"),
        stage: graph::text(p, "stage"),
        hold_until_ms: num(p, "hold_until_ms"),
        held_since_ms: num(p, "held_since_ms"),
        branch: graph::text(p, "branch"),
        base: graph::text(p, "base"),
        attempt: num(p, "attempt") as u32,
        reworks: num(p, "reworks") as u32,
        parts: graph::texts(p, "parts"),
        runner: graph::text(p, "runner"),
        runner_name: graph::text(p, "runner_name"),
        job_id: graph::text(p, "job_id"),
        session_id: graph::text(p, "session_id"),
        landed: graph::text(p, "landed"),
        landed_ms: num(p, "landed_ms"),
        soak_until_ms: num(p, "soak_until_ms"),
        stopped: graph::flag(p, "stopped"),
        created_ms: num(p, "created_ms"),
    }
}

fn ask_props(rec: &AskRec) -> Map<String, Value> {
    let ask = &rec.ask;
    props([
        ("of", json!(OF)),
        ("kind", json!(ask.kind)),
        ("repo", json!(ask.repo)),
        ("request", json!(ask.request)),
        ("other_request", json!(rec.other_request)),
        ("package", json!(ask.package)),
        ("title", json!(ask.title)),
        ("subject", json!(ask.subject)),
        ("why", json!(ask.why)),
        ("if_yes", json!(ask.if_yes)),
        ("if_no", json!(ask.if_no)),
        ("actions", list(&ask.actions)),
        ("attempt", json!(ask.attempt)),
        ("since_ms", json!(ask.since_ms)),
        ("open", json!(rec.open)),
        ("resume", json!(rec.resume)),
        ("by", json!(rec.by)),
        ("action", json!(rec.action)),
        ("note", json!(rec.note)),
        ("ms", json!(rec.answered_ms)),
    ])
}

fn ask_from(p: &Map<String, Value>) -> AskRec {
    AskRec {
        ask: Ask {
            id: graph::text(p, "key"),
            kind: graph::text(p, "kind"),
            repo: graph::text(p, "repo"),
            request: graph::text(p, "request"),
            package: graph::text(p, "package"),
            title: graph::text(p, "title"),
            subject: graph::text(p, "subject"),
            why: graph::text(p, "why"),
            if_yes: graph::text(p, "if_yes"),
            if_no: graph::text(p, "if_no"),
            actions: graph::texts(p, "actions"),
            attempt: num(p, "attempt") as u32,
            since_ms: num(p, "since_ms"),
        },
        open: graph::flag(p, "open"),
        other_request: graph::text(p, "other_request"),
        resume: graph::text(p, "resume"),
        by: graph::text(p, "by"),
        action: graph::text(p, "action"),
        note: graph::text(p, "note"),
        answered_ms: num(p, "ms"),
    }
}

impl Store {
    pub fn new() -> Self {
        Self { graph: Graph::new(), data: Mutex::new(Data::default()) }
    }

    /// Look at the records. The lock is held only for `look`, never across a wait.
    pub fn read<T>(&self, look: impl FnOnce(&Data) -> T) -> T {
        look(&self.data.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    /// Change what is only in memory (the strips), or several records at once before saving them.
    pub fn write<T>(&self, change: impl FnOnce(&mut Data) -> T) -> T {
        change(&mut self.data.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    // ---- repositories --------------------------------------------------------------------------

    pub fn put_repo(&self, rec: RepoRec) {
        self.save_repo(&rec);
        self.write(|data| data.repos.insert(rec.repo.id.clone(), rec));
    }

    fn save_repo(&self, rec: &RepoRec) {
        self.graph.put(vec![node(REPO, &rec.repo.id, repo_props(rec))], vec![edge((REPO, &rec.repo.id), "ON", machine())]);
        let delivery = cc_protocol::contracts::memory::GraphNode {
            label: DELIVERY.into(),
            key: rec.repo.id.clone(),
            props: delivery_props(&rec.repo.delivery),
            temporal: temporal([("release_kind_history", json!(rec.repo.delivery.release_kind)), ("main_is_history", json!(rec.repo.delivery.main_is))]),
        };
        self.graph.put(vec![delivery], vec![edge((REPO, &rec.repo.id), "SHIPS_BY", at(DELIVERY, &rec.repo.id))]);
    }

    pub fn update_repo(&self, id: &str, change: impl FnOnce(&mut RepoRec)) -> Option<RepoRec> {
        let rec = self.write(|data| {
            let rec = data.repos.get_mut(id)?;
            change(rec);
            Some(rec.clone())
        })?;
        self.save_repo(&rec);
        Some(rec)
    }

    /// The next request id of a repository (`CT-0142`).
    pub fn next_request_id(&self, repo: &str) -> Option<String> {
        let rec = self.update_repo(repo, |rec| rec.request_seq += 1)?;
        Some(request_id(&rec.repo.prefix, rec.request_seq))
    }

    // ---- requests ------------------------------------------------------------------------------

    pub fn put_request(&self, rec: ReqRec) {
        self.save_request(&rec, &[]);
        self.write(|data| data.requests.insert(rec.r.id.clone(), rec));
    }

    fn save_request(&self, rec: &ReqRec, old_questions: &[Question]) {
        let r = &rec.r;
        let history = temporal([
            ("stage_history", json!(r.stage)),
            ("tier_history", json!(r.tier.as_str())),
            ("kind_history", json!(r.kind)),
            ("urgent_history", json!(r.urgent)),
        ]);
        let mut nodes = vec![cc_protocol::contracts::memory::GraphNode { label: REQUEST.into(), key: r.id.clone(), props: request_props(rec), temporal: history }];
        let mut edges = vec![edge((REPO, &r.repo), "HAS_REQUEST", at(REQUEST, &r.id))];
        for (index, question) in r.questions.iter().enumerate() {
            let key = format!("{}/{}", r.id, question.id);
            nodes.push(node(
                QUESTION,
                &key,
                props([
                    ("of", json!(OF)),
                    ("request", json!(r.id)),
                    ("id", json!(question.id)),
                    ("n", json!(index)),
                    ("text", json!(question.text)),
                    ("options", list(&question.options)),
                    ("multi", json!(question.multi)),
                    ("answer", list(&question.answer)),
                ]),
            ));
            edges.push(edge((REQUEST, &r.id), "ASKS", at(QUESTION, &key)));
        }
        self.graph.put(nodes, edges);
        // Links to records that may not be stored yet go in their own puts: a refused link must
        // not take the request with it.
        if !r.package.is_empty() {
            self.graph.put(Vec::new(), vec![edge((REQUEST, &r.id), "PART_OF", at(PACKAGE, &r.package))]);
        }
        if !r.merged_into.is_empty() {
            self.graph.put(Vec::new(), vec![edge((REQUEST, &r.id), "MERGED_INTO", at(REQUEST, &r.merged_into))]);
        }
        let gone: Vec<_> =
            old_questions.iter().filter(|old| !r.questions.iter().any(|new| new.id == old.id)).map(|old| at(QUESTION, &format!("{}/{}", r.id, old.id))).collect();
        self.graph.retire(gone, "replaced");
    }

    pub fn update_request(&self, id: &str, change: impl FnOnce(&mut ReqRec)) -> Option<ReqRec> {
        let (rec, old) = self.write(|data| {
            let rec = data.requests.get_mut(id)?;
            let old = rec.r.questions.clone();
            change(rec);
            rec.r.updated_ms = now_ms();
            Some((rec.clone(), old))
        })?;
        self.save_request(&rec, &old);
        Some(rec)
    }

    /// Move a request to a stage and note when.
    pub fn set_stage(&self, id: &str, stage: &str) -> Option<ReqRec> {
        self.update_request(id, |rec| {
            if rec.r.stage != stage || rec.stages.is_empty() {
                rec.r.stage = stage.to_owned();
                rec.stages.push((stage.to_owned(), now_ms()));
            }
        })
    }

    // ---- plans ---------------------------------------------------------------------------------

    /// Store a plan version (a new one, or the same version with its verdicts).
    pub fn put_plan(&self, request: &str, rec: PlanRec) {
        let plan = &rec.plan;
        let key = plan_key(request, plan.version);
        let mut nodes = vec![node(
            PLAN,
            &key,
            props([
                ("of", json!(OF)),
                ("request", json!(request)),
                ("version", json!(plan.version)),
                ("analysis", json!(plan.analysis)),
                ("goal", json!(plan.goal)),
                ("approach", json!(plan.approach)),
                ("architecture", json!(plan.architecture)),
                ("risks", list(&plan.risks)),
                ("note", json!(plan.note)),
                ("created_ms", json!(plan.created_ms)),
                ("entry_seq", json!(rec.entry_seq)),
            ]),
        )];
        let mut edges = vec![edge((REQUEST, request), "HAS_PLAN", at(PLAN, &key))];
        for (index, point) in plan.points.iter().enumerate() {
            let point_key = point_key(request, plan.version, point.n);
            nodes.push(cc_protocol::contracts::memory::GraphNode {
                label: POINT.into(),
                key: point_key.clone(),
                props: props([
                    ("of", json!(OF)),
                    ("request", json!(request)),
                    ("version", json!(plan.version)),
                    ("order", json!(index)),
                    ("n", json!(point.n)),
                    ("title", json!(point.title)),
                    ("brief", json!(point.brief)),
                    ("known", list(&point.known)),
                    ("rules", list(&point.rules)),
                    ("depends_on", list(&point.depends_on)),
                    ("touches", list(&point.touches)),
                    ("checks", list(&point.checks)),
                    ("limit_files", json!(point.limit_files)),
                    ("limit_lines", json!(point.limit_lines)),
                    ("verdict", json!(point.verdict)),
                    ("verdict_reason", json!(point.verdict_reason)),
                ]),
                temporal: temporal([("verdict_history", json!(point.verdict))]),
            });
            edges.push(edge((PLAN, &key), "HAS_POINT", at(POINT, &point_key)));
            for (walk_index, walk) in point.walks.iter().enumerate() {
                let walk_key = format!("{point_key}/w{walk_index}");
                nodes.push(node(
                    WALK,
                    &walk_key,
                    props([
                        ("of", json!(OF)),
                        ("request", json!(request)),
                        ("version", json!(plan.version)),
                        ("point", json!(point.n)),
                        ("order", json!(walk_index)),
                        ("id", json!(walk.id)),
                        ("do", json!(walk.do_)),
                        ("see", json!(walk.see)),
                        ("method", json!(walk.method)),
                        ("evidence_kind", json!(walk.evidence_kind)),
                        ("command", json!(walk.command)),
                        ("expect", json!(walk.expect)),
                        // Walks are the manager's; a builder never sees them.
                        ("hidden", json!(true)),
                    ]),
                ));
                edges.push(edge((POINT, &point_key), "PROVED_BY", at(WALK, &walk_key)));
            }
        }
        self.graph.put(nodes, edges);
        self.write(|data| {
            let plans = data.plans.entry(request.to_owned()).or_default();
            match plans.iter_mut().find(|old| old.plan.version == rec.plan.version) {
                Some(old) => *old = rec,
                None => plans.push(rec),
            }
        });
    }

    // ---- packages ------------------------------------------------------------------------------

    pub fn next_package_id(&self) -> String {
        let seq = self.write(|data| {
            data.package_seq += 1;
            data.package_seq
        });
        self.save_state();
        format!("pk-{seq:04}")
    }

    pub fn put_package(&self, package: Package) {
        let digest = self.read(|data| data.digests.get(&package.id).cloned().unwrap_or_default());
        self.save_package(&package, &digest);
        self.write(|data| data.packages.insert(package.id.clone(), package));
    }

    fn save_package(&self, package: &Package, digest: &str) {
        let node = cc_protocol::contracts::memory::GraphNode {
            label: PACKAGE.into(),
            key: package.id.clone(),
            props: package_props(package, digest),
            temporal: temporal([("hold_until_history", json!(package.hold_until_ms)), ("stage_history", json!(package.stage))]),
        };
        self.graph.put(vec![node], vec![edge((REPO, &package.repo), "HAS_PACKAGE", at(PACKAGE, &package.id))]);
    }

    pub fn update_package(&self, id: &str, change: impl FnOnce(&mut Package)) -> Option<Package> {
        let (package, digest) = self.write(|data| {
            let package = data.packages.get_mut(id)?;
            change(package);
            let package = package.clone();
            Some((package, data.digests.get(id).cloned().unwrap_or_default()))
        })?;
        self.save_package(&package, &digest);
        Some(package)
    }

    /// An emptied package leaves the lists; its node stays in the graph, marked as gone.
    pub fn retire_package(&self, id: &str, reason: &str) {
        self.write(|data| {
            data.packages.remove(id);
            data.strips.remove(id);
        });
        self.graph.retire(vec![at(PACKAGE, id)], reason);
    }

    pub fn set_digest(&self, id: &str, text: &str) {
        self.write(|data| data.digests.insert(id.to_owned(), text.to_owned()));
        self.update_package(id, |_| {});
    }

    pub fn add_gate(&self, package: &str, gate: Gate) {
        let index = self.write(|data| {
            let gates = data.gates.entry(package.to_owned()).or_default();
            gates.push(gate.clone());
            gates.len()
        });
        let key = format!("{package}/g{index}");
        let node = cc_protocol::contracts::memory::GraphNode {
            label: GATE.into(),
            key: key.clone(),
            props: props([
                ("of", json!(OF)),
                ("package", json!(package)),
                ("order", json!(index)),
                ("kind", json!(gate.kind)),
                ("attempt", json!(gate.attempt)),
                ("result", json!(gate.result)),
                ("finding_titles", list(&gate.findings.iter().map(|finding| finding.title.clone()).collect::<Vec<_>>())),
                ("finding_evidence", list(&gate.findings.iter().map(|finding| finding.evidence.clone()).collect::<Vec<_>>())),
                ("ms", json!(gate.ms)),
            ]),
            temporal: temporal([("result_history", json!(gate.result))]),
        };
        self.graph.put(vec![node], vec![edge((PACKAGE, package), "HAS_GATE", at(GATE, &key))]);
    }

    pub fn add_intervention(&self, package: &str, intervention: Intervention, attempt: u32) {
        let index = self.write(|data| {
            let list = data.interventions.entry(package.to_owned()).or_default();
            list.push(IntRec { i: intervention.clone(), attempt });
            list.len()
        });
        let key = format!("{package}/i{index}");
        let node = node(
            INTERVENTION,
            &key,
            props([
                ("of", json!(OF)),
                ("package", json!(package)),
                ("order", json!(index)),
                ("rule", json!(intervention.rule)),
                ("fact", json!(intervention.fact)),
                ("action", json!(intervention.action)),
                ("text", json!(intervention.text)),
                ("task", json!(intervention.task)),
                ("attempt", json!(attempt)),
                ("ms", json!(intervention.ms)),
            ]),
        );
        self.graph.put(vec![node], vec![edge((PACKAGE, package), "WATCHED", at(INTERVENTION, &key))]);
    }

    pub fn next_evidence_id(&self) -> String {
        let seq = self.write(|data| {
            data.evidence_seq += 1;
            data.evidence_seq
        });
        self.save_state();
        format!("ev-{seq:05}")
    }

    pub fn add_evidence(&self, package: &str, evidence: Evidence) {
        let node = node(
            EVIDENCE,
            &evidence.id,
            props([
                ("of", json!(OF)),
                ("package", json!(package)),
                ("walk", json!(evidence.walk)),
                ("request", json!(evidence.request)),
                ("kind", json!(evidence.kind)),
                ("result", json!(evidence.result)),
                ("seen", json!(evidence.seen)),
                ("build", json!(evidence.build)),
                ("ms", json!(evidence.ms)),
            ]),
        );
        self.graph.put(vec![node], vec![edge((PACKAGE, package), "HAS_EVIDENCE", at(EVIDENCE, &evidence.id))]);
        self.write(|data| data.evidence.entry(package.to_owned()).or_default().push(evidence));
    }

    // ---- asks and decisions --------------------------------------------------------------------

    pub fn next_ask_id(&self) -> String {
        let seq = self.write(|data| {
            data.ask_seq += 1;
            data.ask_seq
        });
        self.save_state();
        format!("ask-{seq:05}")
    }

    pub fn put_ask(&self, rec: AskRec) {
        let ask = &rec.ask;
        let from = if !ask.package.is_empty() && self.read(|data| data.packages.contains_key(&ask.package)) {
            (PACKAGE, ask.package.as_str())
        } else if !ask.request.is_empty() {
            (REQUEST, ask.request.as_str())
        } else {
            (REPO, ask.repo.as_str())
        };
        self.graph.put(vec![node(DECISION, &ask.id, ask_props(&rec))], vec![edge(from, "DECIDED", at(DECISION, &ask.id))]);
        self.write(|data| data.asks.insert(rec.ask.id.clone(), rec));
    }

    // ---- the plugin's own state ----------------------------------------------------------------

    pub fn set_hold(&self, on: bool) {
        self.write(|data| data.hold = on);
        self.save_state();
    }

    pub fn set_rules(&self, rules: &str) {
        self.write(|data| data.rules = rules.to_owned());
        self.save_state();
    }

    fn save_state(&self) {
        let (hold, packages, asks, evidence, rules) = self.read(|data| (data.hold, data.package_seq, data.ask_seq, data.evidence_seq, data.rules.clone()));
        let props = props([("of", json!(OF)), ("held", json!(hold)), ("package_seq", json!(packages)), ("ask_seq", json!(asks)), ("evidence_seq", json!(evidence)), ("rules", json!(rules))]);
        self.graph.put(vec![node(STATE, STATE_KEY, props)], vec![edge((STATE, STATE_KEY), "ON", machine())]);
    }

    // ---- reading it all back -------------------------------------------------------------------

    /// Every live node of a label that is ours, by key.
    async fn all(&self, label: &str) -> Result<Vec<Map<String, Value>>, RpcError> {
        let cypher = format!("MATCH (n:{label}) WHERE n.ended_ms IS NULL AND n.of = $of AND n.key > $after RETURN n ORDER BY n.key");
        let mut out: Vec<Map<String, Value>> = Vec::new();
        let mut after = String::new();
        loop {
            let params = props([("of", json!(OF)), ("after", json!(after))]);
            let rows = self.graph.nodes(&cypher, params, PAGE).await?;
            let count = rows.len();
            if let Some(last) = rows.last() {
                after = graph::text(last, "key");
            }
            out.extend(rows);
            if count < PAGE as usize {
                return Ok(out);
            }
        }
    }

    /// Read the records from the graph into memory (once, when the plugin is activated).
    pub async fn load(&self) -> Result<(), RpcError> {
        // Writes asked for before the read go first, so the read follows them.
        self.graph.flush().await;
        let mut data = Data::default();
        if let Some(state) = self.all(STATE).await?.first() {
            data.hold = graph::flag(state, "held");
            data.package_seq = num(state, "package_seq") as u32;
            data.ask_seq = num(state, "ask_seq") as u32;
            data.evidence_seq = num(state, "evidence_seq") as u32;
            data.rules = graph::text(state, "rules");
        }
        for p in self.all(REPO).await? {
            let rec = repo_from(&p);
            // A `Repo` node another plugin made (a charter's) has no name of ours.
            if !rec.repo.name.is_empty() {
                data.repos.insert(rec.repo.id.clone(), rec);
            }
        }
        for p in self.all(DELIVERY).await? {
            if let Some(rec) = data.repos.get_mut(&graph::text(&p, "key")) {
                rec.repo.delivery = delivery_from(&p);
            }
        }
        for p in self.all(REQUEST).await? {
            let rec = request_from(&p);
            data.requests.insert(rec.r.id.clone(), rec);
        }
        let mut questions = self.all(QUESTION).await?;
        questions.sort_by_key(|p| num(p, "n"));
        for p in questions {
            if let Some(rec) = data.requests.get_mut(&graph::text(&p, "request")) {
                rec.r.questions.push(Question {
                    id: graph::text(&p, "id"),
                    text: graph::text(&p, "text"),
                    options: graph::texts(&p, "options"),
                    multi: graph::flag(&p, "multi"),
                    answer: graph::texts(&p, "answer"),
                });
            }
        }
        for p in self.all(PLAN).await? {
            let plan = Plan {
                version: num(&p, "version") as u32,
                analysis: graph::text(&p, "analysis"),
                goal: graph::text(&p, "goal"),
                approach: graph::text(&p, "approach"),
                architecture: graph::text(&p, "architecture"),
                risks: graph::texts(&p, "risks"),
                points: Vec::new(),
                note: graph::text(&p, "note"),
                created_ms: num(&p, "created_ms"),
            };
            data.plans.entry(graph::text(&p, "request")).or_default().push(PlanRec { plan, entry_seq: num(&p, "entry_seq") });
        }
        for plans in data.plans.values_mut() {
            plans.sort_by_key(|rec| rec.plan.version);
        }
        let mut points = self.all(POINT).await?;
        points.sort_by_key(|p| num(p, "order"));
        for p in points {
            let request = graph::text(&p, "request");
            let version = num(&p, "version") as u32;
            let Some(rec) = data.plans.get_mut(&request).and_then(|plans| plans.iter_mut().find(|rec| rec.plan.version == version)) else { continue };
            rec.plan.points.push(Point {
                n: num(&p, "n") as u32,
                title: graph::text(&p, "title"),
                brief: graph::text(&p, "brief"),
                known: graph::texts(&p, "known"),
                rules: graph::texts(&p, "rules"),
                depends_on: numbers(&p, "depends_on").into_iter().map(|n| n as u32).collect(),
                touches: graph::texts(&p, "touches"),
                checks: graph::texts(&p, "checks"),
                limit_files: num(&p, "limit_files") as u32,
                limit_lines: num(&p, "limit_lines") as u32,
                walks: Vec::new(),
                verdict: graph::text(&p, "verdict"),
                verdict_reason: graph::text(&p, "verdict_reason"),
                request,
            });
        }
        let mut walks = self.all(WALK).await?;
        walks.sort_by_key(|p| num(p, "order"));
        for p in walks {
            let version = num(&p, "version") as u32;
            let n = num(&p, "point") as u32;
            let point = data
                .plans
                .get_mut(&graph::text(&p, "request"))
                .and_then(|plans| plans.iter_mut().find(|rec| rec.plan.version == version))
                .and_then(|rec| rec.plan.points.iter_mut().find(|point| point.n == n));
            if let Some(point) = point {
                point.walks.push(Walk {
                    id: graph::text(&p, "id"),
                    do_: graph::text(&p, "do"),
                    see: graph::text(&p, "see"),
                    method: graph::text(&p, "method"),
                    evidence_kind: graph::text(&p, "evidence_kind"),
                    command: graph::text(&p, "command"),
                    expect: graph::text(&p, "expect"),
                });
            }
        }
        for p in self.all(PACKAGE).await? {
            let package = package_from(&p);
            let digest = graph::text(&p, "digest");
            if !digest.is_empty() {
                data.digests.insert(package.id.clone(), digest);
            }
            data.packages.insert(package.id.clone(), package);
        }
        let mut gates = self.all(GATE).await?;
        gates.sort_by_key(|p| num(p, "order"));
        for p in gates {
            let findings = graph::texts(&p, "finding_titles")
                .into_iter()
                .zip(graph::texts(&p, "finding_evidence").into_iter().chain(std::iter::repeat(String::new())))
                .map(|(title, evidence)| Finding { title, evidence })
                .collect();
            let gate = Gate { kind: graph::text(&p, "kind"), attempt: num(&p, "attempt") as u32, result: graph::text(&p, "result"), findings, ms: num(&p, "ms") };
            data.gates.entry(graph::text(&p, "package")).or_default().push(gate);
        }
        let mut interventions = self.all(INTERVENTION).await?;
        interventions.sort_by_key(|p| num(p, "order"));
        for p in interventions {
            let intervention = Intervention {
                rule: graph::text(&p, "rule"),
                fact: graph::text(&p, "fact"),
                action: graph::text(&p, "action"),
                text: graph::text(&p, "text"),
                task: num(&p, "task") as u32,
                ms: num(&p, "ms"),
            };
            data.interventions.entry(graph::text(&p, "package")).or_default().push(IntRec { i: intervention, attempt: num(&p, "attempt") as u32 });
        }
        let mut evidence = self.all(EVIDENCE).await?;
        evidence.sort_by_key(|p| num(p, "ms"));
        for p in evidence {
            let item = Evidence {
                id: graph::text(&p, "key"),
                walk: graph::text(&p, "walk"),
                request: graph::text(&p, "request"),
                kind: graph::text(&p, "kind"),
                result: graph::text(&p, "result"),
                seen: graph::text(&p, "seen"),
                build: graph::text(&p, "build"),
                ms: num(&p, "ms"),
            };
            data.evidence.entry(graph::text(&p, "package")).or_default().push(item);
        }
        for p in self.all(DECISION).await? {
            let rec = ask_from(&p);
            data.asks.insert(rec.ask.id.clone(), rec);
        }
        self.write(|slot| *slot = data);
        Ok(())
    }
}

/// `CT`, 142 -> `CT-0142`.
pub fn request_id(prefix: &str, seq: u32) -> String {
    format!("{prefix}-{seq:04}")
}
