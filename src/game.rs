use std::{collections::HashMap, sync::Arc};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, future::join_all, stream::SplitSink};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast::{Receiver, Sender};

#[derive(Error, Debug)]
pub enum RoomError {
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

    pub fn handle_message(&mut self, name: Arc<str>, message: PlayerMessage) {
        match message {
            PlayerMessage::Chat { text } => self.send(ServerMessage::Chat { name, text }),
            PlayerMessage::Start => {
                if name == self.host && self.game_state.is_none() {
                    self.game_state = Some(GameState::Bidding);
                }
            }
        }
    }

    pub fn handle_join(&mut self, name: Arc<str>) -> Result<(), RoomError> {
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

    pub fn handle_leave(&mut self, name: Arc<str>) -> Result<(), RoomError> {
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

    pub fn subscribe(&self) -> Receiver<ServerMessage> {
        self.channel_handle.subscribe()
    }

    fn send(&self, message: ServerMessage) {
        // this "fails" if all receiver handles have been dropped. we don't care how many receiver
        // handles there are left, because we want to keep the game open even if everyone has
        // disconnected
        let _ = self.channel_handle.send(message);
    }

    async fn send_one(
        &mut self,
        recipient: Arc<str>,
        message: ServerMessage,
    ) -> Result<(), RoomError> {
        Ok(self
            .players
            .get_mut(&recipient)
            .ok_or(RoomError::PlayerNotFound(recipient.clone()))?
            .socket_sink
            .as_mut()
            .ok_or(RoomError::PlayerAlreadyDisconnected(recipient))?
            .send(message.to_socket_message())
            .await
            .unwrap())
    }

    async fn send_all(&mut self, message: ServerMessage) -> Result<(), RoomError> {
        join_all(self.players.values_mut().filter_map(|player| {
            Some(
                player
                    .socket_sink
                    .as_mut()?
                    .send(message.to_socket_message()),
            )
        }))
        .await;
        Ok(())
    }
}

#[derive(Debug)]
struct Player {
    points: i32,
    socket_sink: Option<SplitSink<WebSocket, Message>>,
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

impl ServerMessage {
    fn to_socket_message(&self) -> Message {
        Message::text(serde_json::to_string(self).expect("failed to parse server message"))
    }
}
