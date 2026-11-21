//! The manager's questions (`manage::flow`), one model call each: the stage's prompt as the system
//! message, the evidence as the user message with the step to take as its last line, the answer
//! held to the stage's schema and decoded greedily, in a context of its own.
//!
//! No answer is taken on trust. Each is read against its schema and then checked by code (a quoted
//! sentence must be in the request, a yes must name a line, a verdict needs a reason). An answer
//! that fails is asked once more, with what was wrong; after that the cautious choice stands
//! (`major`, `ask`, `no`, `unrelated`), or an error where no cautious answer exists.

use crate::data::Data;
use crate::flow::Flow;
use crate::llm::{self, Ask};
use crate::schemas::{self, FILES, POINTS, Shape};
use cc_protocol::contracts::manage::{
    AssessParams, AssessedParams, ConvergeAnswer, ConvergeParams, DriftAnswer, DriftParams, MatchAnswer, MatchParams, Question, Tier, ValidateAnswer, ValidateParams, WhyAnswer,
};
use cc_protocol::contracts::model::ChatMessage;
use cc_protocol::{RpcError, codes};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// One constrained question.
pub struct Asked<'a> {
    pub stage: &'a str,
    /// The evidence, in the order it is read.
    pub evidence: String,
    /// The step to take: the message's last line.
    pub step: String,
    pub shape: Shape,
}

/// Both asks failed: what was wrong with the second, and the last answer that had the schema's
/// shape (so a fallback can keep what it did say).
pub struct Unusable {
    pub problem: String,
    pub last: Option<Value>,
}

/// A model answer as JSON (the grammar gives nothing else; a borrowed engine might).
fn parse(content: &str) -> Result<Value, String> {
    serde_json::from_str(content.trim()).map_err(|_| "it was not one complete JSON object".to_owned())
}

/// Blocks of a message, a blank line between them; empty ones are left out.
pub fn blocks(parts: &[String]) -> String {
    parts.iter().map(|part| part.trim()).filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\n\n")
}

pub fn list(data: &Data, items: &[String]) -> String {
    items.iter().map(|item| item.trim()).filter(|item| !item.is_empty()).map(|item| data.text("list_item", &[("text", item)])).collect::<Vec<_>>().join("\n")
}

/// Letters, digits and single spaces, lower case: how a quote and its source are compared, so a
/// changed quote mark, line break or capital does not make a true quote fail.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for character in text.chars() {
        if character.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.extend(character.to_lowercase());
        } else {
            gap = true;
        }
    }
    out
}

/// Whether `quote` is words of `text`, in order and unbroken: at least three of them, or all of a
/// shorter text (one word is found in any text and shows nothing).
pub fn quoted(quote: &str, text: &str) -> bool {
    let (quote, text) = (plain(quote), plain(text));
    if quote.is_empty() {
        return false;
    }
    let found = format!(" {text} ").contains(&format!(" {quote} "));
    found && (quote.split(' ').count() >= 3 || quote == text)
}

/// The files a unified diff changes, as it names them (`+++ b/<path>`; a deleted file by its
/// `--- a/<path>`), in order.
pub fn diff_files(diff: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    let mut old: Option<String> = None;
    for line in diff.lines() {
        let name = |rest: &str, side: &str| {
            let path = rest.split('\t').next().unwrap_or(rest).trim();
            (path != "/dev/null" && !path.is_empty()).then(|| path.strip_prefix(side).unwrap_or(path).to_owned())
        };
        if let Some(rest) = line.strip_prefix("--- ") {
            old = name(rest, "a/");
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            if let Some(file) = name(rest, "b/").or(old.take())
                && !files.contains(&file)
            {
                files.push(file);
            }
            old = None;
        }
    }
    files
}

/// The diff with each added or kept line's number in the changed file before it, counted from the
/// hunk headers: the model reads a line's number instead of counting to it (it counts badly), and
/// the number it answers with is the one the caller looks up.
pub fn numbered(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len() + diff.len() / 8);
    // Lines still to come in the open hunk (old side, new side), and the next new line's number.
    let (mut old_left, mut new_left, mut next) = (0u64, 0u64, 0u64);
    for line in diff.lines() {
        if old_left == 0 && new_left == 0 {
            if let Some((old, new)) = hunk(line) {
                (old_left, new_left, next) = (old.1, new.1, new.0);
            }
            out.push_str(line);
        } else if line.starts_with('+') {
            out.push_str(&format!("{next:>5} {line}"));
            next += 1;
            new_left = new_left.saturating_sub(1);
        } else if line.starts_with('-') {
            out.push_str(&format!("      {line}"));
            old_left = old_left.saturating_sub(1);
        } else if line.starts_with('\\') {
            // "\ No newline at end of file": not a line of either side.
            out.push_str(&format!("      {line}"));
        } else {
            out.push_str(&format!("{next:>5} {line}"));
            next += 1;
            old_left = old_left.saturating_sub(1);
            new_left = new_left.saturating_sub(1);
        }
        out.push('\n');
    }
    out
}

/// `@@ -a,b +c,d @@` -> ((a, b), (c, d)); a missing count is 1.
fn hunk(line: &str) -> Option<((u64, u64), (u64, u64))> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let side = |text: &str| -> Option<(u64, u64)> {
        match text.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((text.parse().ok()?, 1)),
        }
    };
    Some((side(old)?, side(new)?))
}

/// A diff no longer than the data set allows, cut at a line's end with a line saying so.
fn fitted(data: &Data, diff: String) -> String {
    let most = data.limits.diff_chars;
    if diff.len() <= most {
        return diff;
    }
    let mut end = most;
    while !diff.is_char_boundary(end) {
        end -= 1;
    }
    let end = diff[..end].rfind('\n').map_or(end, |at| at + 1);
    let left = diff[end..].lines().count().to_string();
    format!("{}{}", &diff[..end], data.text("diff_cut", &[("lines", &left)]))
}

/// The facts of DIGEST and WHY as lines a model reads: `name: value`, a list's items joined.
pub fn facts(value: &Value) -> String {
    fn scalar(value: &Value) -> String {
        match value {
            Value::String(text) => text.clone(),
            Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join("; "),
            Value::Object(map) => map.iter().map(|(key, value)| format!("{} {}", key.replace('_', " "), scalar(value))).collect::<Vec<_>>().join(", "),
            other => other.to_string(),
        }
    }
    match value {
        Value::Object(map) => map
            .iter()
            .filter(|(_, value)| !value.is_null() && value.as_str() != Some(""))
            .map(|(key, value)| format!("- {}: {}", key.replace('_', " "), scalar(value)))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Array(items) => items.iter().map(|item| format!("- {}", scalar(item))).collect::<Vec<_>>().join("\n"),
        other => scalar(other),
    }
}

// ---- reading each stage's answer into the contract's type ----------------------------------------

fn text_of(value: &Value, field: &str) -> String {
    value.get(field).and_then(Value::as_str).unwrap_or("").trim().to_owned()
}

fn flag(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool).unwrap_or(false)
}

/// ASSESS: the answer's fields as the contract names them, with the question ids the host gives.
/// No check yet: [`checked_assess`] does that.
pub fn read_assess(value: &Value, request: &str) -> Result<AssessedParams, String> {
    let questions = value
        .get("questions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| !text_of(item, "text").is_empty())
                .enumerate()
                .map(|(index, item)| Question {
                    id: format!("q{}", index + 1),
                    text: text_of(item, "text"),
                    options: item.get("options").and_then(Value::as_array).map(|options| options.iter().filter_map(Value::as_str).map(|option| option.trim().to_owned()).filter(|option| !option.is_empty()).collect()).unwrap_or_default(),
                    multi: flag(item, "multi"),
                    answer: Vec::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut surface: Vec<String> = Vec::new();
    for part in value.get("surface").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if !surface.iter().any(|known| known == part) {
            surface.push(part.to_owned());
        }
    }
    Ok(AssessedParams {
        request: request.to_owned(),
        actionable: flag(value, "actionable"),
        questions,
        title: text_of(value, "title"),
        kind: text_of(value, "kind"),
        tier: Tier::parse(&text_of(value, "tier")).ok_or("`tier` must be safe, minor or major")?,
        surface,
        urgent: flag(value, "urgent"),
        because: text_of(value, "because"),
        urgent_because: text_of(value, "urgent_because"),
    })
}

/// ASSESS's host checks: both quotes must be in the request, and a request that cannot be acted on
/// must come with the questions that would make it so. Then what the host settles: no urgency
/// without its sentence, and questions only where the request waits for them.
fn checked_assess(mut answer: AssessedParams, text: &str) -> Result<AssessedParams, String> {
    if !quoted(&answer.because, text) {
        return Err("`because` is not a sentence of the request. Copy one sentence of the request word for word.".into());
    }
    if answer.urgent && !quoted(&answer.urgent_because, text) {
        return Err("`urgent_because` is not a sentence of the request. Copy one word for word, or leave it empty and set `urgent` to false.".into());
    }
    if !answer.actionable && answer.questions.is_empty() {
        return Err("`actionable` is false but `questions` is empty. Write the question, or set `actionable` to true.".into());
    }
    if !answer.urgent {
        answer.urgent_because.clear();
    }
    if answer.actionable {
        answer.questions.clear();
    }
    Ok(answer)
}

/// The cautious assessment when no answer could be read: minor, not urgent, to be acted on. Not
/// major: an unreadable answer is no reason to stop and ask a person; the plan's paths can still
/// raise it. What kind of work it is stays as
/// the model last said when it said anything readable; nothing is quoted, because no quote held.
fn cautious_assess(last: Option<&Value>, request: &str) -> AssessedParams {
    let said = last.and_then(|value| read_assess(value, request).ok()).unwrap_or_default();
    AssessedParams { request: request.to_owned(), actionable: true, questions: Vec::new(), title: said.title, kind: said.kind, tier: Tier::Minor, surface: said.surface, urgent: false, because: String::new(), urgent_because: String::new() }
}

pub fn read_match(value: &Value) -> Result<MatchAnswer, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

pub fn read_validate(value: &Value) -> Result<ValidateAnswer, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

pub fn read_drift(value: &Value) -> Result<DriftAnswer, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

pub fn read_why(value: &Value) -> Result<WhyAnswer, String> {
    let answer: WhyAnswer = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    if answer.why.trim().is_empty() {
        return Err("`why` is empty. Write one sentence from the facts.".into());
    }
    Ok(answer)
}

/// CONVERGE: `{file, why}` and `{point, why}` items as the contract's pairs.
pub fn read_converge(value: &Value) -> Result<ConvergeAnswer, String> {
    let mut answer = ConvergeAnswer::default();
    for item in value.get("extra").and_then(Value::as_array).into_iter().flatten() {
        let file = text_of(item, "file");
        if !file.is_empty() && !answer.extra.iter().any(|(known, _)| *known == file) {
            answer.extra.push((file, text_of(item, "why")));
        }
    }
    for item in value.get("missing").and_then(Value::as_array).into_iter().flatten() {
        let point: u32 = text_of(item, "point").parse().map_err(|_| "`point` must be a point's number")?;
        if !answer.missing.iter().any(|(known, _)| *known == point) {
            answer.missing.push((point, text_of(item, "why")));
        }
    }
    Ok(answer)
}

// ---- the stages ----------------------------------------------------------------------------------

impl Flow {
    /// Ask one constrained question: once, and once more with what was wrong when the answer
    /// cannot be read or fails `read`. A failed call (no model, cancelled) is an error, never an
    /// answer.
    pub async fn decide<T>(&self, data: &Data, asked: Asked<'_>, cancel: &CancellationToken, read: impl Fn(&Value) -> Result<T, String>) -> Result<Result<T, Unusable>, RpcError> {
        let ctx = self.ctx()?;
        let config = data.stage(asked.stage)?;
        let grammar = schemas::grammar(&asked.shape);
        let mut problem: Option<String> = None;
        let mut last = None;
        for _ in 0..2 {
            // The step stays the last line; what was wrong with the first answer goes right before it.
            let again = problem.as_ref().map(|problem| data.text("again", &[("problem", problem)])).unwrap_or_default();
            let user = blocks(&[asked.evidence.clone(), again, asked.step.clone()]);
            let key = llm::key(asked.stage);
            let ask = Ask { stage: asked.stage, config, key: &key, messages: vec![ChatMessage::new("system", data.system(config)), ChatMessage::new("user", user)], tools: Vec::new(), grammar: Some(grammar.clone()) };
            let result = llm::chat(ctx, data, ask, cancel).await;
            llm::end(ctx, &key).await;
            let shaped = parse(&result?.content).and_then(|value| asked.shape.check(&value).map(|()| value));
            let outcome = shaped.and_then(|value| {
                let outcome = read(&value);
                last = Some(value);
                outcome
            });
            match outcome {
                Ok(answer) => return Ok(Ok(answer)),
                Err(found) => {
                    tracing::warn!(stage = asked.stage, problem = %found, "the answer could not be used");
                    problem = Some(found);
                }
            }
        }
        Ok(Err(Unusable { problem: problem.unwrap_or_default(), last }))
    }

    pub async fn assess(&self, params: AssessParams, cancel: &CancellationToken) -> Result<AssessedParams, RpcError> {
        if params.text.trim().is_empty() {
            return Err(RpcError::invalid_params("the request has no text to assess"));
        }
        let data = self.data()?;
        let shape = data.shape(data.stage("ASSESS")?).cloned().ok_or_else(|| RpcError::internal("ASSESS has no schema"))?;
        let evidence = blocks(&[
            data.text("block_app", &[("name", &params.repo_name)]),
            if params.identity.trim().is_empty() { String::new() } else { data.text("block_identity", &[("text", params.identity.trim())]) },
            data.text("block_request", &[("text", params.text.trim())]),
        ]);
        let asked = Asked { stage: "ASSESS", evidence, step: data.text("assess_step", &[]), shape };
        let answer = self.decide(&data, asked, cancel, |value| read_assess(value, &params.request).and_then(|answer| checked_assess(answer, &params.text))).await?;
        Ok(answer.unwrap_or_else(|unusable| cautious_assess(unusable.last.as_ref(), &params.request)))
    }

    pub async fn match_pair(&self, params: MatchParams, cancel: &CancellationToken) -> Result<MatchAnswer, RpcError> {
        let data = self.data()?;
        let shape = data.shape(data.stage("MATCH")?).cloned().ok_or_else(|| RpcError::internal("MATCH has no schema"))?;
        let evidence = data.text("match_pair", &[("a_title", params.a_title.trim()), ("a_text", params.a_text.trim()), ("b_title", params.b_title.trim()), ("b_text", params.b_text.trim())]);
        let asked = Asked { stage: "MATCH", evidence, step: data.text("match_step", &[]), shape };
        let answer = self.decide(&data, asked, cancel, read_match).await?;
        Ok(answer.unwrap_or_else(|_| MatchAnswer { relation: "unrelated".into(), because: data.text("match_unreadable", &[]) }))
    }

    pub async fn validate(&self, params: ValidateParams, cancel: &CancellationToken) -> Result<ValidateAnswer, RpcError> {
        let data = self.data()?;
        let shape = data.shape(data.stage("VALIDATE")?).cloned().ok_or_else(|| RpcError::internal("VALIDATE has no schema"))?;
        let point = &params.point;
        let none = data.text("validate_none", &[]);
        let or_none = |text: String| if text.trim().is_empty() { none.clone() } else { text };
        let walks = point
            .walks
            .iter()
            .map(|walk| match walk.method.as_str() {
                "command" => data.text("validate_walk_command", &[("command", &walk.command), ("expect", &walk.expect), ("do", &walk.do_), ("see", &walk.see)]),
                method => data.text("validate_walk", &[("do", &walk.do_), ("see", &walk.see), ("method", method)]),
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The point is the subject, so it comes last: the request and the laws it is held against
        // are read first, the host's own fact right before it.
        let evidence = blocks(&[
            if params.identity.trim().is_empty() { String::new() } else { data.text("block_identity", &[("text", params.identity.trim())]) },
            if params.laws.is_empty() { String::new() } else { data.text("block_laws", &[("items", &list(&data, &params.laws))]) },
            data.text("block_request", &[("text", params.text.trim())]),
            data.text(if params.touches_standard { "validate_standard" } else { "validate_standard_clear" }, &[]),
            data.text(
                "validate_point",
                &[("title", point.title.trim()), ("brief", &or_none(point.brief.trim().to_owned())), ("rules", &or_none(list(&data, &point.rules))), ("touches", &or_none(point.touches.join(", "))), ("walks", &or_none(walks))],
            ),
        ]);
        let asked = Asked { stage: "VALIDATE", evidence, step: data.text("validate_step", &[]), shape };
        let answer = self.decide(&data, asked, cancel, read_validate).await?;
        let mut answer = match answer {
            Ok(answer) => answer,
            // Nothing is known, so nothing is claimed: a person decides.
            Err(_) => return Ok(ValidateAnswer { necessary: false, sensible: false, fits: false, provable: false, standard_safe: !params.touches_standard, verdict: "ask".into(), reason: data.text("validate_unreadable", &[]) }),
        };
        // What the host knows, it does not ask: a point on a standard file is not standard-safe.
        if params.touches_standard {
            answer.standard_safe = false;
        }
        let all = answer.necessary && answer.sensible && answer.fits && answer.provable && answer.standard_safe;
        if answer.reason.trim().is_empty() {
            answer.verdict = "ask".into();
            answer.reason = data.text("validate_no_reason", &[]);
        } else if answer.verdict == "keep" && !all {
            answer.verdict = "ask".into();
            answer.reason = format!("{} {}", data.text("validate_contradiction", &[]), answer.reason.trim());
        }
        Ok(answer)
    }

    pub async fn drift(&self, params: DriftParams, cancel: &CancellationToken) -> Result<DriftAnswer, RpcError> {
        let data = self.data()?;
        let files = diff_files(&params.diff);
        if files.is_empty() {
            // No changed file: nothing can do the task, and there is nothing to ask about.
            return Ok(DriftAnswer { does_it: false, file: String::new(), line: 0, reason: data.text("drift_no_diff", &[]) });
        }
        let shape = data.shape(data.stage("DRIFT")?).ok_or_else(|| RpcError::internal("DRIFT has no schema"))?.filled(FILES, &files);
        let evidence = blocks(&[
            data.text("drift_task", &[("title", params.title.trim()), ("rules", &{ if params.rules.is_empty() { data.text("validate_none", &[]) } else { list(&data, &params.rules) } })]),
            data.text("drift_diff", &[("diff", &fitted(&data, numbered(&params.diff)))]),
        ]);
        let asked = Asked { stage: "DRIFT", evidence, step: data.text("drift_step", &[]), shape };
        let answer = self.decide(&data, asked, cancel, read_drift).await?;
        let mut answer = answer.unwrap_or_else(|_| DriftAnswer { does_it: false, file: String::new(), line: 0, reason: data.text("drift_unreadable", &[]) });
        // A yes points at a line (the caller then finds that line in the diff); one that does not is a no.
        if answer.does_it && (answer.file.trim().is_empty() || answer.line == 0) {
            answer.does_it = false;
            answer.reason = format!("{} {}", data.text("drift_no_line", &[]), answer.reason.trim()).trim().to_owned();
        }
        Ok(answer)
    }

    pub async fn converge(&self, params: ConvergeParams, cancel: &CancellationToken) -> Result<ConvergeAnswer, RpcError> {
        if params.points.is_empty() {
            return Err(RpcError::invalid_params("there are no points to compare the change with"));
        }
        let data = self.data()?;
        let files = diff_files(&params.diff);
        if files.is_empty() {
            // An empty change does none of the points: no model is needed to see that.
            let why = data.text("drift_no_diff", &[]);
            return Ok(ConvergeAnswer { extra: Vec::new(), missing: params.points.iter().map(|(n, _, _)| (*n, why.clone())).collect() });
        }
        let numbers: Vec<String> = params.points.iter().map(|(n, _, _)| n.to_string()).collect();
        let shape = data.shape(data.stage("CONVERGE")?).ok_or_else(|| RpcError::internal("CONVERGE has no schema"))?.filled(FILES, &files).filled(POINTS, &numbers);
        let points = params
            .points
            .iter()
            .map(|(n, title, rules)| {
                let rules: String = rules.iter().map(|rule| data.text("converge_rule", &[("text", rule.trim())])).collect();
                data.text("converge_point", &[("n", &n.to_string()), ("title", title.trim()), ("rules", &rules)])
            })
            .collect::<Vec<_>>()
            .join("\n");
        let evidence = blocks(&[data.text("converge_points", &[("points", &points)]), data.text("converge_diff", &[("diff", &fitted(&data, params.diff.clone()))])]);
        let asked = Asked { stage: "CONVERGE", evidence, step: data.text("converge_step", &[]), shape };
        match self.decide(&data, asked, cancel, read_converge).await? {
            Ok(answer) => Ok(answer),
            // Empty lists would read as "the change is exactly the plan". It was not compared: say so.
            Err(unusable) => Err(RpcError::new(codes::UNAVAILABLE, data.text("converge_failed", &[("problem", &unusable.problem)])).with_data(json!({"extra": [], "missing": [], "compared": false}))),
        }
    }

    /// DIGEST: the only free writing. Two lines; nothing is decided from them.
    pub async fn digest(&self, facts_value: &Value, cancel: &CancellationToken) -> Result<String, RpcError> {
        let data = self.data()?;
        let ctx = self.ctx()?;
        let config = data.stage("DIGEST")?;
        let told = facts(facts_value);
        if told.trim().is_empty() {
            return Err(RpcError::invalid_params("there are no facts to write the digest from"));
        }
        let user = blocks(&[data.text("digest_facts", &[("facts", &told)]), data.text("digest_step", &[])]);
        let key = llm::key("DIGEST");
        let ask = Ask { stage: "DIGEST", config, key: &key, messages: vec![ChatMessage::new("system", data.system(config)), ChatMessage::new("user", user)], tools: Vec::new(), grammar: None };
        let result = llm::chat(ctx, &data, ask, cancel).await;
        llm::end(ctx, &key).await;
        let lines: Vec<String> = result?.content.lines().map(|line| line.trim().to_owned()).filter(|line| !line.is_empty()).take(2).collect();
        if lines.is_empty() {
            return Err(RpcError::new(codes::UNAVAILABLE, "the model wrote no digest"));
        }
        Ok(lines.join("\n"))
    }

    /// REPLY: what the manager says back to a person in a session. Free writing; nothing is
    /// decided from it.
    pub async fn reply(&self, facts_value: &Value, wrote: &str, cancel: &CancellationToken) -> Result<String, RpcError> {
        let data = self.data()?;
        let ctx = self.ctx()?;
        let config = data.stage("REPLY")?;
        let user = blocks(&[data.text("reply_facts", &[("facts", &facts(facts_value))]), data.text("reply_wrote", &[("text", wrote)]), data.text("reply_step", &[])]);
        let key = llm::key("REPLY");
        let ask = Ask { stage: "REPLY", config, key: &key, messages: vec![ChatMessage::new("system", data.system(config)), ChatMessage::new("user", user)], tools: Vec::new(), grammar: None };
        let result = llm::chat(ctx, &data, ask, cancel).await;
        llm::end(ctx, &key).await;
        let text = result?.content.trim().to_owned();
        if text.is_empty() {
            return Err(RpcError::new(codes::UNAVAILABLE, "the model wrote no answer"));
        }
        Ok(text)
    }

    pub async fn why(&self, facts_value: &Value, cancel: &CancellationToken) -> Result<WhyAnswer, RpcError> {
        let data = self.data()?;
        let shape = data.shape(data.stage("WHY")?).cloned().ok_or_else(|| RpcError::internal("WHY has no schema"))?;
        let told = facts(facts_value);
        if told.trim().is_empty() {
            return Err(RpcError::invalid_params("there are no facts to write the decision's words from"));
        }
        let asked = Asked { stage: "WHY", evidence: data.text("why_facts", &[("facts", &told)]), step: data.text("why_step", &[]), shape };
        match self.decide(&data, asked, cancel, read_why).await? {
            Ok(answer) => Ok(answer),
            // The caller writes the item from its facts itself rather than show made-up words.
            Err(unusable) => Err(RpcError::new(codes::UNAVAILABLE, data.text("why_failed", &[("problem", &unusable.problem)]))),
        }
    }
}
