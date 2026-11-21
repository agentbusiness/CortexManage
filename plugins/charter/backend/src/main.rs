//! `cc-charter` executable: serve `charter@1` to the frame.

use cc_charter::PROVIDES;
use cc_charter::service::Charters;
use cc_protocol::contracts::lifecycle::{Hello, ProbeReport};
use cc_protocol::{BoxFuture, Call, Ctx, Plugin, PluginEnv, RpcError, run_plugin};
use serde_json::Value;
use std::sync::Arc;

struct CharterPlugin {
    charters: Arc<Charters>,
}

impl Plugin for CharterPlugin {
    fn handle(&self, ctx: Ctx, call: Call) -> BoxFuture<'static, Result<Value, RpcError>> {
        self.charters.attach(&ctx);
        let charters = self.charters.clone();
        Box::pin(async move { charters.handle(&call.method, call.params).await })
    }

    fn probe(&self, _ctx: Ctx) -> BoxFuture<'static, Result<ProbeReport, RpcError>> {
        let report = self.charters.probe();
        Box::pin(async move { Ok(report) })
    }

    fn activate(&self, ctx: Ctx) -> BoxFuture<'static, Result<(), RpcError>> {
        self.charters.attach(&ctx);
        Box::pin(async { Ok(()) })
    }

    /// Nothing is handed over: the graph is the record. The writes still on their way are
    /// finished first, so the next version reads everything this one wrote.
    fn export_state(&self, _ctx: Ctx) -> BoxFuture<'static, Result<Value, RpcError>> {
        let charters = self.charters.clone();
        Box::pin(async move {
            charters.settle().await;
            Ok(Value::Null)
        })
    }

    fn shutdown(&self, _ctx: Ctx) -> BoxFuture<'static, ()> {
        let charters = self.charters.clone();
        Box::pin(async move { charters.settle().await })
    }
}

fn main() {
    cc_stats_kit::start("CORTEX_LOG", "info", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cc-charter: cannot start the async runtime: {error}");
            std::process::exit(1);
        }
    };
    let code = runtime.block_on(async {
        let env = match PluginEnv::from_env() {
            Ok(env) => env,
            Err(error) => {
                eprintln!("cc-charter: {error}");
                return 2;
            }
        };
        let mut hello = Hello::new("cc-charter", env!("CARGO_PKG_VERSION"), "charter", "charter@1");
        hello.provides = PROVIDES.iter().map(|m| (*m).to_owned()).collect();
        let plugin = Arc::new(CharterPlugin { charters: Charters::new(env.live_dir.clone()) });
        match run_plugin(hello, plugin).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("cc-charter: {error}");
                1
            }
        }
    });
    std::process::exit(code);
}
