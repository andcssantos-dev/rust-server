use aurenfall_contracts::MessageKind;
use aurenfall_core::ConnectionId;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{DatagramFrameError, encode_datagram_frame};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeDatagram {
    pub kind: MessageKind,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
enum RealtimeBackend {
    Quinn {
        connection: quinn::Connection,
        maximum_datagram_bytes: usize,
    },
    Channel {
        tx: mpsc::Sender<RealtimeDatagram>,
    },
}

#[derive(Debug, Clone)]
pub struct RealtimeConnectionSender {
    connection_id: ConnectionId,
    backend: RealtimeBackend,
}

impl RealtimeConnectionSender {
    pub(crate) fn new(
        connection_id: ConnectionId,
        connection: quinn::Connection,
        maximum_datagram_bytes: usize,
    ) -> Self {
        Self {
            connection_id,
            backend: RealtimeBackend::Quinn {
                connection,
                maximum_datagram_bytes,
            },
        }
    }

    pub fn bounded_channel(
        connection_id: ConnectionId,
        capacity: usize,
    ) -> Result<(Self, mpsc::Receiver<RealtimeDatagram>), RealtimeEgressConfigError> {
        if capacity == 0 {
            return Err(RealtimeEgressConfigError::ZeroCapacity);
        }
        let (tx, rx) = mpsc::channel(capacity);
        Ok((
            Self {
                connection_id,
                backend: RealtimeBackend::Channel { tx },
            },
            rx,
        ))
    }

    #[must_use]
    pub const fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    pub fn try_send(&self, kind: MessageKind, payload: &[u8]) -> Result<(), RealtimeSendError> {
        match &self.backend {
            RealtimeBackend::Quinn {
                connection,
                maximum_datagram_bytes,
            } => {
                let frame = encode_datagram_frame(kind, payload, *maximum_datagram_bytes)?;
                connection
                    .send_datagram(frame.into())
                    .map_err(|source| RealtimeSendError::Transport {
                        connection_id: self.connection_id,
                        source,
                    })
            }
            RealtimeBackend::Channel { tx } => tx
                .try_send(RealtimeDatagram {
                    kind,
                    payload: payload.to_vec(),
                })
                .map_err(|error| match error {
                    mpsc::error::TrySendError::Full(_) => RealtimeSendError::ChannelFull {
                        connection_id: self.connection_id,
                    },
                    mpsc::error::TrySendError::Closed(_) => RealtimeSendError::ChannelClosed {
                        connection_id: self.connection_id,
                    },
                }),
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeEgressConfigError {
    #[error("test/channel realtime egress capacity must be greater than zero")]
    ZeroCapacity,
}

#[derive(Debug, Error)]
pub enum RealtimeSendError {
    #[error(transparent)]
    Frame(#[from] DatagramFrameError),
    #[error("realtime datagram send failed for connection {connection_id:?}: {source}")]
    Transport {
        connection_id: ConnectionId,
        #[source]
        source: quinn::SendDatagramError,
    },
    #[error("realtime test/channel egress is full for connection {connection_id:?}")]
    ChannelFull { connection_id: ConnectionId },
    #[error("realtime test/channel egress is closed for connection {connection_id:?}")]
    ChannelClosed { connection_id: ConnectionId },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_channel_capability_is_scoped_to_one_connection() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) = RealtimeConnectionSender::bounded_channel(ConnectionId(7), 2)?;
        assert_eq!(sender.connection_id(), ConnectionId(7));

        sender.try_send(MessageKind::SelfMovementSnapshot, &[1, 2, 3])?;
        assert_eq!(
            receiver.try_recv()?,
            RealtimeDatagram {
                kind: MessageKind::SelfMovementSnapshot,
                payload: vec![1, 2, 3],
            }
        );
        Ok(())
    }

    #[test]
    fn bounded_channel_capability_applies_backpressure() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, _receiver) = RealtimeConnectionSender::bounded_channel(ConnectionId(7), 1)?;

        sender.try_send(MessageKind::SelfMovementSnapshot, &[1])?;
        assert!(matches!(
            sender.try_send(MessageKind::SelfMovementSnapshot, &[2]),
            Err(RealtimeSendError::ChannelFull { .. })
        ));
        Ok(())
    }
}
