use std::{collections::HashMap, sync::Arc};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, future::join_all, stream::SplitSink};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc::{self, Receiver, Sender};

#[derive(Error, Debug)]
pub enum RoomError {
    #[error("player '{0}' already exists")]
    PlayerAlreadyExists(Arc<str>),
    #[error("player '{0}' not found in room")]
    PlayerNotFound(Arc<str>),
    #[error("player '{0}' already connected")]
    PlayerAlreadyConnected(Arc<str>),
    #[error("player '{0}' already disconnected")]
    PlayerAlreadyDisconnected(Arc<str>),
}

#[derive(Debug)]
pub struct Room {
    channel_handle: Sender<ServerMessage>,
    players: HashMap<Arc<str>, Player>,
    host: Arc<str>,
    game_state: Option<GameState>,
}

impl Room {
    pub fn new(
        channel_handle: Sender<ServerMessage>,
        players: HashMap<Arc<str>, Player>,
        host: Arc<str>,
    ) -> Result<Self, RoomError> {
        if !players.contains_key(&host) {
            Err(RoomError::PlayerNotFound(host))
        } else {
            Ok(Self {
                channel_handle,
                players,
                host,
                game_state: None,
            })
        }
    }

    pub async fn handle_message(&mut self, name: Arc<str>, message: PlayerMessage) {
        match message {
            PlayerMessage::Chat { text } => self.send_all(ServerMessage::Chat { name, text }).await,
            PlayerMessage::Start => {
                if name == self.host && self.game_state.is_none() {
                    self.game_state = Some(GameState::Bidding);
                }
            }
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
            Err(RoomError::PlayerAlreadyConnected(name))
        } else {
            let (sender, receiver) = mpsc::channel::<Arc<ServerMessage>>(1);
            channel_handle.insert(sender);

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
            .ok_or(RoomError::PlayerAlreadyDisconnected(name.clone()))?;

        self.send_all(ServerMessage::Disconnect { name }).await;
        Ok(())
    }

    pub fn join(&mut self, name: Arc<str>) -> Result<(), RoomError> {
        if self.players.contains_key(&name) {
            Err(RoomError::PlayerAlreadyExists(name))
        } else {
            self.players.insert(name.clone(), Player::default());

            self.send_all(ServerMessage::Join { name });
            Ok(())
        }
    }

    pub fn leave(&mut self, name: Arc<str>) -> Result<(), RoomError> {
        self.players
            .remove(&name)
            .ok_or(RoomError::PlayerNotFound(name.clone()))?;

        self.send_all(ServerMessage::Leave { name });
        Ok(())
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
            .ok_or(RoomError::PlayerAlreadyDisconnected(recipient))?
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

impl Default for Player {
    fn default() -> Self {
        Player {
            points: 0,
            channel_handle: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum GameState {
    Bidding,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PlayerMessage {
    Chat { text: Arc<str> },
    Start,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMessage {
    Join { name: Arc<str> },
    Leave { name: Arc<str> },
    Connect { name: Arc<str> },
    Disconnect { name: Arc<str> },
    Chat { name: Arc<str>, text: Arc<str> },
}
