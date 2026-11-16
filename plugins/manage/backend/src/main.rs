//! `cc-manage` executable: serve `manage@1` to the frame.

use cc_manage::service::Manager;
use cc_manage::{PROVIDES, SUBSCRIBES};
use cc_protocol::contracts::lifecycle::{Hello, ProbeReport};
use cc_protocol::{BoxFuture, Call, Ctx, Plugin, PluginEnv, RpcError, run_plugin};
use serde_json::Value;
use std::sync::Arc;

struct ManagePlugin {
    manager: Arc<Manager>,
}

impl Plugin for ManagePlugin {
    fn handle(&self, ctx: Ctx, call: Call) -> BoxFuture<'static, Result<Value, RpcError>> {
        self.manager.attach(&ctx);
        let manager = self.manager.clone();
        Box::pin(async move { manager.handle(&call.method, call.params).await })
    }

    fn probe(&self, ctx: Ctx) -> BoxFuture<'static, Result<ProbeReport, RpcError>> {
        self.manager.attach(&ctx);
        let manager = self.manager.clone();
        Box::pin(async move { Ok(manager.probe().await) })
    }

    fn activate(&self, ctx: Ctx) -> BoxFuture<'static, Result<(), RpcError>> {
        self.manager.attach(&ctx);
        // The records are read in the background: activation does not wait for the memory plugin.
        self.manager.activate();
        Box::pin(async { Ok(()) })
    }

    fn event(&self, ctx: Ctx, topic: String, data: Value) -> BoxFuture<'static, ()> {
        self.manager.attach(&ctx);
        let manager = self.manager.clone();
        Box::pin(async move { manager.event(&topic, data).await })
    }

    fn shutdown(&self, _ctx: Ctx) -> BoxFuture<'static, ()> {
        self.manager.shutdown();
        Box::pin(async {})
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("CORTEX_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cc-manage: cannot start the async runtime: {error}");
            std::process::exit(1);
        }
    };
    let code = runtime.block_on(async {
        let env = match PluginEnv::from_env() {
            Ok(env) => env,
            Err(error) => {
                eprintln!("cc-manage: {error}");
                return 2;
            }
        };
        if let Err(error) = std::fs::create_dir_all(&env.data_dir) {
            eprintln!("cc-manage: cannot create {}: {error}", env.data_dir.display());
            return 1;
        }
        let mut hello = Hello::new("cc-manage", env!("CARGO_PKG_VERSION"), "manage", "manage@1");
        hello.provides = PROVIDES.iter().map(|method| (*method).to_owned()).collect();
        hello.subscribes = SUBSCRIBES.iter().map(|topic| (*topic).to_owned()).collect();
        let plugin = Arc::new(ManagePlugin { manager: Manager::new(env.home.clone(), env.data_dir.clone(), env.live_dir.clone()) });
        match run_plugin(hello, plugin).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("cc-manage: {error}");
                1
            }
        }
    });
    std::process::exit(code);
}
