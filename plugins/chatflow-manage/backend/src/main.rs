//! `cc-chatflow-manage`: CortexManage's chat flow (`chatflow-manage`, `chatflow@1`).
//!
//! Everything the manager asks a model goes through here, one question per call: ASSESS, MATCH,
//! PLAN, VALIDATE, DRIFT, CONVERGE, WHY (each a JSON answer held to a schema, decoded greedily and
//! checked by code) and DIGEST (two lines of text nothing is decided from). The manage plugin calls
//! them and owns what happens with the answers. The flow keeps no records of its own.

mod data;
mod flow;
mod llm;
mod plan;
mod schemas;
mod stages;

use cc_protocol::contracts::lifecycle::{Hello, ProbeCheck, ProbeReport};
use cc_protocol::contracts::manage::Tier;
use cc_protocol::{BoxFuture, Call, Ctx, Plugin, PluginEnv, RpcError, run_plugin};
use data::Data;
use flow::{CATEGORY, Flow};
use schemas::{FILES, POINTS, TASKS};
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;

struct FlowPlugin {
    flow: Arc<Flow>,
}

impl Plugin for FlowPlugin {
    fn handle(&self, ctx: Ctx, call: Call) -> BoxFuture<'static, Result<Value, RpcError>> {
        self.flow.connect(ctx);
        let flow = self.flow.clone();
        Box::pin(async move {
            let verb = call.method.strip_prefix(CATEGORY).and_then(|rest| rest.strip_prefix('.')).unwrap_or(&call.method).to_owned();
            flow.dispatch(&verb, call.params, &call.cancel).await
        })
    }

    fn probe(&self, ctx: Ctx) -> BoxFuture<'static, Result<ProbeReport, RpcError>> {
        self.flow.connect(ctx.clone());
        Box::pin(async move { Ok(ProbeReport::from_checks(probe(&ctx.env))) })
    }

    fn shutdown(&self, _ctx: Ctx) -> BoxFuture<'static, ()> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.cancel_all() })
    }
}

/// Load every prompt, schema, text and stage setting (the live folder's over the shipped ones) and
/// show that each stage's schema gives an answer the contract's type can hold: an answer of the
/// schema's shape is built and read the way a model's answer is. No model is called.
fn probe(env: &PluginEnv) -> Vec<ProbeCheck> {
    let started = Instant::now();
    let check = |name: &str, outcome: Result<String, String>, started: Instant| {
        let ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok(detail) => ProbeCheck { name: name.to_owned(), ok: true, detail, ms },
            Err(detail) => ProbeCheck { name: name.to_owned(), ok: false, detail, ms },
        }
    };
    let data = match Data::load(env) {
        Ok(data) => data,
        Err(error) => return vec![check("data", Err(error), started)],
    };
    let (prompts, shapes) = data.counts();
    let mut detail = format!("{}: {prompts} prompts, {shapes} schemas, {} stages", data.label(), data::STAGES.len());
    for problem in &data.problems {
        detail.push_str(&format!("; {problem}"));
    }
    let mut checks = vec![check("data", Ok(detail), started)];

    let files = vec!["src/sample.rs".to_owned()];
    let one = vec!["1".to_owned()];
    // (stage, how its sample answer is read into the contract's type)
    type Read = fn(&Value, &Data) -> Result<(), String>;
    let stages: &[(&str, Read)] = &[
        ("ASSESS", |value, _| stages::read_assess(value, "CT-0001").map(|_| ())),
        ("MATCH", |value, _| stages::read_match(value).map(|_| ())),
        ("PLAN", |value, data| {
            let plan = plan::read_plan(value, "CT-0001", Tier::Minor, data)?;
            let walk = plan.points.first().and_then(|point| point.walks.first()).ok_or("the plan's schema gives a task no walk")?;
            if walk.do_.is_empty() || walk.see.is_empty() || walk.method.is_empty() {
                return Err("the plan's schema gives a walk no `do`, `see` or `method`".into());
            }
            Ok(())
        }),
        ("VALIDATE", |value, _| stages::read_validate(value).map(|_| ())),
        ("DRIFT", |value, _| stages::read_drift(value).map(|_| ())),
        ("CONVERGE", |value, _| {
            let answer = stages::read_converge(value)?;
            if answer.extra.is_empty() || answer.missing.is_empty() {
                return Err("the schema's `extra` or `missing` items lack `file`, `point` or `why`".into());
            }
            Ok(())
        }),
        ("WHY", |value, _| stages::read_why(value).map(|_| ())),
    ];
    for (stage, read) in stages {
        let started = Instant::now();
        let outcome = (|| {
            let config = data.stage(stage).map_err(|error| error.message)?;
            if data.system(config).trim().is_empty() {
                return Err("its prompt is empty".to_owned());
            }
            let shape = data.shape(config).ok_or("it has no schema")?.filled(FILES, &files).filled(POINTS, &one).filled(TASKS, &one);
            let sample = shape.sample();
            shape.check(&sample)?;
            read(&sample, &data)?;
            let grammar = schemas::grammar(&shape);
            Ok(format!("prompt and schema read; a sample answer fits the contract ({} grammar rules)", grammar.lines().count()))
        })();
        checks.push(check(&stage.to_lowercase(), outcome, started));
    }
    // The two stages without a schema: their prompts, and the tag each claims.
    for stage in ["DISCOVER", "DIGEST", "REPLY"] {
        let started = Instant::now();
        let outcome = data.stage(stage).map_err(|error| error.message).and_then(|config| {
            if data.system(config).trim().is_empty() { Err("its prompt is empty".to_owned()) } else { Ok(format!("prompt read; claims a `{}` model", config.tag)) }
        });
        checks.push(check(&stage.to_lowercase(), outcome, started));
    }
    checks
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("CORTEX_LOG").unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cc-chatflow-manage: cannot start the async runtime: {error}");
            std::process::exit(1);
        }
    };
    let code = runtime.block_on(async {
        let env = match PluginEnv::from_env() {
            Ok(env) => env,
            Err(error) => {
                eprintln!("cc-chatflow-manage: {error}");
                return 2;
            }
        };
        // The shipped data is compiled in: a build whose own data does not validate must not start.
        if let Err(error) = Data::embedded() {
            eprintln!("cc-chatflow-manage: the shipped data is not valid: {error}");
            return 2;
        }
        let flow = Arc::new(Flow::new(env));
        let mut hello = Hello::new("cc-chatflow-manage", env!("CARGO_PKG_VERSION"), CATEGORY, "chatflow@1");
        hello.provides = flow::VERBS.iter().map(|verb| format!("{CATEGORY}.{verb}")).collect();
        let plugin = Arc::new(FlowPlugin { flow: flow.clone() });
        let mut terminate = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("cc-chatflow-manage: cannot handle SIGTERM: {error}");
                return 1;
            }
        };
        let outcome = tokio::select! {
            outcome = run_plugin(hello, plugin) => outcome,
            _ = terminate.recv() => Ok(()),
            _ = tokio::signal::ctrl_c() => Ok(()),
        };
        flow.cancel_all();
        match outcome {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("cc-chatflow-manage: {error}");
                1
            }
        }
    });
    std::process::exit(code);
}
