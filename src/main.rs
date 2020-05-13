extern crate pretty_env_logger;
#[macro_use]
extern crate log;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::net::SocketAddr;
use warp::Filter;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let base = warp::path("ws");

    // engine.io protocol branches here for websocket connect, so need to fix that.
    // It still uses the query params:
    // - If a sid is passed, then it looks up the engine.io socket and attempts a transport upgrade.
    // - If no sid, does a engine.io handshake over websockets.
    let ws = base
        .and(warp::ws())
        .map(|ws: warp::ws::Ws| ws.on_upgrade(handle_conn));

    let handshake = base
        .and(warp::get())
        .and(warp::query::<HandshakeArgs>())
        .map(move |args: HandshakeArgs| format!("{:?}", args));

    let polling = base
        .and(warp::get())
        .and(warp::query::<SessionArgs>())
        .map(move |args: SessionArgs| format!("{:?}", args));

    let hello = warp::path("hello").map(|| "Hello, world!");

    let routes = ws.or(polling).or(handshake).or(hello);

    let addr: SocketAddr = "127.0.0.1:8080".parse().expect("Invalid host and port");
    info!("Running on {}", addr);
    warp::serve(routes).run(addr).await;
}

#[derive(Deserialize, Debug, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum TransportType {
    Polling,
    Websocket,
}

#[derive(Deserialize, Debug, Eq, PartialEq)]
struct UpgradeArgs {}

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

struct Transport {}

impl Transport {}

struct Socket {}

impl Socket {}

async fn handle_conn(ws: warp::ws::WebSocket) {
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
