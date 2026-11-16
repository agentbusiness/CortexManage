//! `cc-train` executable: serve `train@1` to the frame.

use cc_protocol::contracts::lifecycle::{Hello, ProbeReport};
use cc_protocol::{BoxFuture, Call, Ctx, Plugin, PluginEnv, RpcError, run_plugin};
use cc_train::PROVIDES;
use cc_train::service::Train;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct TrainPlugin {
    train: Arc<Train>,
    stop: CancellationToken,
}

impl Plugin for TrainPlugin {
    fn handle(&self, ctx: Ctx, call: Call) -> BoxFuture<'static, Result<Value, RpcError>> {
        self.train.attach(&ctx);
        let train = self.train.clone();
        Box::pin(async move { train.handle(&call.method, call.params).await })
    }

    fn probe(&self, ctx: Ctx) -> BoxFuture<'static, Result<ProbeReport, RpcError>> {
        self.train.attach(&ctx);
        let train = self.train.clone();
        Box::pin(async move { Ok(train.probe().await) })
    }

    fn activate(&self, ctx: Ctx) -> BoxFuture<'static, Result<(), RpcError>> {
        self.train.attach(&ctx);
        self.train.spawn_clock(self.stop.clone());
        Box::pin(async { Ok(()) })
    }

    fn shutdown(&self, _ctx: Ctx) -> BoxFuture<'static, ()> {
        self.stop.cancel();
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
            eprintln!("cc-train: cannot start the async runtime: {error}");
            std::process::exit(1);
        }
    };
    let code = runtime.block_on(async {
        let env = match PluginEnv::from_env() {
            Ok(env) => env,
            Err(error) => {
                eprintln!("cc-train: {error}");
                return 2;
            }
        };
        // It subscribes to nothing: everything it acts on is told to it through its methods.
        let mut hello = Hello::new("cc-train", env!("CARGO_PKG_VERSION"), "train", "train@1");
        hello.provides = PROVIDES.iter().map(|method| (*method).to_owned()).collect();
        let plugin = Arc::new(TrainPlugin { train: Train::new(env.live_dir.clone()), stop: CancellationToken::new() });
        match run_plugin(hello, plugin).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("cc-train: {error}");
                1
            }
        }
    });
    std::process::exit(code);
}
