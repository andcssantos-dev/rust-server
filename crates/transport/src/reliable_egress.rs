use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use aurenfall_contracts::MessageKind;
use aurenfall_core::ConnectionId;
use quinn::VarInt;
use thiserror::Error;
use tokio::{
    sync::{Mutex, mpsc},
    time::timeout,
};

use crate::{FrameIoError, write_single_frame};

const RELIABLE_DELIVERY_FAILED_CODE: u32 = 0x1004;
const RELIABLE_DELIVERY_FAILED_REASON: &[u8] = b"reliable control delivery failed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReliableControlFrame {
    pub kind: MessageKind,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReliableCapabilitySendOutcome {
    Sent,
    UnsupportedProtocolMinor { negotiated: u16, required: u16 },
}

#[derive(Debug, Clone)]
enum ReliableBackend {
    Quinn { connection: quinn::Connection },
    Channel { tx: mpsc::Sender<ReliableControlFrame> },
}

#[derive(Debug, Clone)]
pub struct ReliableConnectionSender {
    connection_id: ConnectionId,
    negotiated_protocol_minor: u16,
    maximum_payload_bytes: usize,
    send_timeout: Duration,
    send_gate: Arc<Mutex<()>>,
    failed_closed: Arc<AtomicBool>,
    backend: ReliableBackend,
}

impl ReliableConnectionSender {
    pub fn new(
        connection_id: ConnectionId,
        connection: quinn::Connection,
        maximum_payload_bytes: usize,
        send_timeout: Duration,
    ) -> Result<Self, ReliableEgressConfigError> {
        validate_config(maximum_payload_bytes, send_timeout)?;
        Ok(Self {
            connection_id,
            negotiated_protocol_minor: 0,
            maximum_payload_bytes,
            send_timeout,
            send_gate: Arc::new(Mutex::new(())),
            failed_closed: Arc::new(AtomicBool::new(false)),
            backend: ReliableBackend::Quinn { connection },
        })
    }

    pub fn bounded_channel(
        connection_id: ConnectionId,
        capacity: usize,
        maximum_payload_bytes: usize,
        send_timeout: Duration,
    ) -> Result<(Self, mpsc::Receiver<ReliableControlFrame>), ReliableEgressConfigError> {
        if capacity == 0 {
            return Err(ReliableEgressConfigError::ZeroCapacity);
        }
        validate_config(maximum_payload_bytes, send_timeout)?;
        let (tx, rx) = mpsc::channel(capacity);
        Ok((
            Self {
                connection_id,
                negotiated_protocol_minor: 0,
                maximum_payload_bytes,
                send_timeout,
                send_gate: Arc::new(Mutex::new(())),
                failed_closed: Arc::new(AtomicBool::new(false)),
                backend: ReliableBackend::Channel { tx },
            },
            rx,
        ))
    }

    #[must_use]
    pub const fn with_negotiated_protocol_minor(mut self, negotiated_protocol_minor: u16) -> Self {
        self.negotiated_protocol_minor = negotiated_protocol_minor;
        self
    }

    #[must_use]
    pub const fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn negotiated_protocol_minor(&self) -> u16 {
        self.negotiated_protocol_minor
    }

    #[must_use]
    pub const fn supports_protocol_minor(&self, minimum_protocol_minor: u16) -> bool {
        self.negotiated_protocol_minor >= minimum_protocol_minor
    }

    pub async fn send_if_supported(
        &self,
        minimum_protocol_minor: u16,
        kind: MessageKind,
        payload: &[u8],
    ) -> Result<ReliableCapabilitySendOutcome, ReliableSendError> {
        if !self.supports_protocol_minor(minimum_protocol_minor) {
            return Ok(ReliableCapabilitySendOutcome::UnsupportedProtocolMinor {
                negotiated: self.negotiated_protocol_minor,
                required: minimum_protocol_minor,
            });
        }

        self.send(kind, payload).await?;
        Ok(ReliableCapabilitySendOutcome::Sent)
    }

    pub async fn send(&self, kind: MessageKind, payload: &[u8]) -> Result<(), ReliableSendError> {
        if self.failed_closed.load(Ordering::Acquire) {
            return Err(ReliableSendError::FailClosed {
                connection_id: self.connection_id,
            });
        }

        let _guard = self.send_gate.lock().await;
        if self.failed_closed.load(Ordering::Acquire) {
            return Err(ReliableSendError::FailClosed {
                connection_id: self.connection_id,
            });
        }

        let result = self.send_attempt(kind, payload).await;
        if result.is_err() {
            self.fail_close();
        }
        result
    }

    async fn send_attempt(&self, kind: MessageKind, payload: &[u8]) -> Result<(), ReliableSendError> {
        if payload.len() > self.maximum_payload_bytes {
            return Err(ReliableSendError::PayloadTooLarge {
                actual: payload.len(),
                maximum: self.maximum_payload_bytes,
            });
        }

        timeout(self.send_timeout, self.send_inner(kind, payload))
            .await
            .map_err(|_| ReliableSendError::TimedOut {
                connection_id: self.connection_id,
            })?
    }

    async fn send_inner(&self, kind: MessageKind, payload: &[u8]) -> Result<(), ReliableSendError> {
        match &self.backend {
            ReliableBackend::Quinn { connection } => {
                let mut stream =
                    connection
                        .open_uni()
                        .await
                        .map_err(|source| ReliableSendError::OpenStream {
                            connection_id: self.connection_id,
                            source,
                        })?;
                write_single_frame(&mut stream, kind, payload, self.maximum_payload_bytes).await?;
                Ok(())
            }
            ReliableBackend::Channel { tx } => tx
                .send(ReliableControlFrame {
                    kind,
                    payload: payload.to_vec(),
                })
                .await
                .map_err(|_| ReliableSendError::ChannelClosed {
                    connection_id: self.connection_id,
                }),
        }
    }

    pub fn fail_close(&self) {
        if self.failed_closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let ReliableBackend::Quinn { connection } = &self.backend {
            connection.close(
                VarInt::from_u32(RELIABLE_DELIVERY_FAILED_CODE),
                RELIABLE_DELIVERY_FAILED_REASON,
            );
        }
    }
}

fn validate_config(
    maximum_payload_bytes: usize,
    send_timeout: Duration,
) -> Result<(), ReliableEgressConfigError> {
    if maximum_payload_bytes == 0 {
        return Err(ReliableEgressConfigError::ZeroMaximumPayloadBytes);
    }
    if send_timeout.is_zero() {
        return Err(ReliableEgressConfigError::ZeroSendTimeout);
    }
    Ok(())
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ReliableEgressConfigError {
    #[error("test/channel reliable egress capacity must be greater than zero")]
    ZeroCapacity,
    #[error("reliable egress maximum payload bytes must be greater than zero")]
    ZeroMaximumPayloadBytes,
    #[error("reliable egress send timeout must be greater than zero")]
    ZeroSendTimeout,
}

#[derive(Debug, Error)]
pub enum ReliableSendError {
    #[error("reliable control payload {actual} exceeds configured maximum {maximum}")]
    PayloadTooLarge { actual: usize, maximum: usize },
    #[error("reliable control send timed out for connection {connection_id:?}")]
    TimedOut { connection_id: ConnectionId },
    #[error("failed to open reliable control stream for connection {connection_id:?}: {source}")]
    OpenStream {
        connection_id: ConnectionId,
        #[source]
        source: quinn::ConnectionError,
    },
    #[error(transparent)]
    Frame(#[from] FrameIoError),
    #[error("reliable test/channel egress is closed for connection {connection_id:?}")]
    ChannelClosed { connection_id: ConnectionId },
    #[error("reliable control capability is fail-closed for connection {connection_id:?}")]
    FailClosed { connection_id: ConnectionId },
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TIMEOUT: Duration = Duration::from_secs(2);

    #[test]
    fn invalid_channel_configuration_is_rejected() {
        assert!(matches!(
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 0, 128, TEST_TIMEOUT),
            Err(ReliableEgressConfigError::ZeroCapacity)
        ));
        assert!(matches!(
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 1, 0, TEST_TIMEOUT),
            Err(ReliableEgressConfigError::ZeroMaximumPayloadBytes)
        ));
        assert!(matches!(
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 1, 128, Duration::ZERO),
            Err(ReliableEgressConfigError::ZeroSendTimeout)
        ));
    }

    #[test]
    fn negotiated_minor_is_explicit_and_clone_stable() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, _receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 1, 128, TEST_TIMEOUT)?;
        assert_eq!(sender.negotiated_protocol_minor(), 0);
        assert!(!sender.supports_protocol_minor(1));

        let sender = sender.with_negotiated_protocol_minor(1);
        let clone = sender.clone();
        assert_eq!(sender.negotiated_protocol_minor(), 1);
        assert!(sender.supports_protocol_minor(1));
        assert_eq!(clone.negotiated_protocol_minor(), 1);
        assert!(clone.supports_protocol_minor(1));
        Ok(())
    }

    #[tokio::test]
    async fn capability_gate_suppresses_unsupported_minor_without_fail_closing()
    -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 1, 128, TEST_TIMEOUT)?;

        let outcome = sender
            .send_if_supported(1, MessageKind::EnvironmentPresentationPolicy, &[2, 0, 6])
            .await?;
        assert_eq!(
            outcome,
            ReliableCapabilitySendOutcome::UnsupportedProtocolMinor {
                negotiated: 0,
                required: 1,
            }
        );
        assert!(receiver.try_recv().is_err());

        sender.send(MessageKind::FrontierManifest, &[1]).await?;
        assert_eq!(
            receiver.recv().await,
            Some(ReliableControlFrame {
                kind: MessageKind::FrontierManifest,
                payload: vec![1],
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn capability_gate_delivers_supported_minor() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 1, 128, TEST_TIMEOUT)?;
        let sender = sender.with_negotiated_protocol_minor(1);

        let outcome = sender
            .send_if_supported(1, MessageKind::EnvironmentPresentationPolicy, &[2, 0, 6])
            .await?;
        assert_eq!(outcome, ReliableCapabilitySendOutcome::Sent);
        assert_eq!(
            receiver.recv().await,
            Some(ReliableControlFrame {
                kind: MessageKind::EnvironmentPresentationPolicy,
                payload: vec![2, 0, 6],
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn channel_preserves_order_and_connection_scope() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 2, 128, TEST_TIMEOUT)?;
        assert_eq!(sender.connection_id(), ConnectionId(7));

        sender.send(MessageKind::FrontierManifest, &[1]).await?;
        sender.send(MessageKind::FrontierManifest, &[2]).await?;

        assert_eq!(
            receiver.recv().await,
            Some(ReliableControlFrame {
                kind: MessageKind::FrontierManifest,
                payload: vec![1],
            })
        );
        assert_eq!(
            receiver.recv().await,
            Some(ReliableControlFrame {
                kind: MessageKind::FrontierManifest,
                payload: vec![2],
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn payload_limit_fails_before_enqueue() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 1, 2, TEST_TIMEOUT)?;

        assert!(matches!(
            sender.send(MessageKind::FrontierManifest, &[1, 2, 3]).await,
            Err(ReliableSendError::PayloadTooLarge {
                actual: 3,
                maximum: 2,
            })
        ));
        assert!(receiver.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn channel_backpressures_instead_of_dropping() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, mut receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(7), 1, 128, TEST_TIMEOUT)?;
        sender.send(MessageKind::FrontierManifest, &[1]).await?;

        let second_sender = sender.clone();
        let second =
            tokio::spawn(async move { second_sender.send(MessageKind::FrontierManifest, &[2]).await });
        tokio::task::yield_now().await;
        assert!(!second.is_finished());

        assert_eq!(receiver.recv().await.map(|frame| frame.payload), Some(vec![1]));
        second.await??;
        assert_eq!(receiver.recv().await.map(|frame| frame.payload), Some(vec![2]));
        Ok(())
    }

    #[tokio::test]
    async fn closed_channel_is_an_explicit_send_error() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(9), 1, 128, TEST_TIMEOUT)?;
        drop(receiver);

        assert!(matches!(
            sender.send(MessageKind::FrontierManifest, &[1]).await,
            Err(ReliableSendError::ChannelClosed {
                connection_id: ConnectionId(9),
            })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn first_send_failure_fail_closes_all_sender_clones() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(9), 1, 128, TEST_TIMEOUT)?;
        let clone = sender.clone();
        drop(receiver);

        assert!(matches!(
            sender.send(MessageKind::FrontierManifest, &[1]).await,
            Err(ReliableSendError::ChannelClosed {
                connection_id: ConnectionId(9),
            })
        ));
        assert!(matches!(
            clone.send(MessageKind::FrontierManifest, &[2]).await,
            Err(ReliableSendError::FailClosed {
                connection_id: ConnectionId(9),
            })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn explicit_fail_close_invalidates_sender_clones() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, _receiver) =
            ReliableConnectionSender::bounded_channel(ConnectionId(9), 1, 128, TEST_TIMEOUT)?;
        let clone = sender.clone();
        sender.fail_close();

        assert!(matches!(
            clone.send(MessageKind::FrontierManifest, &[1]).await,
            Err(ReliableSendError::FailClosed {
                connection_id: ConnectionId(9),
            })
        ));
        Ok(())
    }
}
