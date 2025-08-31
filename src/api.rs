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
};

#[derive(Error, Debug)]
enum RoomError {
    #[error("player '{0}' not found in room")]
    PlayerNotFound(Arc<str>),
    #[error("player '{0}' already connected")]
    PlayerAlreadyConnected(Arc<str>),
    #[error("player '{0}' already disconnected")]
    PlayerAlreadyDisconnected(Arc<str>),
}

#[derive(Debug)]
struct Room {
    channel_handle: Sender<ServerMessage>,
    players: HashMap<Arc<str>, Player>,
    game_state: Option<GameState>,
}

impl Room {
    fn handle_message(&mut self, name: Arc<str>, message: PlayerMessage) {
        match message {
            PlayerMessage::Chat { text } => {
                let _ = self.send(ServerMessage::Chat { name, text });
            }
        }
    }

    fn handle_join(&mut self, name: Arc<str>) -> Result<(), RoomError> {
        let player = self
            .players
            .get_mut(&name)
            .ok_or(RoomError::PlayerNotFound(name.clone()))?;

        player
            .is_connected
            .then(|| player.is_connected = false)
            .ok_or(RoomError::PlayerAlreadyConnected(name.clone()))?;

        self.send(ServerMessage::Join { name });
        Ok(())
    }

    fn handle_leave(&mut self, name: Arc<str>) -> Result<(), RoomError> {
        let player = self
            .players
            .get_mut(&name)
            .ok_or(RoomError::PlayerNotFound(name.clone()))?;

        player
            .is_connected
            .then(|| player.is_connected = false)
            .ok_or(RoomError::PlayerAlreadyDisconnected(name.clone()))?;

        self.send(ServerMessage::Leave { name });
        Ok(())
    }

    fn send(&self, message: ServerMessage) {
        // this "fails" if all receiver handles have been dropped. we don't care how many receiver
        // handles there are left, because we want to keep the game open even if everyone has
        // disconnected
        let _ = self.channel_handle.send(message);
    }

    fn subscribe(&self) -> Receiver<ServerMessage> {
        self.channel_handle.subscribe()
    }
}

#[derive(Debug, Clone, Copy)]
struct Player {
    points: i32,
    is_connected: bool,
}

#[derive(Debug, Clone, Copy)]
enum GameState {}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum PlayerMessage {
    Chat { text: Arc<str> },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum ServerMessage {
    Join { name: Arc<str> },
    Leave { name: Arc<str> },
    Chat { name: Arc<str>, text: Arc<str> },
}

pub fn api() -> Router {
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
    let mut channel_receiver = room.lock().await.subscribe();

    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = channel_receiver.recv().await {
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

    let moving_name = name.clone();
    let moving_room = room.clone();
    let mut receive_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(json))) = socket_receiver.next().await {
            let message = serde_json::from_str::<PlayerMessage>(json.as_str())
                .expect("parsing player message failed");

            moving_room
                .lock()
                .await
                .handle_message(moving_name.clone(), message);
        }
    });

    room.lock().await.handle_join(name.clone()).unwrap();

    tokio::select! {
        _ = &mut send_task => receive_task.abort(),
        _ = &mut receive_task => send_task.abort(),
    };

    room.lock().await.handle_leave(name).unwrap();
}
