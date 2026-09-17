use aurenfall_contracts::{FrontierManifestAck, MineIntent, MoveIntent};
use aurenfall_core::{ConnectionId, SessionId};
use crate::{RealtimeConnectionSender, ReliableConnectionSender};

#[derive(Debug)]
pub enum TransportSessionEvent {
    Admitted {
        connection_id: ConnectionId,
        session_id: SessionId,
        realtime: RealtimeConnectionSender,
        reliable: ReliableConnectionSender,
    },
    MoveIntent {
        connection_id: ConnectionId,
        intent: MoveIntent,
    },
    MineIntent {
        connection_id: ConnectionId,
        intent: MineIntent,
    },
    FrontierManifestAck {
        connection_id: ConnectionId,
        ack: FrontierManifestAck,
    },
    Disconnected {
        connection_id: ConnectionId,
        session_id: SessionId,
    },
}

impl PartialEq for TransportSessionEvent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Admitted {
                    connection_id: c1,
                    session_id: s1,
                    ..
                },
                Self::Admitted {
                    connection_id: c2,
                    session_id: s2,
                    ..
                },
            ) => c1 == c2 && s1 == s2,
            (
                Self::MoveIntent {
                    connection_id: c1,
                    intent: i1,
                },
                Self::MoveIntent {
                    connection_id: c2,
                    intent: i2,
                },
            ) => c1 == c2 && i1 == i2,
            (
                Self::MineIntent {
                    connection_id: c1,
                    intent: i1,
                },
                Self::MineIntent {
                    connection_id: c2,
                    intent: i2,
                },
            ) => c1 == c2 && i1 == i2,
            (
                Self::FrontierManifestAck {
                    connection_id: c1,
                    ack: a1,
                },
                Self::FrontierManifestAck {
                    connection_id: c2,
                    ack: a2,
                },
            ) => c1 == c2 && a1 == a2,
            (
                Self::Disconnected {
                    connection_id: c1,
                    session_id: s1,
                },
                Self::Disconnected {
                    connection_id: c2,
                    session_id: s2,
                },
            ) => c1 == c2 && s1 == s2,
            _ => false,
        }
    }
}