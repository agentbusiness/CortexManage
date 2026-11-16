//! `cc-charter`: every app's design specification (contract `charter@1`, docs/specs/03-CHARTER.md).
//!
//! A charter is an app's laws, constraints and conventions, with what the app is. It lives in the
//! graph, never in the repository it governs; the repository only holds the print (`AGENTS.md`).
//! This plugin keeps the charters, imports a first draft from an existing file, takes the owner's
//! edits, turns a wished change into a proposal bound to its exact wording, applies approved
//! amendments, prints the file, and runs the checks that code can keep over a diff.

pub mod book;
pub mod checks;
pub mod data;
pub mod glob;
pub mod graph;
pub mod import;
pub mod print;
pub mod service;

use cc_protocol::contracts::charter::methods;

/// Methods this plugin serves.
pub const PROVIDES: &[&str] = &[
    methods::GET,
    methods::LIST,
    methods::IMPORT,
    methods::ACCEPT,
    methods::RULE_EDIT,
    methods::RULE_DELETE,
    methods::PROPOSE,
    methods::AMEND,
    methods::PRINT,
    methods::CHECK,
    methods::RULES_FOR,
    methods::DRIFT,
];
