use std::{io, time::Duration};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::session::Session;

/// A Client has disconnected.
#[derive(Message)]
pub struct Disconnected {
    pub session: Session,
    pub entity: Entity,
    pub reason: DisconnectReason,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum DisconnectReason {
    /// The client's TCPStream was closed.
    Quit,

    /// The server forced a disconnection.
    Kicked { reason: String },

    /// An administrator banned the player.
    Banned {
        reason: String,
        duration: Option<Duration>,
    },

    /// No messages were received from this player for some time.
    Timeout,

    /// The player sent a message that violated a protocol expectation.
    ProtocolViolation { explanation: String },

    /// The server shut down.
    ServerShutdown,

    /// An IO Error was emitted by the OS.
    IoError { kind: String },
}

impl From<io::ErrorKind> for DisconnectReason {
    fn from(value: io::ErrorKind) -> Self {
        Self::IoError {
            kind: format!("{value}"),
        }
    }
}

/// A connectionw as established.
#[derive(Message)]
pub struct Connected {
    /// The Session assigned to the connection.
    pub session: Session,

    /// The entity created for the TCP/UDP Transports.
    pub entity: Entity,
}
