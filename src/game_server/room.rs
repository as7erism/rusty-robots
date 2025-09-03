use std::{collections::HashMap, sync::Arc};

use axum::extract::ws::{Message, WebSocket};
use bimap::BiMap;
use futures_util::{SinkExt, future::join_all, stream::SplitSink, task::waker};
use rand::{Rng, RngCore, SeedableRng, rng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc::{self, Receiver, Sender};

const TOKEN_LEN: usize = 16;
const CHANNEL_CAPACITY: usize = 10;

#[derive(Error, Debug, Clone, Serialize)]
pub enum RoomError {
    #[error("game already started")]
    GameStarted,
    #[error("player '{0}' already exists")]
    PlayerExists(Arc<str>),
    #[error("player '{0}' not found in room")]
    PlayerNotFound(Arc<str>),
    #[error("player '{0}' already connected")]
    PlayerConnected(Arc<str>),
    #[error("player '{0}' already disconnected")]
    PlayerDisconnected(Arc<str>),
    #[error("incorrect password")]
    IncorrectPassword,
}

#[derive(Debug)]
pub struct Room {
    tokens: HashMap<[u8; TOKEN_LEN], Arc<str>>,
    password: Option<Arc<str>>,
    players: HashMap<Arc<str>, Player>,
    host: Arc<str>,
    phase: Option<Phase>,
    websocket_channel: Sender<(Arc<str>, PlayerMessage)>,
}

fn generate_token() -> [u8; TOKEN_LEN] {
    let mut token = [0; TOKEN_LEN];
    rng().fill_bytes(&mut token);
    token
}

impl Room {
    pub fn create(host: Arc<str>, password: Option<Arc<str>>) -> (Self, [u8; TOKEN_LEN]) {
        let mut room = Self {
            tokens: HashMap::new(),
            password,
            players: HashMap::new(),
            host: host.clone(),
            phase: None,
        };

        room.players.insert(host.clone(), Player::default());
        let token = room.create_token(host);

        tokio::spawn(|| async {})(room, token)
    }

    fn create_token(&mut self, username: Arc<str>) -> [u8; TOKEN_LEN] {
        let mut token = generate_token();
        while self.tokens.contains_key(&token) {
            token = generate_token();
        }
        self.tokens.insert(token, username);
        token
    }

    pub async fn handle_message(&mut self, username: Arc<str>, message: PlayerMessage) {
        match message {
            PlayerMessage::Chat { text } => {
                self.send_all(Arc::new(ServerMessage::Chat { username, text }))
                    .await
            }
            PlayerMessage::Start => unimplemented!(),
        };
    }

    pub async fn connect(
        &mut self,
        username: Arc<str>,
    ) -> Result<Receiver<Arc<ServerMessage>>, RoomError> {
        tracing::info!("player {username} connecting");
        let channel_handle = &mut self
            .players
            .get_mut(&username)
            .ok_or(RoomError::PlayerNotFound(username.clone()))?
            .channel_handle;

        if channel_handle.is_some() {
            tracing::warn!("player {username} tried to connect while connected");
            Err(RoomError::PlayerConnected(username))
        } else {
            let (sender, receiver) = mpsc::channel::<Arc<ServerMessage>>(CHANNEL_CAPACITY);
            *channel_handle = Some(sender);

            let _ = self
                .send_one(
                    username.clone(),
                    Arc::new(ServerMessage::Welcome {
                        username: username.clone(),
                        players: self
                            .players
                            .iter()
                            .map(|(n, p)| PlayerDescriptor {
                                username: n.clone(),
                                points: p.points,
                            })
                            .collect(),
                        host: self.host.clone(),
                        phase: self.phase.clone(),
                    }),
                )
                .await;
            self.send_all(Arc::new(ServerMessage::Connect { username }))
                .await;
            Ok(receiver)
        }
    }

    pub async fn disconnect(&mut self, username: Arc<str>) -> Result<(), RoomError> {
        tracing::info!("player {username} disconnecting");

        self.players
            .get_mut(&username)
            .ok_or(RoomError::PlayerNotFound(username.clone()))?
            .channel_handle
            .take()
            .ok_or(RoomError::PlayerDisconnected(username.clone()))?;

        self.send_all(Arc::new(ServerMessage::Disconnect { username }))
            .await;
        Ok(())
    }

    pub fn authenticate<T>(&self, token: T) -> Option<Arc<str>>
    where
        T: AsRef<[u8]>,
    {
        self.tokens.get(token.as_ref()).cloned()
    }

    pub async fn join(
        &mut self,
        username: Arc<str>,
        password: Option<Arc<str>>,
    ) -> Result<[u8; TOKEN_LEN], RoomError> {
        if self.phase.is_some() {
            Err(RoomError::GameStarted)
        } else if self.players.contains_key(&username) {
            Err(RoomError::PlayerExists(username))
        } else if self.password != password {
            Err(RoomError::IncorrectPassword)
        } else {
            self.players.insert(username.clone(), Player::default());

            self.send_all(Arc::new(ServerMessage::Join {
                username: username.clone(),
            }))
            .await;
            Ok(self.create_token(username))
        }
    }

    pub async fn leave(&mut self, username: Arc<str>) -> Result<(), RoomError> {
        todo!();

        //self.players
        //    .remove(&username)
        //    .ok_or(RoomError::PlayerNotFound(username.clone()))?;
        //
        //self.tokens.remove_by_right(&username);
        //
        //self.send_all(ServerMessage::Leave { username }).await;
        //Ok(())
    }

    async fn send_one(
        &mut self,
        recipient: Arc<str>,
        message: Arc<ServerMessage>,
    ) -> Result<(), RoomError> {
        tracing::info!("sending message {message:?} to {recipient}");
        let _ = self
            .players
            .get_mut(&recipient)
            .ok_or(RoomError::PlayerNotFound(recipient.clone()))?
            .channel_handle
            .as_mut()
            .ok_or(RoomError::PlayerDisconnected(recipient.clone()))?
            .send(message.clone())
            .await;
        Ok(())
    }

    async fn send_all(&mut self, message: Arc<ServerMessage>) {
        tracing::info!("sending message {message:?} to all");
        join_all(
            self.players
                .values_mut()
                .filter_map(|player| Some(player.channel_handle.as_mut()?.send(message.clone()))),
        )
        .await;
    }
}

#[derive(Debug)]
struct Player {
    points: i32,
    channel_handle: Option<Sender<Arc<ServerMessage>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlayerDescriptor {
    username: Arc<str>,
    points: i32,
}

impl Default for Player {
    fn default() -> Self {
        Player {
            points: 0,
            channel_handle: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Phase {
    Bidding,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PlayerMessage {
    Chat { text: Arc<str> },
    Start,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMessage {
    Join {
        username: Arc<str>,
    },
    Leave {
        username: Arc<str>,
    },
    Connect {
        username: Arc<str>,
    },
    Disconnect {
        username: Arc<str>,
    },
    Welcome {
        username: Arc<str>,
        players: Vec<PlayerDescriptor>,
        host: Arc<str>,
        phase: Option<Phase>,
    },
    Chat {
        username: Arc<str>,
        text: Arc<str>,
    },
}
