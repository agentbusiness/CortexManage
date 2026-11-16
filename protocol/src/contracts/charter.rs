//! `charter@1`: every app's design specification (docs/specs/03-CHARTER.md). Laws, constraints and
//! conventions; proposals; the printed `AGENTS.md`; the checks kept by code. CortexManage's own.

use serde::{Deserialize, Serialize};

pub mod methods {
    /// `{repo}` -> [`super::Charter`]. `repo` may be `standard` for the standard charter.
    pub const GET: &str = "charter.get";
    /// `{}` -> `{charters: [{repo, version, rules, draft}]}`.
    pub const LIST: &str = "charter.list";
    /// [`super::ImportParams`] -> [`super::Charter`]: a draft made from the given text (an
    /// `AGENTS.md`), one rule per paragraph or list item, each with the lines it came from.
    pub const IMPORT: &str = "charter.import";
    /// `{repo}` -> [`super::Charter`]: the draft becomes version 1.0.
    pub const ACCEPT: &str = "charter.accept";
    /// [`super::RulePut`] -> [`super::Charter`]: add or change a rule of a draft, or (with
    /// `confirm` equal to the rule's id) edit a law of an accepted charter by hand.
    pub const RULE_EDIT: &str = "charter.rule.edit";
    /// `{repo, id}` -> [`super::Charter`]: retire a rule of a draft.
    pub const RULE_DELETE: &str = "charter.rule.delete";
    /// [`super::ProposeParams`] -> [`super::Proposal`]: old and new text, conflicts, the tier the
    /// change needs. Nothing is changed.
    pub const PROPOSE: &str = "charter.propose";
    /// [`super::AmendParams`] -> [`super::Charter`]: apply an approved proposal (the caller is the
    /// manage plugin, when the amendment's request lands). Refused when `digest` is not the
    /// proposal's.
    pub const AMEND: &str = "charter.amend";
    /// `{repo}` -> [`super::Printed`]: the `AGENTS.md` text for the repository.
    pub const PRINT: &str = "charter.print";
    /// [`super::CheckParams`] -> [`super::CheckAnswer`]: run the checks kept by code over a diff.
    pub const CHECK: &str = "charter.check";
    /// [`super::RulesForParams`] -> `{rules: [Rule]}`: the rules that apply to these paths.
    pub const RULES_FOR: &str = "charter.rules_for";
    /// [`super::DriftParams`] -> [`super::DriftAnswer`]: compare a repository's `AGENTS.md` text
    /// with what the charter prints.
    pub const DRIFT: &str = "charter.drift";
}

pub mod topics {
    /// `{repo, version}` after any change.
    pub const CHANGED: &str = "charter.changed";
}

pub mod level {
    pub const LAW: &str = "law";
    pub const CONSTRAINT: &str = "constraint";
    pub const CONVENTION: &str = "convention";
}

pub mod kept_by {
    pub const CHECK: &str = "check";
    pub const VALIDATOR: &str = "validator";
    pub const TEXT: &str = "text";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    /// `L3`, `C12`, `V7`: stable.
    pub id: String,
    /// law | constraint | convention.
    pub level: String,
    /// `standard` or the repository's id.
    pub scope: String,
    pub text: String,
    /// The reason, for people.
    #[serde(default)]
    pub why: String,
    /// Path globs; empty: everywhere.
    #[serde(default)]
    pub applies_to: Vec<String>,
    /// check | validator | text.
    #[serde(default)]
    pub kept_by: String,
    /// The check's name, when kept by a check.
    #[serde(default)]
    pub check: String,
    /// The charter version it last changed in.
    #[serde(default)]
    pub since: String,
    /// The request that last changed it.
    #[serde(default)]
    pub amended_by: String,
    /// Where an imported rule came from (`AGENTS.md:41-44`).
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Identity {
    /// What the app is, one paragraph.
    #[serde(default)]
    pub what: String,
    #[serde(default)]
    pub who_for: String,
    /// What it must never become, one line each.
    #[serde(default)]
    pub never: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Charter {
    pub repo: String,
    /// `1.4`; `0.0` while a draft.
    pub version: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub identity: Identity,
    pub rules: Vec<Rule>,
    /// The standard charter's rules (laws and constraints), for an app's charter.
    #[serde(default)]
    pub standard: Vec<Rule>,
    #[serde(default)]
    pub standard_version: String,
    /// sha256 of the charter's content.
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub updated_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportParams {
    pub repo: String,
    /// The file's name (`AGENTS.md`) and its text.
    pub file: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RulePut {
    pub repo: String,
    /// Empty: a new rule (the id is given from its level).
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub applies_to: Option<Vec<String>>,
    #[serde(default)]
    pub kept_by: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    /// For a law of an accepted charter: the rule's id, typed by the owner.
    #[serde(default)]
    pub confirm: String,
    /// Identity fields, when the edit is to the identity.
    #[serde(default)]
    pub identity: Option<Identity>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProposeParams {
    pub repo: String,
    /// The rule to change; empty for a new rule.
    #[serde(default)]
    pub id: String,
    /// For a new rule.
    #[serde(default)]
    pub level: String,
    pub text: String,
    #[serde(default)]
    pub why: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    pub repo: String,
    pub id: String,
    pub level: String,
    pub old: String,
    pub new: String,
    #[serde(default)]
    pub why: String,
    /// `law`: only the owner, by hand. `major` | `minor`: the tier of the request that changes it.
    pub needs: String,
    /// Rules it may conflict with (found by code: same paths, opposite words).
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// sha256 of `repo`, `id` and `new`: an approval is bound to it.
    pub digest: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AmendParams {
    pub proposal: Proposal,
    pub digest: String,
    /// The request that carries it.
    pub request: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Printed {
    pub repo: String,
    pub version: String,
    /// The whole file.
    pub text: String,
    /// sha256 of `text` without its banner line.
    pub digest: String,
    /// How many characters the laws and constraints take (they must fit in 6000).
    pub head_chars: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckParams {
    pub repo: String,
    /// Unified diff.
    pub diff: String,
    /// The files it changes (when the diff is cut).
    #[serde(default)]
    pub files: Vec<String>,
    /// True for a standard change: the standard's paths may change.
    #[serde(default)]
    pub standard_change: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckFinding {
    pub check: String,
    /// The rule it keeps.
    #[serde(default)]
    pub rule: String,
    pub file: String,
    #[serde(default)]
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckAnswer {
    pub ok: bool,
    /// The checks that ran.
    pub ran: Vec<String>,
    pub findings: Vec<CheckFinding>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RulesForParams {
    pub repo: String,
    pub paths: Vec<String>,
    /// Only these levels (empty: laws and constraints).
    #[serde(default)]
    pub levels: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DriftParams {
    pub repo: String,
    /// The repository's `AGENTS.md` as it is.
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DriftAnswer {
    /// same | edited | not_printed (the file carries no banner).
    pub state: String,
    /// Lines only in the file, and lines only in the print.
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}
