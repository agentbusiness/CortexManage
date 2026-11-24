//! Every model-facing text and setting of the flow, as data: `stages.toml` (per stage: the model
//! tag it claims, its token limit, its prompt and schema; the tool loop's limits; sampling),
//! `texts.toml` (short templates with `{placeholders}`), `prompts/*.md` and `schemas/*.json`.
//!
//! Two layers, per file: the shipped file compiled into the binary (always valid), and the file of
//! the same name in the live folder (`<live>/backend/data/`), where it is edited while the app
//! runs. The set is read again for every stage call, so an edit is in use at the next question. A
//! live file that does not validate is left out (the shipped one stays in use) and named in
//! [`Data::problems`], which the probe shows.

use crate::schemas::{self, Shape};
use cc_protocol::contracts::model::Sampling;
use cc_protocol::{PluginEnv, RpcError};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The shipped files (relative path, content).
const EMBEDDED: &[(&str, &str)] = &[
    ("stages.toml", include_str!("../data/stages.toml")),
    ("texts.toml", include_str!("../data/texts.toml")),
    ("prompts/assess.md", include_str!("../data/prompts/assess.md")),
    ("prompts/match.md", include_str!("../data/prompts/match.md")),
    ("prompts/discover.md", include_str!("../data/prompts/discover.md")),
    ("prompts/plan.md", include_str!("../data/prompts/plan.md")),
    ("prompts/plan_fields.md", include_str!("../data/prompts/plan_fields.md")),
    ("prompts/validate.md", include_str!("../data/prompts/validate.md")),
    ("prompts/drift.md", include_str!("../data/prompts/drift.md")),
    ("prompts/converge.md", include_str!("../data/prompts/converge.md")),
    ("prompts/digest.md", include_str!("../data/prompts/digest.md")),
    ("prompts/why.md", include_str!("../data/prompts/why.md")),
    ("prompts/reply.md", include_str!("../data/prompts/reply.md")),
    ("prompts/delivery_read.md", include_str!("../data/prompts/delivery_read.md")),
    ("prompts/delivery.md", include_str!("../data/prompts/delivery.md")),
    ("prompts/commit.md", include_str!("../data/prompts/commit.md")),
    ("prompts/rules.md", include_str!("../data/prompts/rules.md")),
    ("schemas/assess.json", include_str!("../data/schemas/assess.json")),
    ("schemas/match.json", include_str!("../data/schemas/match.json")),
    ("schemas/plan.json", include_str!("../data/schemas/plan.json")),
    ("schemas/validate.json", include_str!("../data/schemas/validate.json")),
    ("schemas/drift.json", include_str!("../data/schemas/drift.json")),
    ("schemas/converge.json", include_str!("../data/schemas/converge.json")),
    ("schemas/why.json", include_str!("../data/schemas/why.json")),
    ("schemas/delivery.json", include_str!("../data/schemas/delivery.json")),
    ("schemas/rules.json", include_str!("../data/schemas/rules.json")),
];

/// The stages the flow runs; each must be in `stages.toml`.
pub const STAGES: &[&str] = &["ASSESS", "MATCH", "DISCOVER", "PLAN", "VALIDATE", "DRIFT", "CONVERGE", "DIGEST", "WHY", "REPLY", "DELIVERY_READ", "DELIVERY", "COMMIT", "RULES"];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStages {
    label: String,
    sampling: BTreeMap<String, RawSampling>,
    limits: Limits,
    stage: BTreeMap<String, StageConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSampling {
    temperature: f32,
    top_p: f32,
    top_k: u32,
    #[serde(default)]
    min_p: f32,
    #[serde(default)]
    presence_penalty: f32,
}

fn delivery_tool_calls() -> u32 {
    16
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Tool calls PLAN's discovery may make before the plan is written.
    pub plan_tool_calls: u32,
    /// Tool calls learning how a repository ships may make.
    #[serde(default = "delivery_tool_calls")]
    pub delivery_tool_calls: u32,
    /// Most characters of one tool result the model is shown (head and tail).
    pub tool_result_chars: usize,
    /// Most characters of one tool result carried into the plan's own question.
    pub plan_evidence_chars: usize,
    /// Most tasks in a plan.
    pub plan_max_tasks: usize,
    /// Most characters of a diff a question carries.
    pub diff_chars: usize,
    /// Lines `read` gives when the model names none, and hits `search` gives.
    pub read_lines: u32,
    pub search_hits: u32,
    /// How long a call the model plugin refused (restarting, busy) is tried again, in seconds.
    pub refused_window_s: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageConfig {
    /// The model tag the stage claims (`mesh.claim`).
    pub tag: String,
    /// The prompt files that make its system message, in order (`prompts/<name>.md`).
    pub prompts: Vec<String>,
    /// Its answer's schema (`schemas/<name>.json`); none: free text.
    #[serde(default)]
    pub schema: Option<String>,
    /// Most tokens of its answer.
    pub max_tokens: u32,
    /// The sampling profile (`[sampling.<name>]`).
    pub sampling: String,
    /// Thinking before the answer: `none` for every constrained stage (the grammar holds the whole
    /// generation, so a thinking block would take the decision's place).
    #[serde(default = "no_effort")]
    pub effort: String,
    /// Most thinking tokens when `effort` is not `none`.
    #[serde(default)]
    pub think_tokens: u32,
}

fn no_effort() -> String {
    "none".to_owned()
}

/// What each constrained stage's schema must carry for the flow to read its answer.
const SCHEMA_NEEDS: &[(&str, &[&str])] = &[
    ("assess", &["because", "kind", "surface", "major_because", "tier", "urgent_because", "questions", "title"]),
    ("match", &["because", "relation"]),
    ("plan", &["analysis", "goal", "approach", "architecture", "risks", "tasks", "note"]),
    ("validate", &["necessary", "sensible", "fits", "provable", "reason"]),
    ("drift", &["file", "line", "reason", "does_it"]),
    ("converge", &["extra", "missing"]),
    ("why", &["why", "if_yes", "if_no"]),
    ("delivery", &["facts"]),
    ("rules", &["rule", "file", "quote", "reason", "breaks"]),
];

/// One loaded, valid set.
pub struct Data {
    label: String,
    sampling: BTreeMap<String, Sampling>,
    pub limits: Limits,
    stages: BTreeMap<String, StageConfig>,
    texts: BTreeMap<String, String>,
    prompts: BTreeMap<String, String>,
    shapes: BTreeMap<String, Shape>,
    /// Live files left out, each with why.
    pub problems: Vec<String>,
}

/// The `{name}` placeholders of a text.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else { break };
        let name = &rest[..close];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            found.insert(name.to_owned());
        }
        rest = &rest[close + 1..];
    }
    found
}

/// A prompt file without its front matter (`---` … `---`), which only names it.
fn body(text: &str) -> Result<String, String> {
    let text = text.trim_start();
    let body = match text.strip_prefix("---") {
        Some(rest) => rest.split_once("\n---").map(|(_, body)| body).ok_or("the front matter is not closed")?,
        None => text,
    };
    let body = body.trim();
    if body.is_empty() {
        return Err("the prompt is empty".into());
    }
    Ok(body.to_owned())
}

fn parse_stages(text: &str) -> Result<RawStages, String> {
    let raw: RawStages = toml::from_str(text).map_err(|error| error.to_string())?;
    for stage in STAGES {
        let config = raw.stage.get(*stage).ok_or_else(|| format!("[stage.{stage}] is missing"))?;
        if !raw.sampling.contains_key(&config.sampling) {
            return Err(format!("[stage.{stage}] names the sampling profile `{}`, which is not there", config.sampling));
        }
        if config.tag.trim().is_empty() || config.prompts.is_empty() || config.max_tokens == 0 {
            return Err(format!("[stage.{stage}] needs a tag, a prompt and a token limit"));
        }
    }
    if raw.limits.plan_max_tasks == 0 {
        return Err("limits.plan_max_tasks must be at least 1".into());
    }
    Ok(raw)
}

fn parse_shape(name: &str, text: &str) -> Result<Shape, String> {
    let schema: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let shape = schemas::shape_from_schema(&schema)?;
    if let Some((_, needs)) = SCHEMA_NEEDS.iter().find(|(known, _)| *known == name) {
        for field in *needs {
            if shape.field(field).is_none() {
                return Err(format!("the property `{field}` is missing"));
            }
        }
    }
    Ok(shape)
}

impl Data {
    /// The shipped set alone.
    pub fn embedded() -> Result<Self, String> {
        Self::build(|_| None)
    }

    /// The shipped set with the live folder's valid files over it.
    pub fn load(env: &PluginEnv) -> Result<Self, String> {
        let live = env.live_dir.as_ref().map(|dir| dir.join("backend").join("data"));
        Self::build(|file| {
            let path = live.as_ref()?.join(file);
            path.is_file().then(|| std::fs::read_to_string(&path).map_err(|error| error.to_string()))
        })
    }

    /// Build the set; `live` gives a file's live text when there is one.
    fn build(live: impl Fn(&str) -> Option<Result<String, String>>) -> Result<Self, String> {
        let problems = std::cell::RefCell::new(Vec::new());
        let shipped = |file: &str| EMBEDDED.iter().find(|(name, _)| *name == file).map(|(_, text)| *text).unwrap_or("");
        // Each file: the live text when it validates, else the shipped one (which must).
        let pick = |file: &str, valid: &dyn Fn(&str) -> Result<(), String>| -> Result<String, String> {
            if let Some(read) = live(file) {
                match read.and_then(|text| valid(&text).map(|()| text)) {
                    Ok(text) => return Ok(text),
                    Err(error) => problems.borrow_mut().push(format!("live {file} is not in use: {error}")),
                }
            }
            valid(shipped(file)).map_err(|error| format!("shipped {file}: {error}"))?;
            Ok(shipped(file).to_owned())
        };

        let stages_text = pick("stages.toml", &|text| parse_stages(text).map(|_| ()))?;
        let raw = parse_stages(&stages_text)?;

        let shipped_texts: BTreeMap<String, String> = toml::from_str(shipped("texts.toml")).map_err(|error| format!("shipped texts.toml: {error}"))?;
        let texts_text = pick("texts.toml", &|text| toml::from_str::<BTreeMap<String, String>>(text).map(|_| ()).map_err(|error| error.to_string()))?;
        let mut texts: BTreeMap<String, String> = toml::from_str(&texts_text).map_err(|error| error.to_string())?;
        // A text keeps exactly the placeholders it is shipped with: the host fills those and no others.
        for (key, original) in &shipped_texts {
            let keeps = texts.get(key).is_some_and(|text| placeholders(text) == placeholders(original));
            if !keeps {
                if texts.contains_key(key) {
                    problems.borrow_mut().push(format!("live texts.toml `{key}` is not in use: its placeholders differ from the shipped text's"));
                }
                texts.insert(key.clone(), original.clone());
            }
        }

        let mut prompts = BTreeMap::new();
        let mut shapes = BTreeMap::new();
        for (file, _) in EMBEDDED {
            if let Some(name) = file.strip_prefix("prompts/").and_then(|name| name.strip_suffix(".md")) {
                let text = pick(file, &|text| body(text).map(|_| ()))?;
                prompts.insert(name.to_owned(), body(&text)?);
            } else if let Some(name) = file.strip_prefix("schemas/").and_then(|name| name.strip_suffix(".json")) {
                let text = pick(file, &|text| parse_shape(name, text).map(|_| ()))?;
                shapes.insert(name.to_owned(), parse_shape(name, &text)?);
            }
        }
        for (stage, config) in &raw.stage {
            for prompt in &config.prompts {
                if !prompts.contains_key(prompt) {
                    return Err(format!("[stage.{stage}] names the prompt `{prompt}`, which is not there"));
                }
            }
            if let Some(schema) = &config.schema
                && !shapes.contains_key(schema)
            {
                return Err(format!("[stage.{stage}] names the schema `{schema}`, which is not there"));
            }
        }
        let sampling = raw
            .sampling
            .into_iter()
            .map(|(name, raw)| {
                let sampling = Sampling { temperature: raw.temperature, top_p: raw.top_p, top_k: raw.top_k, min_p: raw.min_p, presence_penalty: raw.presence_penalty, repeat_penalty: None, seed: None };
                (name, sampling)
            })
            .collect();
        Ok(Self { label: raw.label, sampling, limits: raw.limits, stages: raw.stage, texts, prompts, shapes, problems: problems.into_inner() })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn stage(&self, name: &str) -> Result<&StageConfig, RpcError> {
        self.stages.get(name).ok_or_else(|| RpcError::internal(format!("the stage {name} is not configured")))
    }

    /// A stage's system message: its prompt files, in order.
    pub fn system(&self, stage: &StageConfig) -> String {
        stage.prompts.iter().filter_map(|name| self.prompts.get(name)).cloned().collect::<Vec<_>>().join("\n\n")
    }

    /// A stage's answer shape (None: free text).
    pub fn shape(&self, stage: &StageConfig) -> Option<&Shape> {
        stage.schema.as_ref().and_then(|name| self.shapes.get(name))
    }

    pub fn sampling(&self, stage: &StageConfig) -> Sampling {
        self.sampling.get(&stage.sampling).cloned().unwrap_or_default()
    }

    /// A short text with its placeholders filled, in one pass: a value that itself holds `{name}`
    /// (a request's text can hold anything) is never filled in turn.
    pub fn text(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let template = self.texts.get(key).map(String::as_str).unwrap_or("");
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let filled = after.find('}').and_then(|close| vars.iter().find(|(name, _)| *name == &after[..close]).map(|(_, value)| (close, *value)));
            match filled {
                Some((close, value)) => {
                    out.push_str(value);
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// How many prompts and schemas the set holds (for the probe's line).
    pub fn counts(&self) -> (usize, usize) {
        (self.prompts.len(), self.shapes.len())
    }
}
