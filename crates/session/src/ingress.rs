use aurenfall_core::{AccountId, CharacterId, ConnectionId, IntentSequence, SessionId, ZoneId};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{
    AuthorizedIntent, IntentPayload, IntentSequenceError, LiveSessionRegistry, SessionPhase,
    SessionRegistryError, WorldBinding,
};

#[derive(Debug)]
pub struct SessionIntentIngress {
    registry: LiveSessionRegistry,
    authorized_tx: mpsc::Sender<AuthorizedIntent>,
}

impl SessionIntentIngress {
    pub fn bounded(
        capacity: usize,
    ) -> Result<(Self, mpsc::Receiver<AuthorizedIntent>), IntentIngressConfigError> {
        if capacity == 0 {
            return Err(IntentIngressConfigError::ZeroCapacity);
        }

        let (authorized_tx, authorized_rx) = mpsc::channel(capacity);
        Ok((
            Self {
                registry: LiveSessionRegistry::default(),
                authorized_tx,
            },
            authorized_rx,
        ))
    }

    #[must_use]
    pub fn live_session_count(&self) -> usize {
        self.registry.len()
    }

    #[must_use]
    pub fn is_world_active_session(&self, session_id: SessionId) -> bool {
        self.registry.is_world_active_session(session_id)
    }

    pub fn register(
        &mut self,
        session_id: SessionId,
        connection_id: ConnectionId,
    ) -> Result<(), SessionRegistryError> {
        self.registry.register(session_id, connection_id)
    }

    pub fn remove_by_connection(
        &mut self,
        connection_id: ConnectionId,
    ) -> Result<SessionId, SessionRegistryError> {
        self.registry.remove_by_connection(connection_id)
    }

    pub fn authenticate(
        &mut self,
        connection_id: ConnectionId,
        account_id: AccountId,
    ) -> Result<(), SessionRegistryError> {
        self.registry.authenticate(connection_id, account_id)
    }

    pub fn bind_character(
        &mut self,
        connection_id: ConnectionId,
        character_id: CharacterId,
        zone_id: ZoneId,
    ) -> Result<(), SessionRegistryError> {
        self.registry.bind_character(connection_id, character_id, zone_id)
    }

    pub fn activate_world(&mut self, connection_id: ConnectionId) -> Result<(), SessionRegistryError> {
        self.registry.activate_world(connection_id)
    }

    pub fn begin_closing(&mut self, connection_id: ConnectionId) -> Result<(), SessionRegistryError> {
        self.registry.begin_closing(connection_id)
    }

    pub fn phase_for_connection(
        &self,
        connection_id: ConnectionId,
    ) -> Result<SessionPhase, SessionRegistryError> {
        self.registry.phase_for_connection(connection_id)
    }

    pub fn world_binding_for_connection(
        &self,
        connection_id: ConnectionId,
    ) -> Result<Option<WorldBinding>, SessionRegistryError> {
        self.registry.world_binding_for_connection(connection_id)
    }

    pub fn try_ingest_move(
        &mut self,
        connection_id: ConnectionId,
        sequence: IntentSequence,
        axis_x: i16,
        axis_y: i16,
    ) -> Result<(), IntentIngressError> {
        let authorized = self.registry.authorize_from_connection(
            connection_id,
            sequence,
            IntentPayload::Move { axis_x, axis_y },
        )?;

        self.authorized_tx
            .try_send(authorized)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => IntentIngressError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => IntentIngressError::QueueClosed,
            })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum IntentIngressConfigError {
    #[error("authorized intent queue capacity must be greater than zero")]
    ZeroCapacity,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum IntentIngressError {
    #[error(transparent)]
    Sequence(#[from] IntentSequenceError),
    #[error("authorized intent queue is full; replaceable movement intent was dropped")]
    QueueFull,
    #[error("authorized intent queue is closed")]
    QueueClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    type ActiveIngressResult =
        Result<(SessionIntentIngress, mpsc::Receiver<AuthorizedIntent>), Box<dyn std::error::Error>>;

    fn active_ingress(capacity: usize) -> ActiveIngressResult {
        let (mut ingress, receiver) = SessionIntentIngress::bounded(capacity)?;
        ingress.register(SessionId(11), ConnectionId(7))?;
        ingress.authenticate(ConnectionId(7), AccountId(41))?;
        ingress.bind_character(ConnectionId(7), CharacterId(99), ZoneId(3))?;
        ingress.activate_world(ConnectionId(7))?;
        Ok((ingress, receiver))
    }

    #[test]
    fn zero_capacity_is_rejected_without_panicking() {
        assert!(matches!(
            SessionIntentIngress::bounded(0),
            Err(IntentIngressConfigError::ZeroCapacity)
        ));
    }

    #[test]
    fn move_ingress_emits_only_server_authorized_identity() -> TestResult {
        let (mut ingress, mut receiver) = active_ingress(2)?;
        ingress.try_ingest_move(ConnectionId(7), IntentSequence(10), 1_000, -2_000)?;

        let intent = receiver.try_recv()?;
        assert_eq!(intent.session_id(), SessionId(11));
        assert_eq!(intent.account_id(), AccountId(41));
        assert_eq!(intent.character_id(), CharacterId(99));
        assert_eq!(intent.zone_id(), ZoneId(3));
        assert_eq!(intent.sequence(), IntentSequence(10));
        assert_eq!(
            intent.payload(),
            IntentPayload::Move {
                axis_x: 1_000,
                axis_y: -2_000,
            }
        );
        Ok(())
    }

    #[test]
    fn live_world_state_can_guard_queued_authorized_intents() -> TestResult {
        let (mut ingress, _receiver) = active_ingress(2)?;
        assert!(ingress.is_world_active_session(SessionId(11)));
        assert_eq!(
            ingress
                .world_binding_for_connection(ConnectionId(7))?
                .map(WorldBinding::character_id),
            Some(CharacterId(99))
        );

        ingress.begin_closing(ConnectionId(7))?;
        assert!(!ingress.is_world_active_session(SessionId(11)));
        assert_eq!(ingress.world_binding_for_connection(ConnectionId(7))?, None);
        Ok(())
    }

    #[test]
    fn full_queue_drops_replaceable_move_and_consumes_sequence() -> TestResult {
        let (mut ingress, mut receiver) = active_ingress(1)?;
        ingress.try_ingest_move(ConnectionId(7), IntentSequence(10), 1, 0)?;

        assert_eq!(
            ingress.try_ingest_move(ConnectionId(7), IntentSequence(11), 1, 0),
            Err(IntentIngressError::QueueFull)
        );

        let _first = receiver.try_recv()?;
        assert!(matches!(
            ingress.try_ingest_move(ConnectionId(7), IntentSequence(11), 1, 0),
            Err(IntentIngressError::Sequence(
                IntentSequenceError::Duplicate { .. }
            ))
        ));

        ingress.try_ingest_move(ConnectionId(7), IntentSequence(12), 1, 0)?;
        let next = receiver.try_recv()?;
        assert_eq!(next.sequence(), IntentSequence(12));
        Ok(())
    }

    #[test]
    fn duplicate_sequence_never_reaches_authorized_queue() -> TestResult {
        let (mut ingress, mut receiver) = active_ingress(2)?;
        ingress.try_ingest_move(ConnectionId(7), IntentSequence(10), 1, 0)?;
        let _first = receiver.try_recv()?;

        assert!(matches!(
            ingress.try_ingest_move(ConnectionId(7), IntentSequence(10), 1, 0),
            Err(IntentIngressError::Sequence(
                IntentSequenceError::Duplicate { .. }
            ))
        ));
        assert!(receiver.try_recv().is_err());
        Ok(())
    }

    #[test]
    fn closed_queue_rejects_after_authority_and_sequence_admission() -> TestResult {
        let (mut ingress, receiver) = active_ingress(1)?;
        drop(receiver);

        assert_eq!(
            ingress.try_ingest_move(ConnectionId(7), IntentSequence(10), 1, 0),
            Err(IntentIngressError::QueueClosed)
        );
        Ok(())
    }
}
