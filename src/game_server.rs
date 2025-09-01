use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, stream::StreamExt};
use rand::{Rng, random_range, rng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::{
    Mutex,
    broadcast::{Receiver, Sender},
    mpsc,
};

use crate::game::{PlayerMessage, Room, ServerMessage};

const NUM_CODE_CHARS: usize = 36;
const CODE_CHARS: [char; NUM_CODE_CHARS] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
];
const CODE_LEN: usize = 4;

type ServerState = Arc<Mutex<HashMap<Arc<str>, Arc<Mutex<Room>>>>>;

#[derive(Error, Debug, Serialize, Clone)]
enum ServerError {
    #[error("room not found")]
    RoomNotFound,
    #[error("room code missing")]
    MissingRoomCode,
    #[error("token missing")]
    MissingToken,
    #[error("token invalid")]
    InvalidToken,
    #[error("room started")]
    RoomStarted,
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        (
            match self {
                Self::RoomNotFound => StatusCode::NOT_FOUND,
                Self::MissingRoomCode => StatusCode::BAD_REQUEST,
                Self::MissingToken => StatusCode::UNAUTHORIZED,
                Self::InvalidToken => StatusCode::FORBIDDEN,
                Self::RoomStarted => StatusCode::CONFLICT,
            },
            self,
        )
            .into_response()
    }
}

pub fn init_game_server() -> Router {
    let rooms = HashMap::<Arc<str>, Arc<Mutex<Room>>>::new();

    Router::new()
        .route("/rooms", get(|| async {}))
        .route("/create", post(handle_create))
        .route("/join", post(|| async {}))
        .route("/ws", get(websocket_handler))
        .with_state(Arc::new(Mutex::new(rooms)))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CreateRequest {
    username: Arc<str>,
    password: Option<Arc<str>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CreateResponse {
    code: Arc<str>,
    token: Arc<str>,
}

fn generate_code() -> Arc<str> {
    let mut code = String::with_capacity(CODE_LEN);

    for _ in 0..CODE_LEN {
        code.push(CODE_CHARS[rng().random_range(..NUM_CODE_CHARS)]);
    }

    code.into()
}

async fn handle_create(
    State(rooms): State<ServerState>,
    Json(payload): Json<CreateRequest>,
) -> impl IntoResponse {
    let mut code = generate_code();
    while rooms.lock().await.contains_key(&code) {
        code = generate_code();
    }

    let (room, host_token) = Room::create(payload.username, payload.password);
    rooms
        .lock()
        .await
        .insert(code.clone(), Arc::new(Mutex::new(room)));

    Json(CreateResponse {
        code,
        token: STANDARD.encode(host_token).into(),
    })
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    State(rooms): State<ServerState>,
) -> Result<impl IntoResponse, ServerError> {
    let room = rooms
        .lock()
        .await
        .get(
            params
                .get("room")
                .ok_or(ServerError::MissingRoomCode)?
                .as_str(),
        )
        .ok_or(ServerError::RoomNotFound)?
        .clone();

    let name = room
        .lock()
        .await
        .authenticate(
            STANDARD
                .decode(headers.get("token").ok_or(ServerError::MissingToken)?)
                .map_err(|_| ServerError::InvalidToken)?,
        )
        .ok_or(ServerError::InvalidToken)?;

    Ok(ws.on_upgrade(|socket| websocket(socket, room, name)))
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
