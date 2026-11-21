//! `manage@1`: CortexManage's own contract (docs/specs/05-MANAGE-CONTRACT.md). Requests, packages,
//! the hold, the job on a runner, the watch, gates and the inbox. Not part of the shared standard.
//!
//! Records live in the graph (the memory plugin); the shapes here are what the methods answer.
//! The word a person sees ([`Word`]) is computed from the facts every time and never stored.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod methods {
    /// `{}` -> `{repos: [Repo]}`.
    pub const REPO_LIST: &str = "manage.repo.list";
    /// `{id}` -> [`super::Repo`].
    pub const REPO_GET: &str = "manage.repo.get";
    /// [`super::RepoCreateParams`] -> [`super::Repo`].
    pub const REPO_CREATE: &str = "manage.repo.create";
    /// [`super::RepoEditParams`] -> [`super::Repo`].
    pub const REPO_EDIT: &str = "manage.repo.edit";
    /// [`super::RequestCreateParams`] -> [`super::RequestView`]: a new request and its session.
    pub const REQUEST_CREATE: &str = "manage.request.create";
    /// `{id}` -> [`super::RequestView`].
    pub const REQUEST_GET: &str = "manage.request.get";
    /// [`super::RequestListParams`] -> `{requests: [RequestView]}`.
    pub const REQUEST_LIST: &str = "manage.request.list";
    /// [`super::RequestEditParams`] -> [`super::RequestView`]: the person's own switches.
    pub const REQUEST_EDIT: &str = "manage.request.edit";
    /// `{id}` -> [`super::PackageView`].
    pub const PACKAGE_GET: &str = "manage.package.get";
    /// `{repo?}` -> `{packages: [PackageView]}`.
    pub const PACKAGE_LIST: &str = "manage.package.list";
    /// [`super::PackageEditParams`] -> [`super::PackageView`]: the hold's buttons.
    pub const PACKAGE_EDIT: &str = "manage.package.edit";
    /// [`super::DecideParams`] -> `{ok}`: answers an open decision. Refused with `conflict` when
    /// `attempt` is not the current one.
    pub const DECIDE: &str = "manage.decide";
    /// `{}` -> [`super::Inbox`].
    pub const INBOX_LIST: &str = "manage.inbox.list";
    /// `{}` -> `{on}`.
    pub const HOLD_GET: &str = "manage.hold.get";
    /// `{on}` -> `{on}`: the Hold switch. While on, no new job is started.
    pub const HOLD_EDIT: &str = "manage.hold.edit";
    /// `{id}` -> `{kind, mime, base64}`: the bytes of a shot, recording or log.
    pub const EVIDENCE_READ: &str = "manage.evidence.read";
    /// [`super::SayParams`] -> `{text}`: the person wrote in a request's session. Before building
    /// the text is added to the request and it is assessed again; while building it is a steer to
    /// the builder. The answer is the line to show.
    pub const REQUEST_SAY: &str = "manage.request.say";
    /// Planning's read-only discovery, served from the runner's mirror (or a checkout on this
    /// computer). [`super::DiscoverParams`] -> `{text}` | `{hits}` | `{entries}`.
    pub const DISCOVER_READ: &str = "manage.discover.read";
    pub const DISCOVER_SEARCH: &str = "manage.discover.search";
    pub const DISCOVER_TREE: &str = "manage.discover.tree";
    /// `{repo, plan_only}` -> the runner's `ReleaseAnswer`: run the repository's release scripts
    /// on its runner (the release train calls this; the manage plugin holds the link).
    pub const RUNNER_RELEASE: &str = "manage.runner.release";
    /// `{repo}` -> `{reachable, node, name, jobs}`: the repository's runner as the Runner page shows it.
    pub const RUNNER_GET: &str = "manage.runner.get";
    /// `{}` -> `{items: [BoardItem], asks: [Ask]}`: every open request (and what was done in the
    /// last seven days) with the column it is in and the facts of its one line, and the decisions
    /// that wait on a person.
    pub const BOARD_ITEMS: &str = "manage.board.items";
    /// The shared service declaration (`<category>.shares`).
    pub const SHARES: &str = "manage.shares";
}

pub mod topics {
    /// [`super::Changed`] after any change.
    pub const CHANGED: &str = "manage.changed";
    /// [`super::WatchBeat`] on every beat that changed something a person sees.
    pub const WATCH: &str = "manage.watch";
}

/// The stages a request moves through (`Request.stage`).
pub mod stage {
    pub const ARRIVED: &str = "arrived";
    pub const CLARIFYING: &str = "clarifying";
    pub const ASSESSED: &str = "assessed";
    pub const PLANNED: &str = "planned";
    pub const VALIDATED: &str = "validated";
    pub const HELD: &str = "held";
    pub const BUILDING: &str = "building";
    pub const PROVING: &str = "proving";
    pub const LANDED: &str = "landed";
    pub const SOAKING: &str = "soaking";
    pub const RELEASED: &str = "released";
    /// In order, for the card's ladder.
    pub const ALL: &[&str] = &[ARRIVED, CLARIFYING, ASSESSED, PLANNED, VALIDATED, HELD, BUILDING, PROVING, LANDED, SOAKING, RELEASED];
}

/// How a request ended (`Request.ended_as`).
pub mod ended {
    pub const RELEASED: &str = "released";
    pub const DECLINED: &str = "declined";
    pub const MERGED: &str = "merged";
    pub const WITHDRAWN: &str = "withdrawn";
}

/// safe < minor < major. The stricter of the host's floor and the model's proposal wins.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Safe,
    Minor,
    Major,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Safe => "safe",
            Tier::Minor => "minor",
            Tier::Major => "major",
        }
    }
    pub fn parse(text: &str) -> Option<Tier> {
        match text {
            "safe" => Some(Tier::Safe),
            "minor" => Some(Tier::Minor),
            "major" => Some(Tier::Major),
            _ => None,
        }
    }
}

/// The one word a person sees, by this precedence (first that holds).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Word {
    OnHold,
    NeedsYou,
    Stuck,
    Rework,
    Working,
    Watching,
    Held,
    Queued,
    Done,
    #[default]
    Idle,
}

/// Where a repository's main line lives. Leaving GitHub means changing `url`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Source {
    /// `git` (the only kind in this version).
    #[serde(default)]
    pub kind: String,
    pub url: String,
    /// The branch packages land on.
    #[serde(default)]
    pub main: String,
    /// `none` | `github-pr`: whether a landed package also opens a pull request.
    #[serde(default)]
    pub changes: String,
    /// Who the commits of this repository's jobs are made as. Both empty: git's own identity on
    /// the computer that builds.
    #[serde(default)]
    pub commit_name: String,
    #[serde(default)]
    pub commit_email: String,
    /// The repository on GitHub, `owner/name`, as a person set it: the apps that declare this
    /// repository send their reports here (`contracts::reports`). Empty: no reports.
    #[serde(default)]
    pub github: String,
}

/// Per repository; a global policy can only be stricter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    pub hold_safe_min: u32,
    pub hold_minor_min: u32,
    pub hold_max_min: u32,
    pub soak_safe_min: u32,
    pub soak_minor_min: u32,
    pub soak_major_min: u32,
    pub rework_safe: u32,
    pub rework_minor: u32,
    pub rework_major: u32,
    /// `[minutes, tokens, lines]` per tier.
    pub budget_safe: [u64; 3],
    pub budget_minor: [u64; 3],
    pub budget_major: [u64; 3],
    /// Path globs that make a change major (also every path in `standard.lock`).
    pub major_paths: Vec<String>,
    /// Path globs that make a change at least minor.
    pub minor_paths: Vec<String>,
    /// `HH:MM`, the train's time of day.
    pub train_at: String,
    /// How often the train leaves: `3h` (every three hours from `train_at`), `week` (on
    /// `train_day`), or empty: every day.
    #[serde(default)]
    pub train_every: String,
    /// For a weekly train: 0 Monday … 6 Sunday.
    #[serde(default)]
    pub train_day: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            hold_safe_min: 120,
            hold_minor_min: 720,
            hold_max_min: 1440,
            soak_safe_min: 720,
            soak_minor_min: 1440,
            soak_major_min: 2880,
            rework_safe: 2,
            rework_minor: 2,
            rework_major: 1,
            budget_safe: [30, 200_000, 150],
            budget_minor: [90, 600_000, 600],
            budget_major: [240, 2_000_000, 2500],
            // Nothing about any one repository's layout is built in: a repository's own paths are
            // set in its settings.
            major_paths: vec!["AGENTS.md".to_owned()],
            minor_paths: Vec::new(),
            train_at: "03:00".into(),
            train_every: String::new(),
            train_day: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Repo {
    /// Short and stable (`cortexterm`); also the prefix of request ids (`CT`).
    pub id: String,
    pub name: String,
    /// Two or three capitals for request ids (`CT-0142`).
    pub prefix: String,
    /// The app's `software` id on the mesh, when it is a Cortex app.
    #[serde(default)]
    pub software: String,
    pub source: Source,
    /// The mesh node of the computer that builds it (the runner). Empty: this computer.
    #[serde(default)]
    pub runner: String,
    /// The runner's name for people.
    #[serde(default)]
    pub runner_name: String,
    #[serde(default)]
    pub policy: Policy,
    /// False while a repository is left out (listed, greyed, never built).
    #[serde(default = "yes")]
    pub managed: bool,
    #[serde(default)]
    pub created_ms: u64,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RepoCreateParams {
    pub name: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub software: String,
    pub source: Source,
    #[serde(default)]
    pub runner: String,
    #[serde(default)]
    pub runner_name: String,
    #[serde(default = "yes")]
    pub managed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RepoEditParams {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub runner: Option<String>,
    #[serde(default)]
    pub runner_name: Option<String>,
    #[serde(default)]
    pub policy: Option<Policy>,
    #[serde(default)]
    pub managed: Option<bool>,
}

/// Who a request came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct From {
    /// `person` | `app` | `policy`.
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub node: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Request {
    /// `CT-0142`.
    pub id: String,
    pub repo: String,
    pub session_id: String,
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub from: From,
    /// fix | feature | design | flow | refactor | performance | architecture | standard | security
    /// | release | docs | charter. Empty until assessed.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub tier: Tier,
    /// The host's floor, from paths and facts.
    #[serde(default)]
    pub tier_floor: Tier,
    /// One line per reason the tier is what it is.
    #[serde(default)]
    pub tier_reasons: Vec<String>,
    /// frontend | backend | flow.
    #[serde(default)]
    pub surface: Vec<String>,
    #[serde(default)]
    pub urgent: bool,
    /// The sentence of the request that shows it is urgent.
    #[serde(default)]
    pub urgent_because: String,
    pub stage: String,
    /// Empty while open ([`ended`]).
    #[serde(default)]
    pub ended_as: String,
    #[serde(default)]
    pub ended_reason: String,
    /// The request this one was merged into.
    #[serde(default)]
    pub merged_into: String,
    #[serde(default)]
    pub package: String,
    /// The parts (plugins, `frame`) the plan touches.
    #[serde(default)]
    pub parts: Vec<String>,
    /// Open questions when it cannot be acted on (the clarify form).
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub updated_ms: u64,
    /// For a request an app reported (an error, a panic): the failure's fingerprint, how many
    /// times it was seen, and when last. A report of the same fingerprint while the request is
    /// open is counted here.
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub seen: u64,
    #[serde(default)]
    pub seen_last_ms: u64,
}

/// One question of the clarify form.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub answer: Vec<String>,
}

/// A request with what is computed about it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RequestView {
    #[serde(flatten)]
    pub request: Request,
    pub word: Word,
    /// The newest plan, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    /// When each stage was reached (`[stage, ms]`).
    #[serde(default)]
    pub stages: Vec<(String, u64)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RequestCreateParams {
    pub repo: String,
    pub text: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub from: From,
    #[serde(default)]
    pub urgent: Option<bool>,
    /// What the person attached when writing it.
    #[serde(default)]
    pub attachments: Vec<RequestFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RequestListParams {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    /// Also the ones that ended.
    #[serde(default)]
    pub with_ended: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RequestEditParams {
    pub id: String,
    #[serde(default)]
    pub urgent: Option<bool>,
    #[serde(default)]
    pub withdraw: bool,
    /// Answers to the clarify form (`[question id, answers]`).
    #[serde(default)]
    pub answers: Vec<(String, Vec<String>)>,
    /// More text from the person (re-assessed).
    #[serde(default)]
    pub add_text: Option<String>,
    /// The person moved it on the board: `true` start it now (or as soon as it is planned),
    /// `false` let it wait until they start it.
    #[serde(default)]
    pub start: Option<bool>,
    /// The person moved it back: plan it again.
    #[serde(default)]
    pub replan: bool,
}

/// The five columns of the board, in a person's words.
pub mod column {
    /// Read, sorted and planned.
    pub const INCOMING: &str = "incoming";
    /// Planned, waiting for its start (or paused by a person).
    pub const WAITING: &str = "waiting";
    /// A CortexCode is building or checking it.
    pub const CODING: &str = "coding";
    /// Built and on the main branch, running in the person's apps until the next release.
    pub const DONE: &str = "done";
    /// In a release (the last seven days).
    pub const SHIPPED: &str = "shipped";
}

/// A request as the board shows it: its column and the facts of its one line.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BoardItem {
    #[serde(flatten)]
    pub request: Request,
    pub word: Word,
    /// [`column`].
    pub column: String,
    /// A person paused it (it waits until started).
    #[serde(default)]
    pub paused: bool,
    /// When it starts by itself (waiting); 0 when it waits for a yes or is paused.
    #[serde(default)]
    pub starts_ms: u64,
    /// The step of its plan being built, and how many steps there are (coding).
    #[serde(default)]
    pub step: u32,
    #[serde(default)]
    pub steps: u32,
    /// Checking what was built (coding).
    #[serde(default)]
    pub checking: bool,
    /// Started but not building yet (coding): `next` (it starts in a moment), `after` (after the
    /// work of its repository being built now), `builder` (no CortexCode can take it yet).
    #[serde(default)]
    pub queued: String,
    /// Until when it runs in the person's apps, and the release after that (done).
    #[serde(default)]
    pub soak_until_ms: u64,
    #[serde(default)]
    pub release_ms: u64,
    /// When it was released (done).
    #[serde(default)]
    pub released_ms: u64,
    /// The package it is built in when that holds more than this request: they build together.
    #[serde(default)]
    pub together: String,
    /// How many reports of the same thing were merged into it (0: none), and who sent them.
    #[serde(default)]
    pub reports: u32,
    #[serde(default)]
    pub reporters: Vec<String>,
}

/// One acceptance test as steps in the real app. Hidden from the builder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Walk {
    pub id: String,
    /// What to do.
    #[serde(rename = "do")]
    pub do_: String,
    /// What is seen when it works.
    pub see: String,
    /// computer | command | mesh | journal.
    pub method: String,
    /// shot | recording | log | trace.
    #[serde(default)]
    pub evidence_kind: String,
    /// For `command`: the command to run in the proving home.
    #[serde(default)]
    pub command: String,
    /// For `command`: text its output must contain.
    #[serde(default)]
    pub expect: String,
}

/// One plan task with what CortexManage adds to CortexCode's plan shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub n: u32,
    pub title: String,
    #[serde(default)]
    pub brief: String,
    #[serde(default)]
    pub known: Vec<String>,
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<u32>,
    /// The scope: files or folders it may touch.
    #[serde(default)]
    pub touches: Vec<String>,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub limit_files: u32,
    #[serde(default)]
    pub limit_lines: u32,
    #[serde(default)]
    pub walks: Vec<Walk>,
    /// keep | cut | merge | ask; empty until validated.
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub verdict_reason: String,
    /// The request this point came from (a package's joined plan).
    #[serde(default)]
    pub request: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    pub version: u32,
    #[serde(default)]
    pub analysis: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub approach: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub risks: Vec<String>,
    pub points: Vec<Point>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub created_ms: u64,
}

/// One gate's answer. `result`: passed | failed | blocked | not_run. Never a silent pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Gate {
    /// scope | standard | charter | build | walks | converge | human.
    pub kind: String,
    pub attempt: u32,
    pub result: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub title: String,
    #[serde(default)]
    pub evidence: String,
}

/// What a walk left behind.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub id: String,
    pub walk: String,
    pub request: String,
    /// shot | recording | log | trace.
    pub kind: String,
    /// passed | failed | blocked | not_run.
    pub result: String,
    /// What was seen, in one or two lines.
    #[serde(default)]
    pub seen: String,
    /// The commit it was taken on.
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub ms: u64,
}

/// One time CortexManage stepped in while a job ran.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Intervention {
    /// W1..W8 | drift | replan | approval.
    pub rule: String,
    /// The fact that fired it.
    pub fact: String,
    /// steer | restart | stop | refuse | note.
    pub action: String,
    /// The text sent to the builder, when one was.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub task: u32,
    #[serde(default)]
    pub ms: u64,
}

/// The strip of the Watch pane.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Strip {
    pub task: u32,
    pub tasks: u32,
    pub attempt: u32,
    pub checkpoint_ms: u64,
    pub files: u32,
    pub lines: u32,
    pub limit_lines: u64,
    pub in_scope: bool,
    pub minutes: u64,
    pub limit_minutes: u64,
    pub steered: u32,
    pub restarted: u32,
    /// The builder's last tool call in plain words.
    #[serde(default)]
    pub now: String,
    /// The rules and whether each fired in this attempt (`[rule, fired]`).
    #[serde(default)]
    pub rules: Vec<(String, bool)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Package {
    /// `pk-0142`.
    pub id: String,
    pub repo: String,
    pub requests: Vec<String>,
    #[serde(default)]
    pub tier: Tier,
    #[serde(default)]
    pub urgent: bool,
    pub stage: String,
    /// When the hold ends (ms); 0 for a major package (it ends by a yes).
    #[serde(default)]
    pub hold_until_ms: u64,
    #[serde(default)]
    pub held_since_ms: u64,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub attempt: u32,
    /// How many rework rounds were used.
    #[serde(default)]
    pub reworks: u32,
    #[serde(default)]
    pub parts: Vec<String>,
    /// The computer that builds this package (its mesh node and its name): the repository's, or
    /// the free CortexCode chosen when its job started.
    #[serde(default)]
    pub runner: String,
    #[serde(default)]
    pub runner_name: String,
    /// The runner's job, once started.
    #[serde(default)]
    pub job_id: String,
    #[serde(default)]
    pub session_id: String,
    /// The commit that landed.
    #[serde(default)]
    pub landed: String,
    #[serde(default)]
    pub landed_ms: u64,
    #[serde(default)]
    pub soak_until_ms: u64,
    /// A person stopped it; it does not go by the clock.
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub created_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PackageView {
    #[serde(flatten)]
    pub package: Package,
    pub word: Word,
    /// The joined plan of its requests (kept points only).
    #[serde(default)]
    pub points: Vec<Point>,
    #[serde(default)]
    pub gates: Vec<Gate>,
    #[serde(default)]
    pub interventions: Vec<Intervention>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strip: Option<Strip>,
    #[serde(default)]
    pub digest: String,
    /// `[minutes, tokens, lines]`.
    #[serde(default)]
    pub budget: [u64; 3],
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PackageEditParams {
    pub id: String,
    #[serde(default)]
    pub start_now: bool,
    #[serde(default)]
    pub stop: bool,
    /// Take this request out of the package (it builds apart).
    #[serde(default)]
    pub unbundle: Option<String>,
    /// Build this package together with the one this request is in: its requests join that
    /// package. Both must be waiting, in the same repository.
    #[serde(default)]
    pub join: Option<String>,
}

/// A decision wanted from a person: what fills the inbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Ask {
    pub id: String,
    /// approve_plan | approve_landing | stuck | contradiction | blocked_walk | charter.
    pub kind: String,
    pub repo: String,
    #[serde(default)]
    pub request: String,
    #[serde(default)]
    pub package: String,
    pub title: String,
    /// The question in one line.
    pub subject: String,
    /// Why it waits, written from facts.
    pub why: String,
    #[serde(default)]
    pub if_yes: String,
    #[serde(default)]
    pub if_no: String,
    /// The actions allowed (`approve`, `send_back`, `decline`, `rerun`, `take_over`, `first`, `second`).
    pub actions: Vec<String>,
    pub attempt: u32,
    pub since_ms: u64,
}

/// Something going by itself that a person may still stop.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Going {
    pub package: String,
    pub repo: String,
    pub title: String,
    /// `builds 21:13`, `urgent · building`.
    pub line: String,
    #[serde(default)]
    pub hold_until_ms: u64,
    pub urgent: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Inbox {
    pub asks: Vec<Ask>,
    pub going: Vec<Going>,
    /// The band's sentence: `2 need you · 2 building · 3 held`.
    pub count_line: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DecideParams {
    /// The ask's id.
    pub id: String,
    pub attempt: u32,
    pub action: String,
    #[serde(default)]
    pub note: String,
}

/// The ASSESS stage's answer ([`flow::ASSESS`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AssessedParams {
    pub request: String,
    pub actionable: bool,
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub title: String,
    pub kind: String,
    pub tier: Tier,
    #[serde(default)]
    pub surface: Vec<String>,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default)]
    pub because: String,
    #[serde(default)]
    pub urgent_because: String,
}


/// Published on [`topics::CHANGED`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Changed {
    /// repo | request | package | inbox | hold.
    pub what: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub repo: String,
}

/// Published on [`topics::WATCH`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WatchBeat {
    pub package: String,
    pub strip: Strip,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub extra: Value,
}

// ---- the chat flow's stages ---------------------------------------------------------------------

/// The stage methods of the `chatflow-manage` flow (category `chatflow-manage`, contract
/// `chatflow@1` plus these). The manage plugin calls them; each is one model question with a JSON
/// schema and greedy decoding, and answers when the model has. Only `plan` uses tools, and only
/// the read-only `manage.discover.*`.
pub mod flow {
    pub const CATEGORY: &str = "chatflow-manage";
    /// [`super::AssessParams`] -> [`super::AssessedParams`].
    pub const ASSESS: &str = "chatflow-manage.assess";
    /// [`super::MatchParams`] -> [`super::MatchAnswer`].
    pub const MATCH: &str = "chatflow-manage.match";
    /// [`super::PlanParams`] -> [`super::Plan`].
    pub const PLAN: &str = "chatflow-manage.plan";
    /// [`super::ValidateParams`] -> [`super::ValidateAnswer`].
    pub const VALIDATE: &str = "chatflow-manage.validate";
    /// [`super::DriftParams`] -> [`super::DriftAnswer`].
    pub const DRIFT: &str = "chatflow-manage.drift";
    /// [`super::ConvergeParams`] -> [`super::ConvergeAnswer`].
    pub const CONVERGE: &str = "chatflow-manage.converge";
    /// `{facts}` -> `{text}`: two lines for the Watch pane. Nothing is decided from it.
    pub const DIGEST: &str = "chatflow-manage.digest";
    /// `{facts}` -> [`super::WhyAnswer`]: the inbox item's words.
    pub const WHY: &str = "chatflow-manage.why";
}

/// The journal entry kinds the manage plugin writes into a request's session, beside the frame's
/// own (`user_message`, `assistant_message`, `note`, `status`). The chat UI renders each.
pub mod kinds {
    /// `{kind, tier, tier_floor, reasons[], surface[], urgent, urgent_because, because, parts[]}`.
    pub const ASSESSMENT: &str = "assessment";
    /// `{questions: [Question]}`: the clarify form.
    pub const CLARIFY: &str = "clarify";
    /// `{plan: Plan}`: the plan with its verdicts (walks are shown; the builder never sees them).
    pub const PLAN: &str = "manage_plan";
    /// `{package, text, hold_until_ms, joined[], urgent}`: held, joined a package, started.
    pub const PACKAGE: &str = "package";
    /// An [`super::Intervention`], or `{checkpoint: {...}, drift: {...}}`.
    pub const WATCH: &str = "watch";
    /// `{text}`: the watch's two lines.
    pub const DIGEST: &str = "digest";
    /// A [`super::Gate`].
    pub const GATE: &str = "gate";
    /// `{stage, text}`: a stage was reached.
    pub const STAGE: &str = "stage";
    /// `{ask: Ask}`: a decision is wanted; `{ask_id, action, by}` when it was answered.
    pub const ASK: &str = "ask";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SayParams {
    pub request: String,
    pub text: String,
    /// What the person attached to the message (stored by the chat flow's `attachment.put`).
    #[serde(default)]
    pub attachments: Vec<RequestFile>,
}

/// A file attached to a request or one of its messages, as the chat flow stored it (the fields of
/// `chatflow::Attachment` the manager reads).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RequestFile {
    /// image | file | folder.
    pub kind: String,
    /// Where it is: the stored copy, or the person's own folder.
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub bytes: u64,
    /// Long text the person pasted, kept as a file.
    #[serde(default)]
    pub pasted_text: bool,
    #[serde(default)]
    pub lines: Option<u64>,
    /// A small preview of an image (a `data:` URL), for the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoverParams {
    pub repo: String,
    /// read: the file; tree: the folder (empty: the root).
    #[serde(default)]
    pub path: String,
    /// search: a fixed string, and a path prefix to search under.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub under: String,
    #[serde(default)]
    pub from: Option<u32>,
    #[serde(default)]
    pub lines: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AssessParams {
    pub session_id: String,
    pub request: String,
    pub text: String,
    pub repo_name: String,
    /// What the app is and what it must never become (from its charter).
    #[serde(default)]
    pub identity: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MatchParams {
    pub a_title: String,
    pub a_text: String,
    pub b_title: String,
    pub b_text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MatchAnswer {
    /// same | together | contradict | unrelated.
    pub relation: String,
    #[serde(default)]
    pub because: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PlanParams {
    pub session_id: String,
    pub request: String,
    pub text: String,
    /// The repository's id, for `manage.discover.*`.
    pub repo: String,
    pub repo_name: String,
    #[serde(default)]
    pub identity: String,
    /// The charter's laws and constraints, one line each.
    #[serde(default)]
    pub rules: Vec<String>,
    pub tier: Tier,
    pub kind: String,
    /// What the person said about an earlier plan, when this is a new version.
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ValidateParams {
    pub session_id: String,
    /// The request's text.
    pub text: String,
    pub point: Point,
    #[serde(default)]
    pub identity: String,
    /// The charter's laws, one line each.
    #[serde(default)]
    pub laws: Vec<String>,
    /// True when the point touches a standard file and the request is not a standard change.
    #[serde(default)]
    pub touches_standard: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ValidateAnswer {
    pub necessary: bool,
    pub sensible: bool,
    pub fits: bool,
    pub provable: bool,
    pub standard_safe: bool,
    /// keep | cut | merge | ask.
    pub verdict: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DriftParams {
    pub title: String,
    #[serde(default)]
    pub rules: Vec<String>,
    pub diff: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DriftAnswer {
    /// True when the diff does what the task says. The host turns a `yes` whose `file` and `line`
    /// are not in the diff into a `no`.
    pub does_it: bool,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ConvergeParams {
    /// `[n, title, rules]` of every kept point.
    pub points: Vec<(u32, String, Vec<String>)>,
    pub diff: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ConvergeAnswer {
    /// Changes no point asked for: `[file, why]`.
    #[serde(default)]
    pub extra: Vec<(String, String)>,
    /// Points the diff does not do: `[n, why]`.
    #[serde(default)]
    pub missing: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WhyAnswer {
    pub why: String,
    #[serde(default)]
    pub if_yes: String,
    #[serde(default)]
    pub if_no: String,
}
