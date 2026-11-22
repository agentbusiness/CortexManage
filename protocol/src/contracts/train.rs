//! `train@1`: soak and the release train (docs/specs/02-HOLD-PACKAGES-RELEASE.md §6-7). Part
//! versions, the daily train per repository, release notes, take back. CortexManage's own.

use serde::{Deserialize, Serialize};

pub mod methods {
    /// `{repo}` -> [`super::TrainView`]: tonight's train, what soaks, what was released.
    pub const GET: &str = "train.get";
    /// [`super::LandedParams`] -> `{ok}`: a package landed; it starts to soak (the manage plugin calls this).
    pub const LANDED: &str = "train.landed";
    /// [`super::ReportParams`] -> `{ok}`: something went wrong in a soaking package's part.
    pub const REPORT: &str = "train.report";
    /// `{repo}` -> [`super::Release`]: run the train now.
    pub const RUN: &str = "train.run";
    /// `{repo}` -> `{ok}`: skip tonight's train.
    pub const SKIP: &str = "train.skip";
    /// `{repo}` -> `{releases: [Release]}`.
    pub const LIST: &str = "train.list";
    /// `{package}` -> `{request}`: open the urgent request that reverts a released package.
    pub const TAKE_BACK: &str = "train.take_back";
    /// [`super::BumpParams`] -> `{version}`: the next version of a part for a tier.
    pub const BUMP: &str = "train.bump";
}

pub mod topics {
    /// `{repo}` after any change.
    pub const CHANGED: &str = "train.changed";
    /// A [`super::Release`], when a train ran.
    pub const RELEASED: &str = "train.released";
}

/// A landed package on its way to a release.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Soak {
    pub package: String,
    pub repo: String,
    /// `[request id, title, tier]`.
    pub requests: Vec<(String, String, String)>,
    pub parts: Vec<String>,
    /// The strictest tier in it.
    pub tier: String,
    pub urgent: bool,
    pub commit: String,
    pub landed_ms: u64,
    pub until_ms: u64,
    /// Reports during the soak; any holds it back from the train.
    #[serde(default)]
    pub reports: Vec<String>,
    /// soaking | ready | held_back | released | taken_back.
    pub state: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PartMove {
    pub part: String,
    pub from: String,
    pub to: String,
    /// The requests it carries, by title.
    #[serde(default)]
    pub carries: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub id: String,
    pub repo: String,
    pub ms: u64,
    pub parts: Vec<PartMove>,
    pub packages: Vec<String>,
    /// One line per request, grouped by part.
    pub note: String,
    /// ran | failed | skipped | nothing.
    pub state: String,
    /// The release script's last lines.
    #[serde(default)]
    pub tail: Vec<String>,
    #[serde(default)]
    pub hotfix: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TrainView {
    pub repo: String,
    /// When the next train leaves (ms).
    pub next_ms: u64,
    /// What it would carry now.
    pub next: Vec<PartMove>,
    pub soaking: Vec<Soak>,
    pub released: Vec<Release>,
    /// What a person said the next train waits for and is not ready yet (the train does not
    /// leave until it is).
    #[serde(default)]
    pub waits: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LandedParams {
    pub package: String,
    pub repo: String,
    pub requests: Vec<(String, String, String)>,
    pub parts: Vec<String>,
    pub tier: String,
    pub urgent: bool,
    pub commit: String,
    /// How long it soaks (minutes); 0 for urgent.
    pub soak_min: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ReportParams {
    pub repo: String,
    /// The part the report names.
    pub part: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BumpParams {
    /// `0.4.12`.
    pub version: String,
    /// safe | minor | major.
    pub tier: String,
    /// A major change whose contract lost or changed a field.
    #[serde(default)]
    pub breaking: bool,
}
