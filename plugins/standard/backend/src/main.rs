//! `cc-standard` executable: serve `standard@1` to the frame.

use cc_protocol::contracts::lifecycle::{Hello, ProbeReport};
use cc_protocol::{BoxFuture, Call, Ctx, Plugin, PluginEnv, RpcError, run_plugin};
use cc_standard::PROVIDES;
use cc_standard::service::Standard;
use serde_json::Value;
use std::sync::Arc;

struct StandardPlugin {
    standard: Arc<Standard>,
}

impl Plugin for StandardPlugin {
    fn handle(&self, ctx: Ctx, call: Call) -> BoxFuture<'static, Result<Value, RpcError>> {
        self.standard.attach(&ctx);
        let standard = self.standard.clone();
        Box::pin(async move { standard.handle(&call.method, call.params).await })
    }

    fn probe(&self, _ctx: Ctx) -> BoxFuture<'static, Result<ProbeReport, RpcError>> {
        let standard = self.standard.clone();
        Box::pin(async move { Ok(standard.probe().await) })
    }

    fn activate(&self, ctx: Ctx) -> BoxFuture<'static, Result<(), RpcError>> {
        // The records are read back from here on, before the first request needs them.
        self.standard.attach(&ctx);
        Box::pin(async { Ok(()) })
    }
}

fn main() {
    cc_stats_kit::start("CORTEX_LOG", "info", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cc-standard: cannot start the async runtime: {error}");
            std::process::exit(1);
        }
    };
    let code = runtime.block_on(async {
        let env = match PluginEnv::from_env() {
            Ok(env) => env,
            Err(error) => {
                eprintln!("cc-standard: {error}");
                return 2;
            }
        };
        if let Err(error) = std::fs::create_dir_all(&env.data_dir) {
            eprintln!("cc-standard: cannot create {}: {error}", env.data_dir.display());
            return 1;
        }
        let mut hello = Hello::new("cc-standard", env!("CARGO_PKG_VERSION"), "standard", "standard@1");
        hello.provides = PROVIDES.iter().map(|m| (*m).to_owned()).collect();
        let plugin = Arc::new(StandardPlugin { standard: Standard::new(env.data_dir.clone(), env.live_dir.clone()) });
        match run_plugin(hello, plugin).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("cc-standard: {error}");
                1
            }
        }
    });
    std::process::exit(code);
}
