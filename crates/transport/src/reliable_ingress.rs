use std::net::SocketAddr;

use aurenfall_contracts::{
    FrontierManifestAck, FrontierManifestAckCodecError, IntentCodecError, MessageKind, MineIntent,
};
use aurenfall_core::ConnectionId;
use quinn::{Connection, VarInt};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{TransportSessionEvent, read_single_frame};

const RELIABLE_CONTROL_PROTOCOL_ERROR_CODE: u32 = 0x1004;
const SESSION_INGRESS_UNAVAILABLE_CODE: u32 = 0x1003;

#[allow(dead_code)]
#[derive(Debug, Error, Clone, PartialEq, Eq)]
enum ReliableIngressDecodeError {
    #[error("unsupported client reliable control kind {0:?}")]
    UnsupportedKind(MessageKind),
    #[error(transparent)]
    FrontierManifestAck(#[from] FrontierManifestAckCodecError),
    #[error(transparent)]
    MineIntent(#[from] IntentCodecError),
}

pub(crate) async fn run_live_reliable_control_ingress(
    connection: Connection,
    maximum_payload_bytes: usize,
    remote: SocketAddr,
    connection_id: ConnectionId,
    events: mpsc::Sender<TransportSessionEvent>,
) {
    loop {
        let mut recv = match connection.accept_uni().await {
            Ok(recv) => recv,
            Err(reason) => {
                debug!(
                    %remote,
                    connection_id = connection_id.0,
                    ?reason,
                    "Aurenfall reliable control ingress stopped"
                );
                break;
            }
        };

        let (kind, payload) = match read_single_frame(&mut recv, maximum_payload_bytes).await {
            Ok(frame) => frame,
            Err(error) => {
                warn!(
                    %remote,
                    connection_id = connection_id.0,
                    %error,
                    "invalid client reliable control frame; closing connection"
                );
                connection.close(
                    VarInt::from_u32(RELIABLE_CONTROL_PROTOCOL_ERROR_CODE),
                    b"invalid reliable control frame",
                );
                break;
            }
        };

        match recv.read_to_end(1).await {
            Ok(trailing) if trailing.is_empty() => {}
            Ok(_) | Err(_) => {
                warn!(
                    %remote,
                    connection_id = connection_id.0,
                    "client reliable control stream contained trailing bytes; closing connection"
                );
                connection.close(
                    VarInt::from_u32(RELIABLE_CONTROL_PROTOCOL_ERROR_CODE),
                    b"reliable control stream must contain exactly one frame",
                );
                break;
            }
        }

        match kind {
            MessageKind::FrontierManifestAck => {
                let ack = match FrontierManifestAck::decode(&payload) {
                    Ok(ack) => ack,
                    Err(error) => {
                        warn!(
                            %remote,
                            connection_id = connection_id.0,
                            %error,
                            "invalid client FrontierManifestAck message; closing connection"
                        );
                        connection.close(
                            VarInt::from_u32(RELIABLE_CONTROL_PROTOCOL_ERROR_CODE),
                            b"invalid reliable FrontierManifestAck message",
                        );
                        break;
                    }
                };

                if events
                    .send(TransportSessionEvent::FrontierManifestAck { connection_id, ack })
                    .await
                    .is_err()
                {
                    warn!(
                        %remote,
                        connection_id = connection_id.0,
                        frontier_revision = ack.revision,
                        "session ingress unavailable for reliable FrontierManifestAck; closing connection"
                    );
                    connection.close(
                        VarInt::from_u32(SESSION_INGRESS_UNAVAILABLE_CODE),
                        b"session ingress unavailable",
                    );
                    break;
                }

                info!(
                    %remote,
                    connection_id = connection_id.0,
                    frontier_revision = ack.revision,
                    "received reliable FrontierManifestAck from client"
                );
            }

            MessageKind::MineIntent => {
                let intent = match MineIntent::decode(&payload) {
                    Ok(intent) => intent,
                    Err(error) => {
                        warn!(
                            %remote,
                            connection_id = connection_id.0,
                            %error,
                            "invalid client MineIntent message; closing connection"
                        );
                        connection.close(
                            VarInt::from_u32(RELIABLE_CONTROL_PROTOCOL_ERROR_CODE),
                            b"invalid reliable MineIntent message",
                        );
                        break;
                    }
                };

                info!(
                    %remote,
                    connection_id = connection_id.0,
                    resource_id = intent.resource_id,
                    sequence = intent.sequence,
                    "RECEBIDO MineIntent confiavel no controle QUIC!"
                );

                if events
                    .send(TransportSessionEvent::MineIntent { connection_id, intent })
                    .await
                    .is_err()
                {
                    warn!(
                        %remote,
                        connection_id = connection_id.0,
                        "session ingress unavailable for reliable MineIntent; closing connection"
                    );
                    connection.close(
                        VarInt::from_u32(SESSION_INGRESS_UNAVAILABLE_CODE),
                        b"session ingress unavailable",
                    );
                    break;
                }
            }

            unsupported => {
                warn!(
                    %remote,
                    connection_id = connection_id.0,
                    error = %format!("unsupported client reliable control kind {:?}", unsupported),
                    "invalid client reliable control message; closing connection"
                );
                connection.close(
                    VarInt::from_u32(RELIABLE_CONTROL_PROTOCOL_ERROR_CODE),
                    b"invalid client reliable control message",
                );
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reliable_ingress_decodes_frontier_manifest_ack() -> anyhow::Result<()> {
        let ack = FrontierManifestAck::new(42)?;
        assert_eq!(
            FrontierManifestAck::decode(&ack.encode())?,
            ack
        );
        Ok(())
    }

    #[test]
    fn reliable_ingress_decodes_mine_intent() -> anyhow::Result<()> {
        let intent = MineIntent::new(2_936_143_873, 1);
        assert_eq!(
            MineIntent::decode(&intent.encode())?,
            intent
        );
        Ok(())
    }

    #[test]
    fn reliable_ingress_rejects_invalid_ack_payload() {
        assert!(matches!(
            FrontierManifestAck::decode(&0_u64.to_le_bytes()),
            Err(FrontierManifestAckCodecError::ZeroRevision)
        ));
    }
}