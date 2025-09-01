use std::{collections::HashMap, sync::Arc};

use axum::extract::ws::{Message, WebSocket};
use bimap::BiMap;
use futures_util::{SinkExt, future::join_all, stream::SplitSink, task::waker};
use rand::{Rng, RngCore, SeedableRng, rng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc::{self, Receiver, Sender};

const TOKEN_LEN: usize = 16;

#[derive(Error, Debug)]
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
}

#[derive(Debug)]
pub struct Room {
    tokens: HashMap<[u8; TOKEN_LEN], Arc<str>>,
    password: Option<Arc<str>>,
    players: HashMap<Arc<str>, Player>,
    host: Arc<str>,
    phase: Option<Phase>,
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

        (room, token)
    }

    fn create_token(&mut self, name: Arc<str>) -> [u8; TOKEN_LEN] {
        let mut token = generate_token();
        while self.tokens.contains_key(&token) {
            token = generate_token();
        }
        self.tokens.insert(token, name);
        token
    }

    pub async fn handle_message(&mut self, name: Arc<str>, message: PlayerMessage) {
        match message {
            PlayerMessage::Chat { text } => self.send_all(ServerMessage::Chat { name, text }).await,
            PlayerMessage::Start => unimplemented!(),
        };
    }

    pub async fn connect(
        &mut self,
        name: Arc<str>,
    ) -> Result<Receiver<Arc<ServerMessage>>, RoomError> {
        let channel_handle = &mut self
            .players
            .get_mut(&name)
            .ok_or(RoomError::PlayerNotFound(name.clone()))?
            .channel_handle;

        if channel_handle.is_some() {
            Err(RoomError::PlayerConnected(name))
        } else {
            let (sender, receiver) = mpsc::channel::<Arc<ServerMessage>>(1);
            *channel_handle = Some(sender);

            let _ = self
                .send_one(
                    name.clone(),
                    Arc::new(ServerMessage::Welcome {
                        name: name.clone(),
                        players: self
                            .players
                            .iter()
                            .map(|(n, p)| PlayerDescriptor {
                                name: n.clone(),
                                points: p.points,
                            })
                            .collect(),
                        host: self.host.clone(),
                        phase: self.phase.clone(),
                    }),
                )
                .await;
            self.send_all(ServerMessage::Connect { name }).await;
            Ok(receiver)
        }
    }

    pub async fn disconnect(&mut self, name: Arc<str>) -> Result<(), RoomError> {
        self.players
            .get_mut(&name)
            .ok_or(RoomError::PlayerNotFound(name.clone()))?
            .channel_handle
            .take()
            .ok_or(RoomError::PlayerDisconnected(name.clone()))?;

        self.send_all(ServerMessage::Disconnect { name }).await;
        Ok(())
    }

    pub fn authenticate<T>(&self, token: T) -> Option<Arc<str>>
    where
        T: AsRef<[u8]>,
    {
        self.tokens.get(token.as_ref()).cloned()
    }

    pub async fn join(&mut self, name: Arc<str>) -> Result<[u8; TOKEN_LEN], RoomError> {
        if self.phase.is_some() {
            Err(RoomError::GameStarted)
        } else if self.players.contains_key(&name) {
            Err(RoomError::PlayerExists(name))
        } else {
            self.players.insert(name.clone(), Player::default());

            self.send_all(ServerMessage::Join { name: name.clone() })
                .await;
            Ok(self.create_token(name))
        }
    }

    pub async fn leave(&mut self, name: Arc<str>) -> Result<(), RoomError> {
        todo!();

        //self.players
        //    .remove(&name)
        //    .ok_or(RoomError::PlayerNotFound(name.clone()))?;
        //
        //self.tokens.remove_by_right(&name);
        //
        //self.send_all(ServerMessage::Leave { name }).await;
        //Ok(())
    }

    async fn send_one(
        &mut self,
        recipient: Arc<str>,
        message: Arc<ServerMessage>,
    ) -> Result<(), RoomError> {
        let _ = self
            .players
            .get_mut(&recipient)
            .ok_or(RoomError::PlayerNotFound(recipient.clone()))?
            .channel_handle
            .as_mut()
            .ok_or(RoomError::PlayerDisconnected(recipient))?
            .send(message)
            .await;
        Ok(())
    }

    async fn send_all(&mut self, message: ServerMessage) {
        let message = Arc::new(message);
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
struct PlayerDescriptor {
    name: Arc<str>,
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
        name: Arc<str>,
    },
    Leave {
        name: Arc<str>,
    },
    Connect {
        name: Arc<str>,
    },
    Disconnect {
        name: Arc<str>,
    },
    Welcome {
        name: Arc<str>,
        players: Vec<PlayerDescriptor>,
        host: Arc<str>,
        phase: Option<Phase>,
    },
    Chat {
        name: Arc<str>,
        text: Arc<str>,
    },
}
