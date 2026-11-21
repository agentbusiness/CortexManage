//! PLAN: the only stage that is offered tools, and only three that read (`read`, `search`, `tree`),
//! each served by the manage plugin's discovery on the repository's mirror. A request's text can
//! hold anything, so the model that reads it never has a tool that writes or runs.
//!
//! Two parts, each in a context of its own. First a short loop in which the model looks at the
//! code the request concerns (at most `limits.plan_tool_calls` calls). Then the plan itself: one
//! schema-constrained question that carries what the reads found as plain text, and no tools,
//! because a prompt that shows tools it cannot run gets tool calls written as text.

use crate::data::Data;
use crate::flow::Flow;
use crate::llm::{self, Ask};
use crate::schemas::TASKS;
use crate::stages::{Asked, blocks, list};
use cc_protocol::contracts::manage::{DiscoverParams, Plan, PlanParams, Point, Tier, Walk, methods as manage_methods};
use cc_protocol::contracts::model::{ChatMessage, ToolCall, ToolSpec};
use cc_protocol::contracts::now_ms;
use cc_protocol::{RpcError, codes};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// One read the discovery made: how it is named in the plan's question, and what it gave.
struct Found {
    call: String,
    output: String,
}

fn tools(data: &Data) -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read".into(),
            description: data.text("tool_read", &[]),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file, from the repository's top folder."},
                    "from": {"type": "integer", "minimum": 1, "description": "The first line to read. Leave out to start at the top."},
                    "lines": {"type": "integer", "minimum": 1, "description": "How many lines to read."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "search".into(),
            description: data.text("tool_search", &[]),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "The exact text to find: a word or a name, not a sentence."},
                    "under": {"type": "string", "description": "A folder to search in. Leave out to search everywhere."}
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "tree".into(),
            description: data.text("tool_tree", &[]),
            parameters: json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": "The folder. Leave out for the top folder."}},
                "additionalProperties": false
            }),
        },
    ]
}

/// `text` when it fits `most` characters; else its head and tail with `cut` between them, both cut
/// at line ends.
fn head_tail(text: &str, most: usize, cut: impl Fn(usize) -> String) -> String {
    if text.len() <= most {
        return text.to_owned();
    }
    let half = most / 2;
    let mut head = half;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let head = text[..head].rfind('\n').map_or(head, |at| at + 1);
    let mut tail = text.len() - half;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    let tail = text[tail..].find('\n').map_or(tail, |at| tail + at + 1);
    if tail <= head {
        return text.to_owned();
    }
    format!("{}{}{}", &text[..head], cut(text[head..tail].lines().count()), &text[tail..])
}

/// A discovery answer as text. The contract leaves the items' fields open (`{text}`, `{hits}`,
/// `{entries}`), so an item is read by the names such answers use and shown whole otherwise.
fn shown(data: &Data, answer: &Value) -> String {
    fn word<'a>(item: &'a Value, names: &[&str]) -> Option<&'a str> {
        names.iter().find_map(|name| item.get(name).and_then(Value::as_str)).filter(|text| !text.is_empty())
    }
    if let Some(text) = answer.get("text").and_then(Value::as_str) {
        return if text.is_empty() { data.text("tool_empty", &[]) } else { text.to_owned() };
    }
    let items = answer.get("hits").or_else(|| answer.get("entries")).and_then(Value::as_array);
    let Some(items) = items else { return answer.to_string() };
    let lines: Vec<String> = items
        .iter()
        .map(|item| match item {
            Value::String(text) => text.clone(),
            Value::Object(_) => {
                let Some(path) = word(item, &["path", "file", "name"]) else { return item.to_string() };
                let folder = item.get("dir").and_then(Value::as_bool).unwrap_or(false) || matches!(word(item, &["kind", "type"]), Some("dir" | "folder" | "directory"));
                let line = item.get("line").and_then(Value::as_u64).map(|line| format!(":{line}")).unwrap_or_default();
                let text = word(item, &["text", "snippet", "preview"]).map(|text| format!(": {}", text.trim_end())).unwrap_or_default();
                format!("{path}{}{line}{text}", if folder && !path.ends_with('/') { "/" } else { "" })
            }
            other => other.to_string(),
        })
        .collect();
    if lines.is_empty() { data.text("tool_empty", &[]) } else { lines.join("\n") }
}

/// A path as the discovery takes it: from the repository's top folder, without `./` or a leading `/`.
fn repo_path(path: &str) -> String {
    let path = path.trim();
    let path = path.strip_prefix("./").unwrap_or(path);
    path.trim_start_matches('/').to_owned()
}

impl Flow {
    /// One read-only tool call: what it is called in the plan's question, and its answer (Err: the
    /// text the model is shown instead, ending with what to try next).
    async fn discover(&self, data: &Data, repo: &str, call: &ToolCall) -> (String, Result<String, String>) {
        let name = call.name.as_str();
        let arguments = if call.arguments.trim().is_empty() { Some(json!({})) } else { serde_json::from_str::<Value>(&call.arguments).ok().filter(Value::is_object) };
        let Some(arguments) = arguments else {
            return (name.to_owned(), Err(data.text("tool_bad_args", &[("name", name)])));
        };
        let text = |field: &str| arguments.get(field).and_then(Value::as_str).unwrap_or("").trim().to_owned();
        let number = |field: &str| arguments.get(field).and_then(Value::as_u64).map(|value| value.min(u64::from(u32::MAX)) as u32);
        let mut params = DiscoverParams { repo: repo.to_owned(), ..DiscoverParams::default() };
        let (method, label) = match name {
            "read" => {
                params.path = repo_path(&text("path"));
                params.from = number("from");
                params.lines = Some(number("lines").unwrap_or(data.limits.read_lines));
                let from = params.from.map(|from| format!(" from line {from}")).unwrap_or_default();
                (manage_methods::DISCOVER_READ, format!("read {}{from}", params.path))
            }
            "search" => {
                params.text = text("text");
                params.under = repo_path(&text("under"));
                params.limit = Some(data.limits.search_hits);
                let under = if params.under.is_empty() { String::new() } else { format!(" under {}", params.under) };
                (manage_methods::DISCOVER_SEARCH, format!("search \"{}\"{under}", params.text))
            }
            "tree" => {
                params.path = repo_path(&text("path"));
                (manage_methods::DISCOVER_TREE, format!("tree {}", if params.path.is_empty() { "(top folder)" } else { &params.path }))
            }
            other => return (other.to_owned(), Err(data.text("tool_unknown", &[("name", other)]))),
        };
        let params = match serde_json::to_value(&params) {
            Ok(params) => params,
            Err(error) => return (label, Err(error.to_string())),
        };
        let outcome = match self.ctx() {
            Ok(ctx) => ctx.call(method, params).await,
            Err(error) => Err(error),
        };
        match outcome {
            Ok(answer) => {
                let empty_search = name == "search" && answer.get("hits").and_then(Value::as_array).is_some_and(Vec::is_empty);
                if empty_search {
                    return (label, Err(data.text("tool_no_hits", &[("text", &text("text"))])));
                }
                (label, Ok(shown(data, &answer)))
            }
            Err(error) => (label, Err(data.text("tool_failed", &[("error", &error.message)]))),
        }
    }

    /// The reading before the plan: the model asks for reads until it knows enough or the calls
    /// are spent. Its closing words are not kept: the plan is written from what the reads gave.
    async fn look_around(&self, data: &Data, params: &PlanParams, tree: &str, cancel: &CancellationToken) -> Result<Vec<Found>, RpcError> {
        let ctx = self.ctx()?;
        let config = data.stage("DISCOVER")?;
        let specs = tools(data);
        let user = blocks(&[
            data.text("block_app", &[("name", &params.repo_name)]),
            data.text("block_request", &[("text", params.text.trim())]),
            data.text("discover_start", &[("tree", tree)]),
            data.text("discover_step", &[]),
        ]);
        let mut messages = vec![ChatMessage::new("system", data.system(config)), ChatMessage::new("user", user)];
        let mut found: Vec<Found> = Vec::new();
        let mut left = data.limits.plan_tool_calls;
        // One key for the loop: each step continues the same context, so the model plugin keeps
        // its prefix and reads only the new results.
        let key = llm::key("DISCOVER");
        let outcome = async {
            while left > 0 {
                let ask = Ask { stage: "DISCOVER", config, key: &key, messages: messages.clone(), tools: specs.clone(), grammar: None };
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
                    let (label, answer) = self.discover(data, &params.repo, call).await;
                    let text = match answer {
                        Ok(output) => {
                            // A read made twice is carried into the plan once.
                            if !found.iter().any(|known| known.call == label) {
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

    pub async fn plan(&self, params: PlanParams, cancel: &CancellationToken) -> Result<Plan, RpcError> {
        if params.text.trim().is_empty() {
            return Err(RpcError::invalid_params("the request has no text to plan from"));
        }
        let data = self.data()?;
        let ctx = self.ctx()?;
        // The top folder is the first read, made by the host: it shows the model where to start,
        // and tells the host whether the repository can be read at all.
        let top = serde_json::to_value(DiscoverParams { repo: params.repo.clone(), ..DiscoverParams::default() }).map_err(|error| RpcError::internal(error.to_string()))?;
        let (found, blind) = match ctx.call(manage_methods::DISCOVER_TREE, top).await {
            Ok(answer) => {
                let tree = head_tail(&shown(&data, &answer), data.limits.tool_result_chars, |_| "\n".to_owned());
                (self.look_around(&data, &params, &tree, cancel).await?, None)
            }
            Err(error) if error.code == codes::CANCELLED || cancel.is_cancelled() => return Err(RpcError::cancelled()),
            // No mirror, no runner, no link: the plan is written from the request and the rules
            // alone, and its note says so.
            Err(error) => (Vec::new(), Some(error.message)),
        };

        let reads = if blind.is_some() {
            data.text("plan_blind", &[])
        } else if found.is_empty() {
            String::new()
        } else {
            let items: Vec<String> = found
                .iter()
                .map(|read| {
                    let output = head_tail(&read.output, data.limits.plan_evidence_chars, |lines| data.text("found_cut", &[("lines", &lines.to_string())]));
                    data.text("plan_found_item", &[("call", &read.call), ("output", output.trim_end())])
                })
                .collect();
            format!("{}\n{}", data.text("plan_found", &[]), items.join("\n\n"))
        };
        let ceremony = match params.tier {
            Tier::Safe => "ceremony_safe",
            Tier::Minor => "ceremony_minor",
            Tier::Major => "ceremony_major",
        };
        // In reading order: what the app is and its rules, what the code holds, then the request
        // and how large it was judged, so the request is the last thing read before the plan.
        let evidence = blocks(&[
            data.text("block_app", &[("name", &params.repo_name)]),
            if params.identity.trim().is_empty() { String::new() } else { data.text("block_identity", &[("text", params.identity.trim())]) },
            if params.rules.is_empty() { String::new() } else { data.text("block_rules", &[("items", &list(&data, &params.rules))]) },
            reads,
            if params.note.trim().is_empty() { String::new() } else { data.text("block_earlier", &[("text", params.note.trim())]) },
            data.text("block_request", &[("text", params.text.trim())]),
            data.text("block_kind", &[("kind", &params.kind), ("tier", params.tier.as_str())]),
            data.text(ceremony, &[]),
        ]);
        let most = data.limits.plan_max_tasks;
        let numbers: Vec<String> = (1..=most).map(|n| n.to_string()).collect();
        let shape = data.shape(data.stage("PLAN")?).ok_or_else(|| RpcError::internal("PLAN has no schema"))?.filled(TASKS, &numbers).capped("tasks", most);
        let asked = Asked { stage: "PLAN", evidence, step: data.text("plan_step", &[]), shape };
        let answer = self.decide(&data, asked, cancel, |value| read_plan(value, &params.request, params.tier, &data)).await?;
        let mut plan = match answer {
            Ok(plan) => plan,
            // There is no cautious plan: nothing is returned as one.
            Err(unusable) => return Err(RpcError::new(codes::UNAVAILABLE, data.text("plan_failed", &[("problem", &unusable.problem)]))),
        };
        if let Some(error) = blind {
            plan.note = format!("{} {}", data.text("note_blind", &[("error", &error)]), plan.note).trim().to_owned();
        }
        Ok(plan)
    }
}

fn text_of(value: &Value, field: &str) -> String {
    value.get(field).and_then(Value::as_str).unwrap_or("").trim().to_owned()
}

fn texts_of(value: &Value, field: &str) -> Vec<String> {
    value.get(field).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|text| text.trim().to_owned()).filter(|text| !text.is_empty()).collect()
}

/// The model's plan as the contract's [`Plan`]: tasks become points numbered from 1, each walk
/// gets its id, and what code can settle is settled (a task depends only on earlier ones; a
/// `command` walk has a command and no other walk does; a limit is never zero).
///
/// `version` is 1: the manage plugin, which knows the earlier plans, numbers the versions.
pub fn read_plan(value: &Value, request: &str, tier: Tier, data: &Data) -> Result<Plan, String> {
    let tasks = value.get("tasks").and_then(Value::as_array).cloned().unwrap_or_default();
    if tasks.is_empty() {
        return Err("`tasks` is empty. Write at least one task that changes code.".into());
    }
    // A safe change gets one walk on a task; a minor or major one, one or two.
    let walks_most = if tier == Tier::Safe { 1 } else { 2 };
    let mut points = Vec::new();
    for (index, task) in tasks.iter().take(data.limits.plan_max_tasks).enumerate() {
        let n = index as u32 + 1;
        let title = text_of(task, "title");
        if title.is_empty() {
            return Err(format!("task {n} has no `title`. Name what it makes happen."));
        }
        let mut depends_on: Vec<u32> = texts_of(task, "depends_on").iter().filter_map(|text| text.parse().ok()).filter(|earlier| *earlier >= 1 && *earlier < n).collect();
        depends_on.sort_unstable();
        depends_on.dedup();
        let touches = texts_of(task, "touches");
        let mut walks = Vec::new();
        for walk in task.get("walks").and_then(Value::as_array).into_iter().flatten() {
            let method = text_of(walk, "method");
            let by_command = method == "command";
            let (do_, see, command) = (text_of(walk, "do"), text_of(walk, "see"), text_of(walk, "command"));
            // A walk nobody could take proves nothing: it is left out, and the validator then
            // finds the task without proof.
            if do_.is_empty() || see.is_empty() || (by_command && command.is_empty()) {
                continue;
            }
            walks.push(Walk {
                id: format!("w{n}.{}", walks.len() + 1),
                do_,
                see,
                method,
                evidence_kind: text_of(walk, "evidence_kind"),
                command: if by_command { command } else { String::new() },
                expect: if by_command { text_of(walk, "expect") } else { String::new() },
            });
            if walks.len() == walks_most {
                break;
            }
        }
        points.push(Point {
            n,
            title,
            brief: text_of(task, "brief"),
            known: texts_of(task, "known"),
            rules: texts_of(task, "rules"),
            depends_on,
            // No task has a size limit.
            limit_files: 0,
            limit_lines: 0,
            touches,
            checks: Vec::new(),
            walks,
            verdict: String::new(),
            verdict_reason: String::new(),
            request: request.to_owned(),
        });
    }
    Ok(Plan {
        version: 1,
        analysis: text_of(value, "analysis"),
        goal: text_of(value, "goal"),
        approach: text_of(value, "approach"),
        architecture: text_of(value, "architecture"),
        risks: texts_of(value, "risks"),
        points,
        note: text_of(value, "note"),
        created_ms: now_ms(),
    })
}
