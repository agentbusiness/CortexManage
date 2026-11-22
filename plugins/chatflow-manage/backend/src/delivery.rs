//! DELIVERY, COMMIT and RULES: how a repository ships, a commit's message, and the person's rules
//! over a finished change.
//!
//! DELIVERY is like PLAN: first a short loop in which the model reads the repository's main branch
//! on GitHub with the three read-only tools (DELIVERY_READ), then one schema-bound question that
//! writes each value with the file, line and words it stands on. The manage plugin keeps a value
//! only when those words are found there. Nothing here writes or runs anything.

use crate::data::Data;
use crate::flow::Flow;
use crate::llm::{self, Ask};
use crate::plan::{Found, head_tail, shown, tools};
use crate::stages::{Asked, blocks};
use cc_protocol::contracts::manage::{CommitParams, DeliveryAnswer, DeliveryFact, DeliveryParams, DiscoverParams, RulesAnswer, RulesParams, methods as manage_methods};
use cc_protocol::contracts::model::{ChatMessage, ToolCall};
use cc_protocol::{RpcError, codes};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// The fields whose values are lists: several facts of one field add up.
const LIST_FIELDS: &[&str] = &["release_steps", "checks", "secrets", "secret_files"];

fn text_of(value: &Value, field: &str) -> String {
    value.get(field).and_then(Value::as_str).unwrap_or("").trim().to_owned()
}

/// The model's facts as the contract's answer: each value once (a list field's in order), each
/// fact with its place.
pub fn read_delivery(value: &Value) -> Result<DeliveryAnswer, String> {
    let mut answer = DeliveryAnswer::default();
    for item in value.get("facts").and_then(Value::as_array).into_iter().flatten() {
        let field = text_of(item, "field");
        let value = text_of(item, "value");
        let path = text_of(item, "path");
        let quote = text_of(item, "quote");
        if field.is_empty() || value.is_empty() {
            continue;
        }
        if path.is_empty() || quote.is_empty() {
            return Err(format!("the fact for `{field}` has no `path` or no `quote`. Give the file and the exact words for every fact."));
        }
        let line = item.get("line").and_then(Value::as_u64).unwrap_or(0).min(u64::from(u32::MAX)) as u32;
        answer.facts.push(DeliveryFact { field: field.clone(), path, line, quote });
        match answer.values.iter_mut().find(|(known, _)| *known == field) {
            Some((_, known)) if LIST_FIELDS.contains(&field.as_str()) => {
                if !known.lines().any(|line| line == value) {
                    known.push('\n');
                    known.push_str(&value);
                }
            }
            // A field that is not a list keeps its first value.
            Some(_) => {}
            None => answer.values.push((field, value)),
        }
    }
    Ok(answer)
}

impl Flow {
    /// The reading loop before the profile is written, on GitHub.
    async fn delivery_read(&self, data: &Data, params: &DeliveryParams, tree: &str, cancel: &CancellationToken) -> Result<Vec<Found>, RpcError> {
        let ctx = self.ctx()?;
        let config = data.stage("DELIVERY_READ")?;
        let specs = tools(data);
        let user = blocks(&[
            data.text("delivery_start", &[("name", &params.repo_name), ("main", &params.main), ("github", params.github.trim()), ("tree", tree)]),
            data.text("delivery_step_read", &[]),
        ]);
        let mut messages = vec![ChatMessage::new("system", data.system(config)), ChatMessage::new("user", user)];
        let mut found: Vec<Found> = Vec::new();
        let mut left = data.limits.delivery_tool_calls;
        let key = llm::key("DELIVERY_READ");
        let outcome = async {
            while left > 0 {
                let ask = Ask { stage: "DELIVERY_READ", config, key: &key, messages: messages.clone(), tools: specs.clone(), grammar: None };
                let result = llm::chat(ctx, data, ask, cancel).await?;
                if result.tool_calls.is_empty() {
                    break;
                }
                let calls: Vec<ToolCall> = result.tool_calls.into_iter().take(left as usize).collect();
                let mut step = ChatMessage::new("assistant", result.content);
                step.tool_calls = calls.clone();
                messages.push(step);
                for call in &calls {
                    if cancel.is_cancelled() {
                        return Err(RpcError::cancelled());
                    }
                    left -= 1;
                    let (label, answer) = self.discover(data, &params.repo, "github", call).await;
                    let text = match answer {
                        Ok(output) => {
                            // Only file reads carry line numbers a fact can stand on.
                            if call.name == "read" && !found.iter().any(|known| known.call == label) {
                                found.push(Found { call: label, output: output.clone() });
                            }
                            head_tail(&output, data.limits.tool_result_chars, |lines| data.text("tool_cut", &[("lines", &lines.to_string())]))
                        }
                        Err(problem) => problem,
                    };
                    let mut result = ChatMessage::new("tool", text);
                    result.tool_call_id = Some(call.id.clone());
                    result.name = Some(call.name.clone());
                    messages.push(result);
                }
            }
            Ok(())
        }
        .await;
        llm::end(ctx, &key).await;
        outcome.map(|()| found)
    }

    pub async fn delivery(&self, params: DeliveryParams, cancel: &CancellationToken) -> Result<DeliveryAnswer, RpcError> {
        let data = self.data()?;
        let ctx = self.ctx()?;
        let top = serde_json::to_value(DiscoverParams { repo: params.repo.clone(), via: "github".into(), ..DiscoverParams::default() }).map_err(|error| RpcError::internal(error.to_string()))?;
        let tree = match ctx.call(manage_methods::DISCOVER_TREE, top).await {
            Ok(answer) => head_tail(&shown(&data, &answer), data.limits.tool_result_chars, |_| "\n".to_owned()),
            Err(error) if error.code == codes::CANCELLED || cancel.is_cancelled() => return Err(RpcError::cancelled()),
            // Without the repository's files nothing can be written down: say why.
            Err(error) => return Err(RpcError::new(codes::UNAVAILABLE, data.text("delivery_failed", &[("problem", &error.message)]))),
        };
        let found = self.delivery_read(&data, &params, &tree, cancel).await?;
        let items: Vec<String> = found
            .iter()
            .map(|read| {
                let output = head_tail(&read.output, data.limits.plan_evidence_chars, |lines| data.text("found_cut", &[("lines", &lines.to_string())]));
                data.text("plan_found_item", &[("call", &read.call), ("output", output.trim_end())])
            })
            .collect();
        // What GitHub declares and what the files hold, then the step: the evidence right before
        // the answer.
        let evidence = blocks(&[
            data.text("block_app", &[("name", &params.repo_name)]),
            data.text("delivery_github", &[("github", params.github.trim())]),
            if items.is_empty() { String::new() } else { format!("{}\n{}", data.text("delivery_found", &[]), items.join("\n\n")) },
        ]);
        let shape = data.shape(data.stage("DELIVERY")?).cloned().ok_or_else(|| RpcError::internal("DELIVERY has no schema"))?;
        let asked = Asked { stage: "DELIVERY", evidence, step: data.text("delivery_step", &[]), shape };
        let mut answer = match self.decide(&data, asked, cancel, read_delivery).await? {
            Ok(answer) => answer,
            Err(unusable) => return Err(RpcError::new(codes::UNAVAILABLE, data.text("delivery_failed", &[("problem", &unusable.problem)]))),
        };
        answer.read = found.iter().map(|read| read.call.clone()).collect();
        Ok(answer)
    }

    /// COMMIT: free writing; the manage plugin checks the first line.
    pub async fn commit(&self, params: CommitParams, cancel: &CancellationToken) -> Result<String, RpcError> {
        let data = self.data()?;
        let ctx = self.ctx()?;
        let config = data.stage("COMMIT")?;
        let diff = head_tail(&params.diff, data.limits.diff_chars, |lines| data.text("found_cut", &[("lines", &lines.to_string())]));
        let user = blocks(&[
            if params.rules.trim().is_empty() { String::new() } else { data.text("commit_rules", &[("rules", params.rules.trim())]) },
            data.text("commit_diff", &[("diff", diff.trim_end())]),
            data.text("commit_request", &[("title", params.title.trim()), ("text", params.text.trim())]),
            data.text("commit_step", &[]),
        ]);
        let key = llm::key("COMMIT");
        let ask = Ask { stage: "COMMIT", config, key: &key, messages: vec![ChatMessage::new("system", data.system(config)), ChatMessage::new("user", user)], tools: Vec::new(), grammar: None };
        let result = llm::chat(ctx, &data, ask, cancel).await;
        llm::end(ctx, &key).await;
        let text = result?.content.trim().trim_matches('`').trim().to_owned();
        if text.is_empty() {
            return Err(RpcError::new(codes::UNAVAILABLE, "the model wrote no commit message"));
        }
        Ok(text)
    }

    /// RULES: does an added line or a message break one of the person's rules.
    pub async fn rules(&self, params: RulesParams, cancel: &CancellationToken) -> Result<RulesAnswer, RpcError> {
        let data = self.data()?;
        let shape = data.shape(data.stage("RULES")?).cloned().ok_or_else(|| RpcError::internal("RULES has no schema"))?;
        let diff = head_tail(&params.diff, data.limits.diff_chars, |lines| data.text("found_cut", &[("lines", &lines.to_string())]));
        let evidence = blocks(&[
            data.text("rules_rules", &[("rules", params.rules.trim())]),
            if params.messages.is_empty() { String::new() } else { data.text("rules_messages", &[("messages", &params.messages.join("\n---\n"))]) },
            data.text("rules_diff", &[("diff", diff.trim_end())]),
        ]);
        let asked = Asked { stage: "RULES", evidence, step: data.text("rules_step", &[]), shape };
        let read = |value: &Value| -> Result<RulesAnswer, String> {
            let answer = RulesAnswer {
                breaks: value.get("breaks").and_then(Value::as_bool).unwrap_or(false),
                file: text_of(value, "file"),
                quote: text_of(value, "quote"),
                rule: text_of(value, "rule"),
                reason: text_of(value, "reason"),
            };
            if answer.breaks && answer.quote.is_empty() {
                return Err("`breaks` is true but `quote` is empty. Copy the words that break the rule, or set `breaks` to false.".into());
            }
            Ok(answer)
        };
        match self.decide(&data, asked, cancel, read).await? {
            Ok(answer) => Ok(answer),
            Err(unusable) => Err(RpcError::new(codes::UNAVAILABLE, data.text("rules_failed", &[("problem", &unusable.problem)]))),
        }
    }
}
