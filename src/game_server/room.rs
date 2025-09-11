use std::{collections::HashMap, result, sync::Arc};

use actix::{Actor, Context, Handler, Message, Recipient};
use derive_more::{Display, From};
use rand::{RngCore, rng};
use rayon::iter::{ParallelBridge, ParallelIterator};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tracing::{info, warn};

use super::validation::{Password, Username};

const TOKEN_LEN: usize = 16;
const CHANNEL_CAPACITY: usize = 10;

#[derive(Debug, Clone, Error)]
pub enum RoomError {
    #[error("game already started")]
    GameStarted,
    #[error("player '{0}' already exists")]
    PlayerExists(Username),
    #[error("player '{0}' not found in room")]
    PlayerNotFound(Username),
    #[error("player '{0}' already connected")]
    PlayerConnected(Username),
    #[error("player '{0}' already disconnected")]
    PlayerDisconnected(Username),
    #[error("incorrect password")]
    IncorrectPassword,
    #[error("player '{0}' unauthorized")]
    Unauthorized(Username),
    #[error("unauthenticated")]
    Unauthenticated,
}

pub type Result<T> = result::Result<T, RoomError>;

#[derive(Debug)]
pub struct Room {
    tokens: HashMap<Token, Username>,
    password: Option<Password>,
    players: HashMap<Username, Player>,
    host: Username,
    rounds: u32,
    phase: Option<Phase>,
}

pub type Token = [u8; TOKEN_LEN];

#[derive(Clone, Debug, Default)]
struct Player {
    points: i32,
    channel_handle: Option<Sender<Arc<ServerMessage>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Phase {
    Bidding,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlayerDescriptor {
    username: Username,
    points: i32,
}

#[derive(Serialize, Debug, Clone)]
pub enum ServerMessage {
    Join {
        username: Username,
    },
    Leave {
        username: Username,
    },
    Connect {
        username: Username,
    },
    Disconnect {
        username: Username,
    },
    Welcome {
        username: Username,
        players: Vec<PlayerDescriptor>,
        host: Username,
        phase: Option<Phase>,
    },
    Chat {
        username: Username,
        text: Arc<str>,
    },
}

#[derive(Deserialize, Debug)]
enum PlayerMessage {
    Chat { text: Arc<str> },
}

impl PlayerMessage {
    pub fn sign(self, username: Username) -> SignedPlayerMessage {
        SignedPlayerMessage {
            username,
            message: self,
        }
    }
}

#[derive(Debug, Message)]
#[rtype(result = "()")]
struct SignedPlayerMessage {
    username: Username,
    message: PlayerMessage,
}

#[derive(Debug, Message)]
#[rtype(result = "Result<()>")]
pub struct Config {
    token: Token,
}

#[derive(Debug, Message)]
#[rtype(result = "Result<Token>")]
pub struct Join {
    username: Username,
    password: Option<Password>,
}

#[derive(Debug, Message)]
#[rtype(result = "Result<Receiver<Arc<ServerMessage>>>")]
pub struct Connect {
    token: Token,
}

#[derive(Debug, Message)]
#[rtype(result = "Result<()>")]
pub struct Disconnect {
    username: Username,
}

fn generate_token() -> Token {
    let mut token = [0; TOKEN_LEN];
    rng().fill_bytes(&mut token);
    token
}

impl Room {
    pub fn create(host: Username, password: Option<Password>) -> (Self, [u8; TOKEN_LEN]) {
        let mut room = Self {
            tokens: HashMap::new(),
            password,
            players: HashMap::new(),
            host: host.clone(),
            rounds: 0,
            phase: None,
        };

        // unwrap: there should be no players yet, so this should not fail
        let token = room.add_player(host).unwrap();
        (room, token)
    }

    fn is_host(&self, username: Username) -> bool {
        username == self.host
    }

    fn authenticate(&self, token: Token) -> Result<Username> {
        self.tokens
            .get(&token)
            .cloned()
            .ok_or(RoomError::Unauthenticated)
    }

    fn add_player(&mut self, username: Username) -> Result<Token> {
        if self.players.contains_key(&username) {
            Err(RoomError::PlayerExists(username.clone()))
        } else {
            self.players.insert(username.clone(), Player::default());
            let mut token = generate_token();
            while self.tokens.contains_key(&token) {
                token = generate_token();
            }
            Ok(token)
        }
    }

    fn send_one(&self, username: Username, message: Arc<ServerMessage>) -> Result<()> {
        // TODO maybe we should handle this erroring
        let _ = self
            .players
            .get(&username)
            .ok_or(RoomError::PlayerNotFound(username.clone()))?
            .channel_handle
            .as_ref()
            .ok_or(RoomError::PlayerDisconnected(username.clone()))?
            .blocking_send(message);
        Ok(())
    }

    fn send_all(&self, message: Arc<ServerMessage>) -> Result<()> {
        // TODO do something with these results
        let _ = self
            .players
            .values()
            .par_bridge()
            .filter_map(|p| {
                p.channel_handle
                    .as_ref()
                    .and_then(|sender| Some(sender.blocking_send(message.clone())))
            })
            .collect::<Vec<_>>();
        Ok(())
    }
}

impl Actor for Room {
    type Context = Context<Room>;
}

impl Handler<SignedPlayerMessage> for Room {
    type Result = ();

    fn handle(&mut self, msg: SignedPlayerMessage, _ctx: &mut Self::Context) -> Self::Result {
        match msg.message {
            PlayerMessage::Chat { text } => self
                .send_all(Arc::new(ServerMessage::Chat {
                    username: msg.username,
                    text,
                }))
                .expect("sending shouldn't fail"),
        }
    }
}

impl Handler<Join> for Room {
    type Result = Result<Token>;

    fn handle(&mut self, msg: Join, _ctx: &mut Self::Context) -> Self::Result {
        if self.phase.is_some() {
            Err(RoomError::GameStarted)
        } else if self.password != msg.password {
            Err(RoomError::IncorrectPassword)
        } else {
            let token = self.add_player(msg.username.clone())?;
            self.send_all(Arc::new(ServerMessage::Join {
                username: msg.username,
            }));

            Ok(token)
        }
    }
}

impl Handler<Connect> for Room {
    type Result = Result<Receiver<Arc<ServerMessage>>>;

    fn handle(&mut self, msg: Connect, _ctx: &mut Self::Context) -> Self::Result {
        let username = self.authenticate(msg.token)?;

        let channel_handle = &mut self
            .players
            .get_mut(&username)
            // unwrap: a token mapping to an unknown username would violate room invariant
            .unwrap()
            .channel_handle;

        if channel_handle.is_some() {
            warn!("player {username} tried to connect while connected");
            Err(RoomError::PlayerConnected(username))
        } else {
            let (sender, receiver) = channel::<Arc<ServerMessage>>(CHANNEL_CAPACITY);
            *channel_handle = Some(sender);

            let _ = self.send_one(
                username.clone(),
                ServerMessage::Welcome {
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
                }
                .into(),
            );
            self.send_all(Arc::new(ServerMessage::Connect {
                username: username.clone(),
            }));

            info!("player {username} connected");
            Ok(receiver)
        }
    }
}

impl Handler<Disconnect> for Room {
    type Result = Result<()>;

    fn handle(&mut self, msg: Disconnect, _ctx: &mut Self::Context) -> Self::Result {
        self.players
            .get_mut(&msg.username)
            .ok_or(RoomError::PlayerNotFound(msg.username.clone()))?
            .channel_handle
            .take()
            .ok_or(RoomError::PlayerDisconnected(msg.username.clone()))?;

        self.send_all(Arc::new(ServerMessage::Disconnect {
            username: msg.username,
        }));
        Ok(())
    }
}
