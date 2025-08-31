use axum::{
    Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use futures_util::{SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::{
    Mutex,
    broadcast::{Receiver, Sender},
    mpsc,
};

use crate::game::{PlayerMessage, Room, ServerMessage};

pub fn init_game_server() -> Router {
    let rooms = HashMap::<Arc<str>, Arc<Mutex<Room>>>::new();

    Router::new()
        .route("/rooms", get(|| async {}))
        .route("/join", post(|| async {}))
        .route("/ws", get(websocket_handler))
        .with_state(Arc::new(rooms))
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    cookies: CookieJar,
    State(state): State<Arc<HashMap<Arc<str>, Arc<Mutex<Room>>>>>,
) -> impl IntoResponse {
    // TODO check token
    let room = state.get("abcd").unwrap().clone();
    ws.on_upgrade(|socket| websocket(socket, room, "aster".into()))
}

async fn websocket(socket: WebSocket, room: Arc<Mutex<Room>>, name: Arc<str>) {
    let (mut socket_sender, mut socket_receiver) = socket.split();
    let mut channel_receiver = room
        .lock()
        .await
        .connect(name.clone())
        .await
        .expect("player not found");

    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = channel_receiver.recv().await {
            if socket_sender
                .send(Message::text(
                    serde_json::to_string(&msg).expect("parsing message failed"),
                ))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let name2 = name.clone();
    let room2 = room.clone();
    let mut receive_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(json))) = socket_receiver.next().await {
            let message = serde_json::from_str::<PlayerMessage>(json.as_str())
                .expect("parsing player message failed");

            room2
                .lock()
                .await
                .handle_message(name2.clone(), message)
                .await;
        }
    });

    tokio::select! {
        _ = &mut send_task => receive_task.abort(),
        _ = &mut receive_task => send_task.abort(),
    };

    room.lock().await.disconnect(name).await.unwrap();
}
