//! `cc-manage`: CortexManage's lifecycle engine (contract `manage@1`,
//! docs/specs/05-MANAGE-CONTRACT.md). A request is assessed, planned and validated, waits in a
//! package for its hold to end, is built by a runner under the watch, proven by the gates, landed
//! and handed to the release train. The plugin never edits code: a CortexCode on a runner does.

pub mod board;
pub mod delivery;
pub mod diff;
pub mod gates;
pub mod graph;
pub mod inbox;
pub mod job;
pub mod ledger;
pub mod link;
pub mod packages;
pub mod pipeline;
pub mod policy;
pub mod release;
pub mod reports;
pub mod service;
pub mod store;
pub mod texts;
pub mod words;

use cc_protocol::contracts::manage::methods;

/// Methods this plugin serves.
pub const PROVIDES: &[&str] = &[
    methods::REPO_LIST,
    methods::REPO_GET,
    methods::REPO_CREATE,
    methods::REPO_EDIT,
    methods::REQUEST_CREATE,
    methods::REQUEST_GET,
    methods::REQUEST_LIST,
    methods::REQUEST_EDIT,
    methods::REQUEST_SAY,
    methods::PACKAGE_GET,
    methods::PACKAGE_LIST,
    methods::PACKAGE_EDIT,
    methods::DECIDE,
    methods::INBOX_LIST,
    methods::HOLD_GET,
    methods::HOLD_EDIT,
    methods::EVIDENCE_READ,
    methods::DISCOVER_READ,
    methods::DISCOVER_SEARCH,
    methods::DISCOVER_TREE,
    methods::RUNNER_RELEASE,
    methods::RUNNER_GET,
    methods::BOARD_ITEMS,
    methods::SHARES,
    methods::TIMELINE_READ,
    methods::REQUEST_REOPEN,
    methods::REQUEST_STORY,
    methods::DELIVERY_LEARN,
    methods::DELIVERY_EDIT,
    methods::RELEASE_WAITS,
    methods::RULES_GET,
    methods::RULES_EDIT,
    cc_protocol::contracts::reports::methods::SEND,
];

/// Event topics this plugin wants: plugins coming (back) into service, and trains that ran.
pub const SUBSCRIBES: &[&str] = &["plugin.state", "plugin.swapped", "train.released"];
