//! The plugin's data: the texts a person reads (`data/texts.toml`) and the checks kept by code
//! (`data/checks.toml`). The shipped copies are compiled in; the live copies (`backend/data/` in
//! the plugin's live folder) are read on every use, so an edit there takes effect at once. A live
//! file that does not parse, or a text that drops a placeholder, is ignored.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

const TEXTS: &str = include_str!("../data/texts.toml");
const CHECKS: &str = include_str!("../data/checks.toml");

fn placeholders(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = text.split('{').skip(1).filter_map(|part| part.split_once('}').map(|(name, _)| name)).collect();
    out.sort_unstable();
    out
}

pub fn fill(text: &str, vars: &[(&str, &str)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

pub struct Texts {
    live: Option<PathBuf>,
    shipped: BTreeMap<String, String>,
}

impl Texts {
    pub fn new(live_dir: Option<PathBuf>) -> Self {
        Self { live: live_dir.map(|dir| dir.join("backend").join("data").join("texts.toml")), shipped: toml::from_str(TEXTS).unwrap_or_default() }
    }

    /// The text `key` with its `{name}`s filled in.
    pub fn get(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let shipped = self.shipped.get(key).cloned().unwrap_or_default();
        let live = self
            .live
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str::<BTreeMap<String, String>>(&text).ok())
            .and_then(|table| table.get(key).cloned())
            .filter(|text| placeholders(text) == placeholders(&shipped));
        fill(&live.unwrap_or(shipped), vars)
    }
}

/// One check as a person reads it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CheckText {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub keeps: String,
    pub finding: String,
    /// Why the check never runs (empty: it runs).
    #[serde(default)]
    pub not_run: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PrintedFiles {
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Token {
    pub starts: String,
    pub tail: usize,
    pub what: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Secrets {
    pub header_start: String,
    pub header_end: String,
    pub names: Vec<String>,
    pub value_chars: usize,
    #[serde(default)]
    pub token: Vec<Token>,
}

/// `data/checks.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Checks {
    pub check: Vec<CheckText>,
    pub printed: PrintedFiles,
    pub secrets: Secrets,
}

impl Checks {
    pub fn text(&self, id: &str) -> Option<&CheckText> {
        self.check.iter().find(|check| check.id == id)
    }
}

pub struct Data {
    pub texts: Texts,
    live_checks: Option<PathBuf>,
}

impl Data {
    pub fn new(live_dir: Option<PathBuf>) -> Self {
        Self { live_checks: live_dir.as_ref().map(|dir| dir.join("backend").join("data").join("checks.toml")), texts: Texts::new(live_dir) }
    }

    /// The checks' data: the live file when it parses whole, else the shipped one.
    pub fn checks(&self) -> Result<Checks, String> {
        if let Some(live) = self.live_checks.as_ref().and_then(|path| std::fs::read_to_string(path).ok())
            && let Ok(checks) = toml::from_str::<Checks>(&live)
        {
            return Ok(checks);
        }
        toml::from_str(CHECKS).map_err(|error| format!("the shipped checks.toml does not parse: {error}"))
    }
}
