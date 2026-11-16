//! The plugin's records as nodes and edges, and back.
//!
//! - `(:Soak {package, repo, tier, urgent, commit, landed_ms, until_ms, state, parts, reports,
//!   request_ids, request_titles, request_tiers, release, taken_back_request})`, key: the package.
//!   The requests are three lists of the same length, one entry per request.
//! - `(:Release {id, repo, ms, state, note, tail, hotfix, packages})-[:SHIPS]->(:Soak)`, key: the id.
//! - `(:Release)-[:MOVES]->(:PartMove {release, n, part, from, to, carries})`, key: `<release>/<part>`.
//! - `(:TrainClock {repo, ran_day, skip_day})`, key: the repository: the day the daily train last
//!   left and the day it was told to skip.

use crate::graph::{self, at, edge, node, props};
use cc_protocol::contracts::memory::{GraphEdge, GraphNode};
use cc_protocol::contracts::train::{PartMove, Release, Soak};
use serde_json::{Map, Value, json};

pub const SOAK: &str = "Soak";
pub const RELEASE: &str = "Release";
pub const PART_MOVE: &str = "PartMove";
pub const CLOCK: &str = "TrainClock";
pub const SHIPS: &str = "SHIPS";
pub const MOVES: &str = "MOVES";

/// `Soak.state`.
pub mod soak_state {
    pub const SOAKING: &str = "soaking";
    pub const READY: &str = "ready";
    pub const HELD_BACK: &str = "held_back";
    pub const RELEASED: &str = "released";
    pub const TAKEN_BACK: &str = "taken_back";
}

/// `Release.state`.
pub mod release_state {
    pub const RAN: &str = "ran";
    pub const FAILED: &str = "failed";
    pub const SKIPPED: &str = "skipped";
    pub const NOTHING: &str = "nothing";
}

/// A soak with what only this plugin keeps about it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Kept {
    pub soak: Soak,
    /// The release that shipped it.
    pub release: String,
    /// The request that takes it back.
    pub taken_back_request: String,
}

/// What the daily train remembers per repository.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clock {
    /// The day (`2026-11-16`) the daily train's time last passed and was acted on.
    pub ran_day: String,
    /// The day whose train does not run.
    pub skip_day: String,
}

fn optional(text: &str) -> Value {
    if text.is_empty() { Value::Null } else { json!(text) }
}

pub fn soak_node(kept: &Kept) -> GraphNode {
    let soak = &kept.soak;
    let column = |pick: fn(&(String, String, String)) -> &String| graph::list(&soak.requests.iter().map(|request| pick(request).clone()).collect::<Vec<_>>());
    node(
        SOAK,
        &soak.package,
        props([
            ("package", json!(soak.package)),
            ("repo", json!(soak.repo)),
            ("tier", json!(soak.tier)),
            ("urgent", json!(soak.urgent)),
            ("commit", json!(soak.commit)),
            ("landed_ms", json!(soak.landed_ms)),
            ("until_ms", json!(soak.until_ms)),
            ("state", json!(soak.state)),
            ("parts", graph::list(&soak.parts)),
            ("reports", graph::list(&soak.reports)),
            ("request_ids", column(|request| &request.0)),
            ("request_titles", column(|request| &request.1)),
            ("request_tiers", column(|request| &request.2)),
            ("release", optional(&kept.release)),
            ("taken_back_request", optional(&kept.taken_back_request)),
        ]),
    )
}

pub fn soak_from(props: &Map<String, Value>) -> Kept {
    let ids = graph::texts(props, "request_ids");
    let titles = graph::texts(props, "request_titles");
    let tiers = graph::texts(props, "request_tiers");
    let requests = ids.into_iter().enumerate().map(|(n, id)| (id, titles.get(n).cloned().unwrap_or_default(), tiers.get(n).cloned().unwrap_or_default())).collect();
    Kept {
        soak: Soak {
            package: graph::text(props, "package"),
            repo: graph::text(props, "repo"),
            requests,
            parts: graph::texts(props, "parts"),
            tier: graph::text(props, "tier"),
            urgent: graph::flag(props, "urgent"),
            commit: graph::text(props, "commit"),
            landed_ms: graph::number(props, "landed_ms"),
            until_ms: graph::number(props, "until_ms"),
            reports: graph::texts(props, "reports"),
            state: graph::text(props, "state"),
        },
        release: graph::text(props, "release"),
        taken_back_request: graph::text(props, "taken_back_request"),
    }
}

fn move_key(release: &str, part: &str) -> String {
    format!("{release}/{part}")
}

/// A release with its part moves, and the edges to them and to the soaks it shipped.
pub fn release_nodes(release: &Release, shipped: &[String]) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut nodes = vec![node(
        RELEASE,
        &release.id,
        props([
            ("id", json!(release.id)),
            ("repo", json!(release.repo)),
            ("ms", json!(release.ms)),
            ("state", json!(release.state)),
            ("note", json!(release.note)),
            ("tail", graph::list(&release.tail)),
            ("hotfix", json!(release.hotfix)),
            ("packages", graph::list(&release.packages)),
        ]),
    )];
    let mut edges = Vec::new();
    for (n, part) in release.parts.iter().enumerate() {
        let key = move_key(&release.id, &part.part);
        nodes.push(node(
            PART_MOVE,
            &key,
            props([
                ("release", json!(release.id)),
                ("n", json!(n as u64)),
                ("part", json!(part.part)),
                ("from", json!(part.from)),
                ("to", json!(part.to)),
                ("carries", graph::list(&part.carries)),
            ]),
        ));
        edges.push(edge((RELEASE, &release.id), MOVES, at(PART_MOVE, &key)));
    }
    for package in shipped {
        edges.push(edge((RELEASE, &release.id), SHIPS, at(SOAK, package)));
    }
    (nodes, edges)
}

/// A release without its part moves ([`move_from`] gives those).
pub fn release_from(props: &Map<String, Value>) -> Release {
    Release {
        id: graph::text(props, "id"),
        repo: graph::text(props, "repo"),
        ms: graph::number(props, "ms"),
        parts: Vec::new(),
        packages: graph::texts(props, "packages"),
        note: graph::text(props, "note"),
        state: graph::text(props, "state"),
        tail: graph::texts(props, "tail"),
        hotfix: graph::flag(props, "hotfix"),
    }
}

/// A part move: the release it belongs to, its place in that release, and the move.
pub fn move_from(props: &Map<String, Value>) -> (String, u64, PartMove) {
    let part = PartMove { part: graph::text(props, "part"), from: graph::text(props, "from"), to: graph::text(props, "to"), carries: graph::texts(props, "carries") };
    (graph::text(props, "release"), graph::number(props, "n"), part)
}

pub fn clock_node(repo: &str, clock: &Clock) -> GraphNode {
    node(CLOCK, repo, props([("repo", json!(repo)), ("ran_day", optional(&clock.ran_day)), ("skip_day", optional(&clock.skip_day))]))
}

pub fn clock_from(props: &Map<String, Value>) -> (String, Clock) {
    (graph::text(props, "repo"), Clock { ran_day: graph::text(props, "ran_day"), skip_day: graph::text(props, "skip_day") })
}
