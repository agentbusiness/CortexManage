//! `cc-standard`: one mesh standard for every Cortex app (contract `standard@1`,
//! `docs/specs/04-STANDARD.md`).
//!
//! It keeps the members of the standard and the lock in the graph, makes the lock from one
//! member's copies, checks every member's copies on this computer against it, says which paths of
//! a repository are standard files, and asks the members on the mesh for their card.

pub mod graph;
pub mod lock;
pub mod parts;
pub mod service;

use cc_protocol::contracts::standard::methods;

/// Methods this plugin serves.
pub const PROVIDES: &[&str] = &[
    methods::LOCK_GET,
    methods::LOCK_WRITE,
    methods::LOCK_PRINT,
    methods::MEMBER_LIST,
    methods::MEMBER_EDIT,
    methods::CHECK,
    methods::GET,
    methods::TOUCHES,
    methods::PATHS,
    methods::HANDSHAKE,
];
