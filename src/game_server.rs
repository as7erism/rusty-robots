use axum::{
    Json, Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, stream::StreamExt};
use rand::{Rng, rng};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::game::{PlayerMessage, Room, RoomError};

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
    #[error("name missing")]
    MissingName,
    #[error("name invalid")]
    InvalidName,
    #[error("password invalid")]
    InvalidPassword,
    #[error("token missing")]
    MissingToken,
    #[error("token invalid")]
    InvalidToken,
    #[error("room error: {0}")]
    RoomError(#[from] RoomError),
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        (
            match self {
                Self::RoomNotFound => StatusCode::NOT_FOUND,
                Self::MissingName
                | Self::InvalidName
                | Self::InvalidPassword => StatusCode::BAD_REQUEST,
                Self::MissingToken => StatusCode::UNAUTHORIZED,
                Self::InvalidToken => StatusCode::FORBIDDEN,
                Self::RoomError(ref err) => match err {
                    RoomError::GameStarted => StatusCode::CONFLICT,
                    _ => todo!(),
                },
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
        .route("/rooms/create", post(handle_create))
        .route("/rooms/{code}", post(handle_join))
        .route("/rooms/{code}/ws", get(websocket_handler))
        .with_state(Arc::new(Mutex::new(rooms)))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CreateResponse {
    code: Arc<str>,
    token: Arc<str>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JoinResponse {
    token: Arc<str>,
}

fn generate_code() -> Arc<str> {
    let mut code = String::with_capacity(CODE_LEN);

    for _ in 0..CODE_LEN {
        code.push(CODE_CHARS[rng().random_range(..NUM_CODE_CHARS)]);
    }

    code.into()
}

async fn handle_join(
    headers: HeaderMap,
    Path(code): Path<String>,
    State(rooms): State<ServerState>,
) -> Result<impl IntoResponse, ServerError> {
    let room = rooms
        .lock()
        .await
        .get(&Arc::<str>::from(code))
        .ok_or(ServerError::RoomNotFound)?
        .clone();

    let token = room
        .lock()
        .await
        .join(
            headers
                .get("RR-Name")
                .ok_or(ServerError::MissingName)?
                .to_str()
                .map_err(|_| ServerError::InvalidName)?
                .into(),
            match headers.get("RR-Password") {
                Some(password) => Some(
                    password
                        .to_str()
                        .map_err(|_| ServerError::InvalidPassword)?
                        .into(),
                ),
                None => None,
            },
        )
        .await?;

    Ok(Json(JoinResponse {
        token: STANDARD.encode(token).into(),
    }))
}

async fn handle_create(
    headers: HeaderMap,
    State(rooms): State<ServerState>,
) -> Result<impl IntoResponse, ServerError> {
    let mut code = generate_code();
    while rooms.lock().await.contains_key(&code) {
        code = generate_code();
    }

    let (room, host_token) = Room::create(
        headers
            .get("RR-Name")
            .ok_or(ServerError::MissingName)?
            .to_str()
            .map_err(|_| ServerError::InvalidName)?
            .into(),
        match headers.get("RR-Password") {
            Some(password) => Some(
                password
                    .to_str()
                    .map_err(|_| ServerError::InvalidPassword)?
                    .into(),
            ),
            None => None,
        },
    );

    rooms
        .lock()
        .await
        .insert(code.clone(), Arc::new(Mutex::new(room)));

    Ok(Json(CreateResponse {
        code,
        token: STANDARD.encode(host_token).into(),
    }))
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Path(code): Path<String>,
    State(rooms): State<ServerState>,
) -> Result<impl IntoResponse, ServerError> {
    let room = rooms
        .lock()
        .await
        .get(code.as_str())
        .ok_or(ServerError::RoomNotFound)?
        .clone();

    let name = room
        .lock()
        .await
        .authenticate(
            STANDARD
                .decode(
                    headers
                        .get("Authorization")
                        .ok_or(ServerError::MissingToken)?
                        .to_str()
                        .map_err(|_| ServerError::InvalidToken)?
                        .to_string()
                        .strip_prefix("Bearer ")
                        .ok_or(ServerError::InvalidToken)?
                )
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
