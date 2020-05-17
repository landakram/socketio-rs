extern crate pretty_env_logger;
#[macro_use]
extern crate log;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;
use warp::http::StatusCode;
use warp::Filter;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let socket_store = crate::store::new();

    let base = warp::path("ws");

    // engine.io protocol branches here for websocket connect, so need to fix that.
    // It still uses the query params:
    //
    // - If a sid is passed, then it looks up the engine.io socket and attempts a transport upgrade.
    //   Basically, just marking the socket as upgraded to the new transport, if the old transport
    //   allows that.
    //
    // - If no sid, does a engine.io handshake over websockets.
    let ws = base
        .and(warp::query::<UpgradeArgs>())
        .and(warp::ws())
        .and(with_state(socket_store.clone()))
        .map(|args, ws: warp::ws::Ws, socket_store| {
            ws.on_upgrade(|ws: warp::ws::WebSocket| handle_conn(ws, args, socket_store))
        });

    let handshake = base
        .and(warp::get())
        .and(warp::query::<HandshakeArgs>())
        .and(with_state(socket_store.clone()))
        .and_then(on_handshake);

    let polling = base
        .and(warp::get())
        .and(warp::query::<SessionArgs>())
        .and(with_state(socket_store.clone()))
        .and_then(on_poll);

    let data = base
        .and(warp::post())
        .and(warp::query::<SessionArgs>())
        .and(with_state(socket_store.clone()))
        .map(move |args, socket_store| on_data(args, socket_store));

    let hello = warp::path("hello").map(|| "Hello, world!");

    let routes = ws.or(polling).or(handshake).or(data).or(hello);

    let addr: SocketAddr = "127.0.0.1:8080".parse().expect("Invalid host and port");
    info!("Running on {}", addr);
    warp::serve(routes).run(addr).await;
}

fn with_state<T: std::marker::Sync + std::marker::Send>(
    state: Arc<T>,
) -> impl Filter<Extract = (Arc<T>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

async fn on_handshake(
    args: HandshakeArgs,
    socket_store: Arc<impl crate::store::SocketStore + std::fmt::Debug>,
) -> Result<impl warp::Reply, std::convert::Infallible> {
    let sid = Uuid::new_v4().to_string();
    let sock = crate::socket::new(sid.clone());
    socket_store.set(sid.clone(), sock).await;
    info!("{:?}", socket_store);
    let res = HandshakeResponse {
        sid: sid,
        t: "handshake".to_string(),
    };
    Ok(warp::reply::json(&res))
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct HandshakeResponse {
    sid: String,
    t: String,
}

async fn on_poll(
    args: SessionArgs,
    socket_store: Arc<impl crate::store::SocketStore>,
) -> Result<Box<dyn warp::Reply>, warp::Rejection> {
    let sid = args.sid;
    match socket_store.get(sid).await {
        Some(socket) => {
            let res = PollResponse {
                sid: socket.sid.clone(),
                t: "poll".to_string(),
            };
            Ok(Box::new(warp::reply::json(&res)))
        }
        None => Ok(Box::new(StatusCode::NOT_FOUND)),
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct PollResponse {
    sid: String,
    t: String,
}

fn on_data(args: SessionArgs, socket_store: Arc<impl crate::store::SocketStore>) -> String {
    "on_data".to_string()
}

mod store {
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[async_trait]
    pub trait SocketStore: Send + Sync {
        async fn get(&self, sid: String) -> Option<Arc<crate::socket::Socket>>;
        async fn set(&self, sid: String, socket: crate::socket::Socket);
    }

    #[derive(Debug)]
    struct InMemorySocketStore {
        sockets: RwLock<HashMap<String, Arc<crate::socket::Socket>>>,
    }

    #[async_trait]
    impl SocketStore for InMemorySocketStore {
        async fn get(&self, sid: String) -> Option<Arc<crate::socket::Socket>> {
            let s = self.sockets.read().await;
            s.get(&sid).map(|sock| sock.clone())
        }

        async fn set(&self, sid: String, socket: crate::socket::Socket) {
            let mut s = self.sockets.write().await;
            s.insert(sid, Arc::new(socket));
        }
    }

    pub fn new() -> Arc<impl SocketStore + std::fmt::Debug> {
        Arc::new(InMemorySocketStore {
            sockets: RwLock::new(HashMap::new()),
        })
    }
}

mod socket {
    #[derive(Debug)]
    pub struct Socket {
        pub sid: String,
        transport: crate::transport::Transport,
    }

    impl Socket {}

    pub fn new(sid: String) -> Socket {
        Socket {
            sid: sid,
            transport: crate::transport::Transport {},
        }
    }
}

mod transport {
    #[derive(Debug)]
    pub struct Transport {}

    impl Transport {}
}

#[derive(Deserialize, Debug, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum TransportType {
    Polling,
    Websocket,
}

#[derive(Deserialize, Debug, Eq, PartialEq)]
struct UpgradeArgs {
    transport: Option<TransportType>,
    sid: Option<String>,
    j: Option<String>,
    b64: Option<bool>,
}

#[derive(Deserialize, Debug, Eq, PartialEq)]
struct SessionArgs {
    transport: Option<TransportType>,
    sid: String,
    j: Option<String>,
    b64: Option<bool>,
}

#[derive(Deserialize, Debug, Eq, PartialEq)]
struct HandshakeArgs {
    transport: TransportType,
    j: Option<String>,
    b64: Option<bool>,
}

async fn handle_conn(
    ws: warp::ws::WebSocket,
    args: UpgradeArgs,
    socket_store: Arc<impl crate::store::SocketStore>,
) {
    let (mut tx, mut rx) = ws.split();

    while let Some(msg) = rx.next().await {
        match msg {
            Err(e) => error!("websocket error: {}", e),
            Ok(v) => {
                if v.is_text() || v.is_binary() {
                    info!("echoing: {:?}", v);

                    let res = tx.send(v).await;
                    if let Err(e) = res {
                        error!("echo error: {}", e)
                    }
                }
            }
        }
    }
}
