//! `cc-train`: soak and the release train (contract `train@1`,
//! `docs/specs/02-HOLD-PACKAGES-RELEASE.md` §6-7).
//!
//! A landed package soaks in the owner's own apps. The daily train of its repository releases
//! every package that finished its soak without a report, through the repository's own release
//! scripts on its runner, and records what moved, the release note and the script's last lines.
//! A released package can be taken back: an urgent request that reverts it.

pub mod bump;
pub mod data;
pub mod graph;
pub mod records;
pub mod service;
pub mod when;

use cc_protocol::contracts::train::methods;

/// Methods this plugin serves.
pub const PROVIDES: &[&str] = &[methods::GET, methods::LANDED, methods::REPORT, methods::RUN, methods::SKIP, methods::LIST, methods::TAKE_BACK, methods::BUMP];
