//! One model call, the way CortexCode's chat flow makes it: `mesh.claim` takes one use of the best
//! free model with the stage's tag (the frame then routes the key's calls there), `model.chat`
//! runs, `mesh.release` gives the model back. The manager claims in the background
//! (`foreground: false`): a person's turn always goes first.
//!
//! Every question has its own key, so it starts from an empty context: nothing a stage wrote can
//! be the cue for another stage's answer.

use crate::data::{Data, StageConfig};
use cc_protocol::contracts::mesh::methods as mesh_methods;
use cc_protocol::contracts::model::{ChatMessage, ChatParams, ChatResult, CountResult, Priority, Thinking, ToolSpec, methods as model_methods};
use cc_protocol::host::{interrupted, refused};
use cc_protocol::{Ctx, RpcError, codes, parse_params};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// How long to wait for the model plugin's answer after cancelling a generation.
const CANCEL_GRACE: Duration = Duration::from_millis(300);
/// Room the engine needs to close a thinking block inside the token limit.
const THINK_CLOSE_TOKENS: u32 = 64;

/// What one call sends.
pub struct Ask<'a> {
    /// The stage, for the prompt dump's file name.
    pub stage: &'a str,
    pub config: &'a StageConfig,
    /// The key of the question's context: the claim's session and the call's cache key.
    pub key: &'a str,
    pub messages: Vec<ChatMessage>,
    /// Offered only in a call that can run them.
    pub tools: Vec<ToolSpec>,
    /// The answer's grammar (None: free text or tool calls).
    pub grammar: Option<String>,
}

/// A new key for one question (`manage-<stage>-<id>`).
pub fn key(stage: &str) -> String {
    format!("manage-{}-{}", stage.to_lowercase(), uuid::Uuid::new_v4().simple())
}

/// One use of the best free model with `tag` for `key`, waiting until one is free. Without the
/// mesh plugin the call runs on this app's own model plugin.
async fn claim(ctx: &Ctx, key: &str, tag: &str, tokens: u32, cancel: &CancellationToken) -> Result<(), RpcError> {
    let params = json!({"kind": "model", "tag": tag, "session_id": key, "foreground": false, "tokens": tokens});
    let Ok(mut call) = ctx.call_stream(mesh_methods::CLAIM, params) else { return Ok(()) };
    loop {
        tokio::select! {
            item = call.next() => if item.is_none() { break },
            _ = cancel.cancelled() => {
                call.cancel();
                return Err(RpcError::cancelled());
            }
        }
    }
    if let Err(error) = call.finish().await {
        tracing::debug!(%error, "no model claimed through the mesh; the call runs on this app's model plugin");
    }
    Ok(())
}

/// The model is free for the next run.
async fn release(ctx: &Ctx, key: &str) {
    let _ = ctx.call(mesh_methods::RELEASE, json!({"kind": "model", "session_id": key})).await;
}

/// The question is over: its key holds no place in the network any more.
pub async fn end(ctx: &Ctx, key: &str) {
    let _ = ctx.call(mesh_methods::PLACE_END, json!({"session_id": key})).await;
}

/// Run one call to its end. A call the model plugin refused (restarting, busy) or lost mid-way is
/// claimed and sent again for the data set's window; any other failure is the caller's.
pub async fn chat(ctx: &Ctx, data: &Data, ask: Ask<'_>, cancel: &CancellationToken) -> Result<ChatResult, RpcError> {
    let thinks = ask.config.effort != "none" && ask.grammar.is_none();
    let thinking = Thinking { enabled: thinks, effort: thinks.then(|| ask.config.effort.clone()), budget_tokens: (thinks && ask.config.think_tokens > 0).then_some(ask.config.think_tokens) };
    let max_tokens = ask.config.max_tokens + if thinks { ask.config.think_tokens + THINK_CLOSE_TOKENS } else { 0 };
    let has_tools = !ask.tools.is_empty();
    let params = ChatParams {
        messages: ask.messages,
        tools: ask.tools,
        tool_choice: has_tools.then(|| json!("auto")),
        grammar: ask.grammar,
        json_schema: None,
        sampling: data.sampling(ask.config),
        thinking,
        priority: Priority::Background,
        cache_key: Some(ask.key.to_owned()),
        stop: Vec::new(),
        max_tokens: Some(max_tokens),
        idle_only: false,
    };
    let params = serde_json::to_value(&params).map_err(|error| RpcError::internal(error.to_string()))?;
    dump(ask.stage, &params);
    // The prompt's size, so the call goes only to a model whose context holds it (0: unknown).
    let counted = ctx.call(model_methods::COUNT, json!({"messages": params["messages"], "tools": params["tools"]})).await.and_then(parse_params::<CountResult>);
    let tokens = counted.map_or(0, |counted| counted.tokens + max_tokens);
    let started = Instant::now();
    let window = Duration::from_secs(data.limits.refused_window_s);
    let mut delay = Duration::from_millis(200);
    loop {
        if cancel.is_cancelled() {
            return Err(RpcError::cancelled());
        }
        claim(ctx, ask.key, &ask.config.tag, tokens, cancel).await?;
        let outcome = stream_once(ctx, &params, cancel).await;
        release(ctx, ask.key).await;
        match outcome {
            Ok(result) => return Ok(result),
            Err(error) if (refused(&error) || interrupted(&error)) && started.elapsed() < window => {
                // The model plugin says when it is back only by answering: try again a little later.
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = cancel.cancelled() => return Err(RpcError::cancelled()),
                }
                delay = (delay * 2).min(Duration::from_secs(2));
            }
            Err(error) => return Err(error),
        }
    }
}

async fn stream_once(ctx: &Ctx, params: &Value, cancel: &CancellationToken) -> Result<ChatResult, RpcError> {
    let mut stream = ctx.call_stream(model_methods::CHAT, params.clone())?;
    loop {
        // The streamed pieces are not shown anywhere: the answer is read whole, when it is done.
        tokio::select! {
            item = stream.next() => if item.is_none() { break },
            _ = cancel.cancelled() => {
                stream.cancel();
                let _ = tokio::time::timeout(CANCEL_GRACE, async { while stream.next().await.is_some() {} }).await;
                return Err(RpcError::cancelled());
            }
        }
    }
    match stream.finish().await {
        Ok(value) => parse_params(value),
        // The model plugin cancelled on its own (preempted while being swapped): try again.
        Err(error) if error.code == codes::CANCELLED && !cancel.is_cancelled() => Err(RpcError::new(codes::UNAVAILABLE, error.message).retryable()),
        Err(error) => Err(error),
    }
}

/// Development aid: with `CORTEX_DUMP_PROMPTS=<dir>` every request is written there as JSON, so a
/// prompt can be replayed against the model by hand.
fn dump(stage: &str, params: &Value) {
    let Ok(dir) = std::env::var("CORTEX_DUMP_PROMPTS") else { return };
    let dir = std::path::PathBuf::from(dir);
    if std::fs::create_dir_all(&dir).is_ok() {
        let name = format!("{}-{stage}-{}.json", cc_protocol::contracts::now_ms(), uuid::Uuid::new_v4().simple());
        let _ = std::fs::write(dir.join(name), params.to_string());
    }
}
