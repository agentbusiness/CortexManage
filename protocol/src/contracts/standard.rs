//! `standard@1`: one mesh standard for every Cortex app (docs/specs/04-STANDARD.md). The lock, the
//! checks against it, the view. CortexManage's own.

use serde::{Deserialize, Serialize};

pub mod methods {
    /// `{}` -> [`super::Lock`]: the lock as it is kept (empty before the first `lock.write`).
    pub const LOCK_GET: &str = "standard.lock.get";
    /// [`super::LockWriteParams`] -> [`super::Lock`]: make the lock from one member's copies.
    pub const LOCK_WRITE: &str = "standard.lock.write";
    /// `{}` -> `{text}`: the lock as the `standard.lock` file's text.
    pub const LOCK_PRINT: &str = "standard.lock.print";
    /// `{}` -> `{members: [Member]}`.
    pub const MEMBER_LIST: &str = "standard.member.list";
    /// [`super::Member`] -> `{members}`: add or change a member (its folders, its priority, where
    /// its checkout is on this computer).
    pub const MEMBER_EDIT: &str = "standard.member.edit";
    /// `{}` -> [`super::Report`]: hash every standard file of every member with a checkout here.
    pub const CHECK: &str = "standard.check";
    /// `{}` -> [`super::Report`]: the last report, without checking again.
    pub const GET: &str = "standard.get";
    /// [`super::DiffParams`] -> `{paths: [string]}`: which of these changed paths are standard files.
    pub const TOUCHES: &str = "standard.touches";
    /// `{}` -> `{paths: [string]}`: every standard path, as globs relative to a repository root,
    /// for a member (`{member}`).
    pub const PATHS: &str = "standard.paths";
    /// `{}` -> [`super::Handshake`]: ask every member on the mesh the standard's read methods.
    pub const HANDSHAKE: &str = "standard.handshake";
}

pub mod topics {
    /// `{version, drift}` after a check or a lock change.
    pub const CHANGED: &str = "standard.changed";
}

/// A member of the standard and where its copy lives.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Member {
    /// The `software` id (`cortexterm`).
    pub id: String,
    /// The name for people (`CortexTerm`).
    pub name: String,
    pub priority: u32,
    /// Folder of each part relative to the member's root (`protocol` -> `protocol`,
    /// `mesh` -> `plugins/mesh/backend/src`).
    pub folders: std::collections::BTreeMap<String, String>,
    /// Parts the member does not carry, by decision.
    #[serde(default)]
    pub without: Vec<String>,
    /// Left out for now: listed, never checked.
    #[serde(default)]
    pub deferred: bool,
    /// The member's checkout on this computer (empty: not here).
    #[serde(default)]
    pub root: String,
    /// The names the member uses for itself, replaced by placeholders before a `names` file is hashed.
    #[serde(default)]
    pub names: Vec<String>,
}

/// One standard file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LockFile {
    /// protocol | mesh | service-kit | sandbox-kit | model-kit | shared-models | memory | theme | charter.
    pub part: String,
    /// Relative to the member's folder for the part.
    pub path: String,
    pub sha256: String,
    /// Hashed after the member's own names are replaced.
    #[serde(default)]
    pub names: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Lock {
    /// `1.08`.
    pub version: String,
    /// The member the lock was made from.
    #[serde(default)]
    pub from: String,
    pub files: Vec<LockFile>,
    #[serde(default)]
    pub written_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LockWriteParams {
    /// The member whose copies become the standard.
    pub from: String,
    /// The new version; empty: the last one's minor plus one.
    #[serde(default)]
    pub version: String,
}

/// One cell of the view: a part of a member against the lock.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cell {
    pub member: String,
    pub part: String,
    /// same | names (same once names are replaced) | differs | missing | without | deferred | not_here.
    pub state: String,
    /// The files that differ or are missing (at most 20).
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub version: String,
    pub parts: Vec<String>,
    pub members: Vec<Member>,
    pub cells: Vec<Cell>,
    /// Members with at least one part that differs or is missing.
    pub drift: Vec<String>,
    pub checked_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DiffParams {
    /// The member whose repository the paths are in.
    pub member: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HandshakeRow {
    pub node: String,
    pub name: String,
    pub software: String,
    /// answers | fails | unreachable.
    pub state: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Handshake {
    pub rows: Vec<HandshakeRow>,
    pub ms: u64,
}
