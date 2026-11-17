//! The flow as the frame sees it: the `chatflow@1` verbs a request's session needs, and the
//! manager's stage methods (`manage::flow`), which the manage plugin calls.
//!
//! A request's session is a conversation about one change. What a person writes there goes to the
//! manage plugin (`manage.request.say`), which adds it to the request or steers the builder. Before
//! the building starts the model answers them (REPLY), from the request as it stands; the manage
//! plugin's own line is shown when the model does not answer and from the building on. The verbs
//! of a coding conversation (plans, specs, approvals, attachments) are not part of this flow.

use crate::data::Data;
use cc_protocol::contracts::chatflow::{FlowState, RunningResult, TurnResult, verbs};
use cc_protocol::contracts::frame::{kinds, methods as frame_methods};
use cc_protocol::contracts::manage::{AssessParams, ConvergeParams, DriftParams, MatchParams, PlanParams, SayParams, ValidateParams, flow as stage_methods, methods as manage_methods};
use cc_protocol::{Ctx, PluginEnv, RpcError, codes, parse_params, to_value};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

pub const CATEGORY: &str = stage_methods::CATEGORY;

/// Every verb the flow serves (its hello's `provides`).
pub const VERBS: &[&str] = &[verbs::TURN, verbs::STEER, verbs::CANCEL, verbs::STATE, verbs::RUNNING, "assess", "match", "plan", "validate", "drift", "converge", "digest", "why"];

/// Something in flight: a stage's model question, or a message on its way to the manage plugin.
struct Run {
    id: String,
    /// The session it belongs to; empty for a question about no one session (MATCH, DRIFT, …).
    session: String,
    stage: String,
    cancel: CancellationToken,
}

/// Takes its run off the list when the work ends, however it ends.
struct Running {
    runs: Arc<Mutex<Vec<Run>>>,
    id: String,
    cancel: CancellationToken,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.retain(|run| run.id != self.id);
        }
    }
}

pub struct Flow {
    env: PluginEnv,
    ctx: OnceLock<Ctx>,
    runs: Arc<Mutex<Vec<Run>>>,
}

#[derive(Deserialize)]
struct Message {
    session_id: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct SessionOnly {
    #[serde(default)]
    session_id: String,
}

#[derive(Deserialize)]
struct Facts {
    #[serde(default)]
    facts: Value,
}

/// What the model is told about a request it talks about, while it can still change: before the
/// building starts and not ended. None from the building on, where the text goes to the builder.
fn talk_facts(view: &Value) -> Option<Value> {
    let text = |name: &str| view.get(name).and_then(Value::as_str).unwrap_or("").to_owned();
    if !text("ended_as").is_empty() || !matches!(text("stage").as_str(), "arrived" | "assessed" | "planned" | "validated" | "held") {
        return None;
    }
    let points: Vec<Value> =
        view.get("plan").and_then(|plan| plan.get("points")).and_then(Value::as_array).into_iter().flatten().filter_map(|point| point.get("title").cloned()).collect();
    let rated = [text("kind"), text("tier")].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(", ");
    Some(json!({
        "the_request_so_far": text("text"),
        "rated_as": rated,
        "the_plan_so_far": points,
    }))
}

impl Flow {
    pub fn new(env: PluginEnv) -> Self {
        Self { env, ctx: OnceLock::new(), runs: Arc::new(Mutex::new(Vec::new())) }
    }

    pub fn connect(&self, ctx: Ctx) {
        let _ = self.ctx.set(ctx);
    }

    pub fn ctx(&self) -> Result<&Ctx, RpcError> {
        self.ctx.get().ok_or_else(|| RpcError::new(codes::UNAVAILABLE, "the manage flow is not connected to the frame yet"))
    }

    /// The data set as it is now (the live folder's valid files over the shipped ones).
    pub fn data(&self) -> Result<Data, RpcError> {
        let data = Data::load(&self.env).map_err(RpcError::internal)?;
        for problem in &data.problems {
            tracing::warn!(%problem, "a live data file is left out");
        }
        Ok(data)
    }

    pub fn cancel_all(&self) {
        if let Ok(runs) = self.runs.lock() {
            for run in runs.iter() {
                run.cancel.cancel();
            }
        }
    }

    /// Put a run on the list. Its token ends with the caller's (`parent`) and with `cancel`.
    fn begin(&self, session: &str, stage: &str, parent: &CancellationToken) -> Running {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let cancel = parent.child_token();
        if let Ok(mut runs) = self.runs.lock() {
            runs.push(Run { id: id.clone(), session: session.to_owned(), stage: stage.to_owned(), cancel: cancel.clone() });
        }
        Running { runs: self.runs.clone(), id, cancel }
    }

    fn state(&self, session: &str) -> FlowState {
        let current = self.runs.lock().ok().and_then(|runs| runs.iter().rev().find(|run| run.session == session).map(|run| (run.id.clone(), run.stage.clone())));
        FlowState {
            session_id: session.to_owned(),
            running: current.is_some(),
            run_id: current.as_ref().map(|(id, _)| id.clone()),
            stage: current.map(|(_, stage)| stage),
            // Nothing is queued here: a message goes straight to the manage plugin.
            pending_steers: 0,
            pending_followups: 0,
        }
    }

    fn running(&self) -> RunningResult {
        let mut sessions: Vec<String> = self.runs.lock().map(|runs| runs.iter().filter(|run| !run.session.is_empty()).map(|run| run.session.clone()).collect()).unwrap_or_default();
        sessions.sort();
        sessions.dedup();
        RunningResult { sessions }
    }

    async fn journal(&self, session: &str, kind: &str, data: Value) -> Result<(), RpcError> {
        self.ctx()?.call(frame_methods::JOURNAL_APPEND, json!({"session_id": session, "kind": kind, "data": data})).await.map(|_| ())
    }

    /// The id of the request whose session this is.
    async fn request_of(&self, session: &str) -> Result<String, RpcError> {
        // Ended requests too: a person can still write in the session of one that was declined.
        let listed = self.ctx()?.call(manage_methods::REQUEST_LIST, json!({"with_ended": true})).await?;
        let found = listed.get("requests").and_then(Value::as_array).into_iter().flatten().find(|request| request.get("session_id").and_then(Value::as_str) == Some(session));
        let id = found.and_then(|request| request.get("id")).and_then(Value::as_str).filter(|id| !id.is_empty());
        match id {
            Some(id) => Ok(id.to_owned()),
            None => Err(RpcError::not_found(self.data()?.text("no_request", &[]))),
        }
    }

    /// What a person wrote in a request's session: shown as theirs at once, passed to the manage
    /// plugin, and its answer shown as the manager's line. Answers as soon as the message is in
    /// the thread; the rest shows up there.
    async fn turn(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        let message: Message = parse_params(params)?;
        let text = message.text.trim().to_owned();
        if text.is_empty() {
            return Err(RpcError::invalid_params(self.data()?.text("empty_message", &[])));
        }
        let session = message.session_id;
        // Not tied to this call: the call answers at once, the passing on goes on after it.
        let running = self.begin(&session, "say", &CancellationToken::new());
        let run_id = running.id.clone();
        self.journal(&session, kinds::USER_MESSAGE, json!({"text": text, "run_id": run_id})).await?;
        let flow = self.clone();
        tokio::spawn(async move {
            let cancel = running.cancel.clone();
            let said = tokio::select! {
                said = flow.say(&session, &text, &cancel) => said,
                _ = cancel.cancelled() => Err(RpcError::cancelled()),
            };
            let entry = match said {
                Ok(line) => (kinds::ASSISTANT_MESSAGE, json!({"text": line, "run_id": running.id})),
                // Stopped by the person: the message is in the thread, and so is that it was not passed on.
                Err(error) if error.code == codes::CANCELLED => (kinds::NOTE, json!({"text": flow.data().map(|data| data.text("cancelled", &[])).unwrap_or_default(), "run_id": running.id})),
                Err(error) => (kinds::ERROR, json!({"message": error.message, "code": error.code, "run_id": running.id})),
            };
            if let Err(error) = flow.journal(&session, entry.0, entry.1).await {
                tracing::warn!(%error, "the manager's line could not be written to the session");
            }
            drop(running);
        });
        to_value(&TurnResult { run_id, queued_as_steer: false })
    }

    async fn say(&self, session: &str, text: &str, cancel: &CancellationToken) -> Result<String, RpcError> {
        let request = self.request_of(session).await?;
        // The request as it stood before this message: what the model answers from.
        let before = self.ctx()?.call(manage_methods::REQUEST_GET, json!({"id": request})).await.ok();
        let params = to_value(&SayParams { request, text: text.to_owned() })?;
        let answer = self.ctx()?.call(manage_methods::REQUEST_SAY, params).await?;
        let line = answer.get("text").and_then(Value::as_str).unwrap_or("").trim().to_owned();
        let line = if line.is_empty() { self.data()?.text("no_line", &[]) } else { line };
        let Some(facts) = before.as_ref().and_then(talk_facts) else { return Ok(line) };
        match self.reply(&facts, text, cancel).await {
            Ok(reply) => Ok(reply),
            Err(error) if error.code == codes::CANCELLED => Err(error),
            Err(error) => {
                tracing::warn!(error = %error.message, "the model did not answer in the session; the manager's own line is shown");
                Ok(line)
            }
        }
    }

    pub async fn dispatch(self: &Arc<Self>, verb: &str, params: Value, caller: &CancellationToken) -> Result<Value, RpcError> {
        match verb {
            // While a job builds the manage plugin passes the text on as a steer; before that it
            // is added to the request. Which one is its to say, so both verbs take the same road.
            verbs::TURN | verbs::STEER => self.turn(params).await,
            verbs::STATE => {
                let session: SessionOnly = parse_params(params)?;
                to_value(&self.state(&session.session_id))
            }
            verbs::RUNNING => to_value(&self.running()),
            verbs::CANCEL => {
                let session: SessionOnly = parse_params(params)?;
                let mut cancelled = 0;
                if let Ok(runs) = self.runs.lock() {
                    for run in runs.iter().filter(|run| !session.session_id.is_empty() && run.session == session.session_id) {
                        run.cancel.cancel();
                        cancelled += 1;
                    }
                }
                Ok(json!({"cancelled": cancelled}))
            }
            "assess" => {
                let params: AssessParams = parse_params(params)?;
                let running = self.begin(&params.session_id, "assess", caller);
                to_value(&self.assess(params, &running.cancel).await?)
            }
            "match" => {
                let params: MatchParams = parse_params(params)?;
                let running = self.begin("", "match", caller);
                to_value(&self.match_pair(params, &running.cancel).await?)
            }
            "plan" => {
                let params: PlanParams = parse_params(params)?;
                let running = self.begin(&params.session_id, "plan", caller);
                to_value(&self.plan(params, &running.cancel).await?)
            }
            "validate" => {
                let params: ValidateParams = parse_params(params)?;
                let running = self.begin(&params.session_id, "validate", caller);
                to_value(&self.validate(params, &running.cancel).await?)
            }
            "drift" => {
                let params: DriftParams = parse_params(params)?;
                let running = self.begin("", "drift", caller);
                to_value(&self.drift(params, &running.cancel).await?)
            }
            "converge" => {
                let params: ConvergeParams = parse_params(params)?;
                let running = self.begin("", "converge", caller);
                to_value(&self.converge(params, &running.cancel).await?)
            }
            "digest" => {
                let params: Facts = parse_params(params)?;
                let running = self.begin("", "digest", caller);
                Ok(json!({"text": self.digest(&params.facts, &running.cancel).await?}))
            }
            "why" => {
                let params: Facts = parse_params(params)?;
                let running = self.begin("", "why", caller);
                to_value(&self.why(&params.facts, &running.cancel).await?)
            }
            other => Err(RpcError::new(codes::UNKNOWN_METHOD, self.data()?.text("not_in_flow", &[("verb", other)]))),
        }
    }
}
