//! The connection to a runner (docs/specs/01-WORK.md §2): `mesh.link {node}` answers a local
//! address that leads to the runner's frame and the token it takes; the plugin opens a WebSocket
//! to it and speaks the frame's own JSON envelopes as a client, as a window does. One connection
//! per runner, opened again whenever it drops. On it: `runner.*` calls, and the `runner.beat`
//! events after `frame.subscribe`.

use cc_protocol::contracts::frame::methods as frame_methods;
use cc_protocol::contracts::mesh::{Link as MeshLink, methods as mesh_methods};
use cc_protocol::contracts::runner::{Facts, topics as runner_topics};
use cc_protocol::{Ctx, Message, RpcError, codes, parse_params};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_util::sync::CancellationToken;

/// Between two attempts to reach a runner: from this, doubled up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);
/// How long a call waits for a connection that is being opened.
const CONNECT_WAIT: Duration = Duration::from_secs(8);

/// What a link tells the manager.
pub enum LinkEvent {
    /// A job's facts.
    Beat(Box<Facts>),
    /// The runner at this node can be called (again).
    Up(String),
}

type Answer = oneshot::Sender<Result<Value, RpcError>>;

#[derive(Default)]
struct Wire {
    out: Option<mpsc::UnboundedSender<String>>,
    pending: HashMap<u64, Answer>,
}

pub struct Link {
    node: String,
    next: AtomicU64,
    wire: Mutex<Wire>,
    /// Subscribed and ready for calls.
    ready: AtomicBool,
    up: Notify,
    stop: CancellationToken,
}

impl Link {
    fn wire<T>(&self, change: impl FnOnce(&mut Wire) -> T) -> T {
        change(&mut self.wire.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    fn unreachable(&self) -> RpcError {
        RpcError::new(codes::UNAVAILABLE, format!("the runner ({}) cannot be reached", self.node)).retryable()
    }

    /// One request on the open socket, answered within `limit`.
    async fn request(&self, method: &str, params: Value, limit: Duration) -> Result<Value, RpcError> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (answer, wait) = oneshot::channel();
        let text = serde_json::to_string(&Message::Req { id, method: method.to_owned(), params }).map_err(|error| RpcError::internal(error.to_string()))?;
        let sent = self.wire(|wire| match &wire.out {
            Some(out) if out.send(text).is_ok() => {
                wire.pending.insert(id, answer);
                true
            }
            _ => false,
        });
        if !sent {
            return Err(self.unreachable());
        }
        match tokio::time::timeout(limit, wait).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(RpcError::closed()),
            Err(_) => {
                // The runner is told that nobody waits for this answer any more.
                self.wire(|wire| {
                    wire.pending.remove(&id);
                    if let (Some(out), Ok(text)) = (&wire.out, serde_json::to_string(&Message::Cancel { id })) {
                        let _ = out.send(text);
                    }
                });
                Err(RpcError::new(codes::TIMEOUT, format!("the runner did not answer `{method}` in {} s", limit.as_secs())))
            }
        }
    }

    /// Call a method on the runner's frame. Waits a short time for a connection being opened.
    pub async fn call(&self, method: &str, params: Value, limit: Duration) -> Result<Value, RpcError> {
        if !self.ready() {
            let up = self.up.notified();
            tokio::pin!(up);
            up.as_mut().enable();
            if !self.ready() && tokio::time::timeout(CONNECT_WAIT, up).await.is_err() {
                return Err(self.unreachable());
            }
        }
        self.request(method, params, limit).await
    }

    fn drop_wire(&self) {
        self.ready.store(false, Ordering::SeqCst);
        let pending = self.wire(|wire| {
            wire.out = None;
            std::mem::take(&mut wire.pending)
        });
        for (_, answer) in pending {
            let _ = answer.send(Err(RpcError::closed()));
        }
    }
}

pub struct Links {
    links: Mutex<HashMap<String, Arc<Link>>>,
    events: mpsc::UnboundedSender<LinkEvent>,
}

impl Links {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<LinkEvent>) {
        let (events, receive) = mpsc::unbounded_channel();
        (Self { links: Mutex::new(HashMap::new()), events }, receive)
    }

    /// The link to a runner node, opened when it is first wanted and kept open.
    pub fn ensure(&self, ctx: &Ctx, node: &str) -> Arc<Link> {
        let mut links = self.links.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(link) = links.get(node) {
            return link.clone();
        }
        let link = Arc::new(Link {
            node: node.to_owned(),
            next: AtomicU64::new(1),
            wire: Mutex::new(Wire::default()),
            ready: AtomicBool::new(false),
            up: Notify::new(),
            stop: CancellationToken::new(),
        });
        links.insert(node.to_owned(), link.clone());
        tokio::spawn(keep_open(link.clone(), ctx.clone(), self.events.clone()));
        link
    }

    pub fn ready(&self, node: &str) -> bool {
        self.links.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(node).is_some_and(|link| link.ready())
    }

    /// Call the runner at `node` and read its answer as `T`.
    pub async fn call<T: DeserializeOwned>(&self, ctx: &Ctx, node: &str, method: &str, params: Value, limit: Duration) -> Result<T, RpcError> {
        let link = self.ensure(ctx, node);
        let value = link.call(method, params, limit).await?;
        serde_json::from_value(value).map_err(|error| RpcError::new(codes::CONTRACT_VIOLATION, format!("{method}: unexpected answer from the runner: {error}")))
    }

    pub fn stop(&self) {
        for link in self.links.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).values() {
            link.stop.cancel();
        }
    }
}

/// Keep one runner's connection open until the plugin stops.
async fn keep_open(link: Arc<Link>, ctx: Ctx, events: mpsc::UnboundedSender<LinkEvent>) {
    let mut delay = RETRY_MIN;
    while !link.stop.is_cancelled() {
        match serve(&link, &ctx, &events).await {
            // It was open and closed: look for it again soon.
            Ok(()) => delay = RETRY_MIN,
            Err(error) => {
                tracing::debug!(node = %link.node, error = %error.message, "the runner cannot be reached");
                delay = (delay * 2).min(RETRY_MAX);
            }
        }
        link.drop_wire();
        tokio::select! {
            _ = link.stop.cancelled() => break,
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// Percent-encode a token for the query string.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Open the socket, subscribe to the beats and serve until it closes.
async fn serve(link: &Arc<Link>, ctx: &Ctx, events: &mpsc::UnboundedSender<LinkEvent>) -> Result<(), RpcError> {
    let target: MeshLink = parse_params(ctx.call(mesh_methods::LINK, json!({"node": link.node})).await?)?;
    let base = match target.url.strip_prefix("http") {
        Some(rest) => format!("ws{rest}"),
        None => target.url.clone(),
    };
    let url = format!("{}/ws?token={}", base.trim_end_matches('/'), encode(&target.token));
    let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .map_err(|error| RpcError::new(codes::UNAVAILABLE, format!("cannot connect to the runner's frame: {error}")))?;
    let (mut sink, mut stream) = socket.split();
    let (out, mut outgoing) = mpsc::unbounded_channel::<String>();
    link.wire(|wire| wire.out = Some(out.clone()));
    let writer = tokio::spawn(async move {
        while let Some(text) = outgoing.recv().await {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    // The subscription's answer arrives through the read loop below, so it is asked beside it.
    let broken = CancellationToken::new();
    {
        let (link, events, broken) = (link.clone(), events.clone(), broken.clone());
        tokio::spawn(async move {
            match link.request(frame_methods::SUBSCRIBE, json!({"topics": [runner_topics::BEAT]}), Duration::from_secs(20)).await {
                Ok(_) => {
                    link.ready.store(true, Ordering::SeqCst);
                    link.up.notify_waiters();
                    let _ = events.send(LinkEvent::Up(link.node.clone()));
                }
                Err(error) => {
                    tracing::warn!(node = %link.node, error = %error.message, "the runner's frame refused the subscription; connecting again");
                    broken.cancel();
                }
            }
        });
    }
    loop {
        let message = tokio::select! {
            _ = link.stop.cancelled() => break,
            // Without the subscription this connection is of no use.
            _ = broken.cancelled() => break,
            message = stream.next() => message,
        };
        let text = match message {
            Some(Ok(WsMessage::Text(text))) => text,
            Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
            Some(Ok(_)) => continue,
        };
        match serde_json::from_str::<Message>(text.as_str()) {
            Ok(Message::Res { id, ok, result, error }) => {
                if let Some(answer) = link.wire(|wire| wire.pending.remove(&id)) {
                    let outcome = match (ok, error) {
                        (true, _) => Ok(result.unwrap_or(Value::Null)),
                        (false, Some(error)) => Err(error),
                        (false, None) => Err(RpcError::internal("the runner answered an error without saying which")),
                    };
                    let _ = answer.send(outcome);
                }
            }
            Ok(Message::Evt { topic, data }) => {
                if topic == runner_topics::BEAT {
                    match serde_json::from_value::<Facts>(data) {
                        Ok(facts) => {
                            let _ = events.send(LinkEvent::Beat(Box::new(facts)));
                        }
                        Err(error) => tracing::warn!(node = %link.node, %error, "a beat that is not the runner contract's facts; ignored"),
                    }
                }
            }
            // This plugin serves nothing on the link.
            Ok(Message::Req { id, method, .. }) => {
                let reply = Message::Res { id, ok: false, result: None, error: Some(RpcError::unknown_method(&method)) };
                if let Ok(text) = serde_json::to_string(&reply) {
                    let _ = out.send(text);
                }
            }
            Ok(Message::Item { .. }) | Ok(Message::Cancel { .. }) => {}
            Err(error) => tracing::debug!(node = %link.node, %error, "a message that is not an envelope; ignored"),
        }
    }
    writer.abort();
    Ok(())
}
