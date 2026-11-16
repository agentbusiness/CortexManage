//! `cc-manage`: CortexManage's lifecycle engine (contract `manage@1`,
//! docs/specs/05-MANAGE-CONTRACT.md). A request is assessed, planned and validated, waits in a
//! package for its hold to end, is built by a runner under the watch, proven by the gates, landed
//! and handed to the release train. The plugin never edits code: a CortexCode on a runner does.

pub mod diff;
pub mod gates;
pub mod graph;
pub mod inbox;
pub mod job;
pub mod link;
pub mod packages;
pub mod pipeline;
pub mod policy;
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
    methods::SHARES,
];

/// Event topics this plugin wants: plugins coming (back) into service, and trains that ran.
pub const SUBSCRIBES: &[&str] = &["plugin.state", "plugin.swapped", "train.released"];
