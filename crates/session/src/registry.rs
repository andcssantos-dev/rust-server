use std::collections::HashMap;

use aurenfall_core::{
    AccountId, CharacterId, ConnectionId, IntentSequence, ReplayDecision, ReplayWindow, SessionId, ZoneId,
};
use thiserror::Error;

use crate::{
    AuthoritativeSession, AuthorizedIntent, IntentPayload, SessionAuthorityError, SessionPhase,
    SessionStateError, WorldBinding,
};

#[derive(Debug)]
struct LiveSession {
    session: AuthoritativeSession,
    replay: ReplayWindow,
}

#[derive(Debug, Default)]
pub struct LiveSessionRegistry {
    by_session: HashMap<SessionId, LiveSession>,
    by_connection: HashMap<ConnectionId, SessionId>,
}

impl LiveSessionRegistry {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_session.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_session.is_empty()
    }

    #[must_use]
    pub fn is_world_active_session(&self, session_id: SessionId) -> bool {
        self.by_session
            .get(&session_id)
            .is_some_and(|live| live.session.phase() == SessionPhase::WorldActive)
    }

    pub fn register(
        &mut self,
        session_id: SessionId,
        connection_id: ConnectionId,
    ) -> Result<(), SessionRegistryError> {
        if self.by_session.contains_key(&session_id) {
            return Err(SessionRegistryError::DuplicateSession { session_id });
        }
        if self.by_connection.contains_key(&connection_id) {
            return Err(SessionRegistryError::DuplicateConnection { connection_id });
        }

        let session = AuthoritativeSession::new(session_id, connection_id)?;
        self.by_session.insert(
            session_id,
            LiveSession {
                session,
                replay: ReplayWindow::default(),
            },
        );
        self.by_connection.insert(connection_id, session_id);
        Ok(())
    }

    pub fn remove_by_connection(
        &mut self,
        connection_id: ConnectionId,
    ) -> Result<SessionId, SessionRegistryError> {
        let session_id = self.session_id_for_connection(connection_id)?;
        if !self.by_session.contains_key(&session_id) {
            return Err(SessionRegistryError::InconsistentIndex {
                connection_id,
                session_id,
            });
        }

        self.by_connection.remove(&connection_id);
        self.by_session.remove(&session_id);
        Ok(session_id)
    }

    pub fn authenticate(
        &mut self,
        connection_id: ConnectionId,
        account_id: AccountId,
    ) -> Result<(), SessionRegistryError> {
        self.live_for_connection_mut(connection_id)?
            .session
            .authenticate(account_id)?;
        Ok(())
    }

    pub fn bind_character(
        &mut self,
        connection_id: ConnectionId,
        character_id: CharacterId,
        zone_id: ZoneId,
    ) -> Result<(), SessionRegistryError> {
        self.live_for_connection_mut(connection_id)?
            .session
            .bind_character(character_id, zone_id)?;
        Ok(())
    }

    pub fn activate_world(&mut self, connection_id: ConnectionId) -> Result<(), SessionRegistryError> {
        self.live_for_connection_mut(connection_id)?
            .session
            .activate_world()?;
        Ok(())
    }

    pub fn begin_closing(&mut self, connection_id: ConnectionId) -> Result<(), SessionRegistryError> {
        self.live_for_connection_mut(connection_id)?
            .session
            .begin_closing();
        Ok(())
    }

    pub fn phase_for_connection(
        &self,
        connection_id: ConnectionId,
    ) -> Result<SessionPhase, SessionRegistryError> {
        Ok(self.live_for_connection(connection_id)?.session.phase())
    }

    pub fn world_binding_for_connection(
        &self,
        connection_id: ConnectionId,
    ) -> Result<Option<WorldBinding>, SessionRegistryError> {
        Ok(self.live_for_connection(connection_id)?.session.world_binding())
    }

    pub fn session_id_for_connection(
        &self,
        connection_id: ConnectionId,
    ) -> Result<SessionId, SessionRegistryError> {
        self.by_connection
            .get(&connection_id)
            .copied()
            .ok_or(SessionRegistryError::UnknownConnection { connection_id })
    }

    pub fn authorize_from_connection(
        &mut self,
        connection_id: ConnectionId,
        sequence: IntentSequence,
        payload: IntentPayload,
    ) -> Result<AuthorizedIntent, IntentSequenceError> {
        let live = self
            .live_for_connection_mut(connection_id)
            .map_err(IntentSequenceError::Registry)?;

        let authorized = live
            .session
            .authorize_intent(sequence, payload)
            .map_err(IntentSequenceError::Authority)?;

        let highest_before = live.replay.highest();
        match live.replay.observe(sequence.0) {
            ReplayDecision::AcceptedNewHighest => Ok(authorized),
            ReplayDecision::AcceptedOutOfOrder => Err(IntentSequenceError::OutOfOrder {
                sequence,
                highest: highest_before,
            }),
            ReplayDecision::RejectedDuplicate => Err(IntentSequenceError::Duplicate { sequence }),
            ReplayDecision::RejectedTooOld => Err(IntentSequenceError::TooOld {
                sequence,
                highest: highest_before,
            }),
        }
    }

    fn live_for_connection(&self, connection_id: ConnectionId) -> Result<&LiveSession, SessionRegistryError> {
        let session_id = self.session_id_for_connection(connection_id)?;
        self.by_session
            .get(&session_id)
            .ok_or(SessionRegistryError::InconsistentIndex {
                connection_id,
                session_id,
            })
    }

    fn live_for_connection_mut(
        &mut self,
        connection_id: ConnectionId,
    ) -> Result<&mut LiveSession, SessionRegistryError> {
        let session_id = self.session_id_for_connection(connection_id)?;
        self.by_session
            .get_mut(&session_id)
            .ok_or(SessionRegistryError::InconsistentIndex {
                connection_id,
                session_id,
            })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SessionRegistryError {
    #[error("session {session_id:?} is already registered")]
    DuplicateSession { session_id: SessionId },
    #[error("connection {connection_id:?} is already bound to a live session")]
    DuplicateConnection { connection_id: ConnectionId },
    #[error("connection {connection_id:?} has no live server session")]
    UnknownConnection { connection_id: ConnectionId },
    #[error(
        "session registry index is inconsistent for connection {connection_id:?} and session {session_id:?}"
    )]
    InconsistentIndex {
        connection_id: ConnectionId,
        session_id: SessionId,
    },
    #[error(transparent)]
    State(#[from] SessionStateError),
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum IntentSequenceError {
    #[error(transparent)]
    Registry(SessionRegistryError),
    #[error(transparent)]
    Authority(SessionAuthorityError),
    #[error("intent sequence {sequence:?} is a duplicate")]
    Duplicate { sequence: IntentSequence },
    #[error("intent sequence {sequence:?} arrived behind the current highest {highest:?}")]
    OutOfOrder {
        sequence: IntentSequence,
        highest: Option<u64>,
    },
    #[error("intent sequence {sequence:?} is older than the replay window behind {highest:?}")]
    TooOld {
        sequence: IntentSequence,
        highest: Option<u64>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn active_registry() -> Result<LiveSessionRegistry, SessionRegistryError> {
        let mut registry = LiveSessionRegistry::default();
        registry.register(SessionId(11), ConnectionId(7))?;
        registry.authenticate(ConnectionId(7), AccountId(41))?;
        registry.bind_character(ConnectionId(7), CharacterId(99), ZoneId(3))?;
        registry.activate_world(ConnectionId(7))?;
        Ok(registry)
    }

    #[test]
    fn registry_rejects_duplicate_runtime_bindings() -> Result<(), SessionRegistryError> {
        let mut registry = LiveSessionRegistry::default();
        registry.register(SessionId(11), ConnectionId(7))?;

        assert_eq!(
            registry.register(SessionId(11), ConnectionId(8)),
            Err(SessionRegistryError::DuplicateSession {
                session_id: SessionId(11),
            })
        );
        assert_eq!(
            registry.register(SessionId(12), ConnectionId(7)),
            Err(SessionRegistryError::DuplicateConnection {
                connection_id: ConnectionId(7),
            })
        );
        Ok(())
    }

    #[test]
    fn registry_derives_authorized_identity_from_connection() -> TestResult {
        let mut registry = active_registry()?;
        let intent = registry.authorize_from_connection(
            ConnectionId(7),
            IntentSequence(10),
            IntentPayload::Move {
                axis_x: 100,
                axis_y: -200,
            },
        )?;

        assert_eq!(intent.session_id(), SessionId(11));
        assert_eq!(intent.account_id(), AccountId(41));
        assert_eq!(intent.character_id(), CharacterId(99));
        assert_eq!(intent.zone_id(), ZoneId(3));

        let binding = registry.world_binding_for_connection(ConnectionId(7))?;
        assert_eq!(binding.map(WorldBinding::account_id), Some(AccountId(41)));
        assert_eq!(binding.map(WorldBinding::character_id), Some(CharacterId(99)));
        assert_eq!(binding.map(WorldBinding::zone_id), Some(ZoneId(3)));
        Ok(())
    }

    #[test]
    fn authority_failure_does_not_consume_sequence() -> TestResult {
        let mut registry = LiveSessionRegistry::default();
        registry.register(SessionId(11), ConnectionId(7))?;

        let before_activation = registry.authorize_from_connection(
            ConnectionId(7),
            IntentSequence(10),
            IntentPayload::Move { axis_x: 1, axis_y: 0 },
        );
        assert!(matches!(
            before_activation,
            Err(IntentSequenceError::Authority(
                SessionAuthorityError::NotWorldActive { .. }
            ))
        ));

        registry.authenticate(ConnectionId(7), AccountId(41))?;
        registry.bind_character(ConnectionId(7), CharacterId(99), ZoneId(3))?;
        registry.activate_world(ConnectionId(7))?;

        assert!(
            registry
                .authorize_from_connection(
                    ConnectionId(7),
                    IntentSequence(10),
                    IntentPayload::Move { axis_x: 1, axis_y: 0 },
                )
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn sequence_guard_rejects_duplicate_and_out_of_order_intents() -> TestResult {
        let mut registry = active_registry()?;
        let payload = IntentPayload::Move {
            axis_x: 100,
            axis_y: 0,
        };

        assert!(
            registry
                .authorize_from_connection(ConnectionId(7), IntentSequence(10), payload)
                .is_ok()
        );
        assert!(
            registry
                .authorize_from_connection(ConnectionId(7), IntentSequence(12), payload)
                .is_ok()
        );
        assert_eq!(
            registry.authorize_from_connection(ConnectionId(7), IntentSequence(12), payload),
            Err(IntentSequenceError::Duplicate {
                sequence: IntentSequence(12),
            })
        );
        assert_eq!(
            registry.authorize_from_connection(ConnectionId(7), IntentSequence(11), payload),
            Err(IntentSequenceError::OutOfOrder {
                sequence: IntentSequence(11),
                highest: Some(12),
            })
        );
        Ok(())
    }

    #[test]
    fn sequence_windows_are_isolated_per_live_session() -> TestResult {
        let mut registry = active_registry()?;
        registry.register(SessionId(12), ConnectionId(8))?;
        registry.authenticate(ConnectionId(8), AccountId(42))?;
        registry.bind_character(ConnectionId(8), CharacterId(100), ZoneId(4))?;
        registry.activate_world(ConnectionId(8))?;

        let payload = IntentPayload::Move { axis_x: 0, axis_y: 1 };
        assert!(
            registry
                .authorize_from_connection(ConnectionId(7), IntentSequence(50), payload)
                .is_ok()
        );
        assert!(
            registry
                .authorize_from_connection(ConnectionId(8), IntentSequence(1), payload)
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn world_active_membership_tracks_closing_and_removal() -> TestResult {
        let mut registry = active_registry()?;
        assert!(registry.is_world_active_session(SessionId(11)));

        registry.begin_closing(ConnectionId(7))?;
        assert!(!registry.is_world_active_session(SessionId(11)));

        assert_eq!(registry.remove_by_connection(ConnectionId(7))?, SessionId(11));
        assert!(!registry.is_world_active_session(SessionId(11)));
        Ok(())
    }

    #[test]
    fn removing_connection_removes_live_session_binding() -> Result<(), SessionRegistryError> {
        let mut registry = LiveSessionRegistry::default();
        registry.register(SessionId(11), ConnectionId(7))?;
        assert_eq!(registry.remove_by_connection(ConnectionId(7))?, SessionId(11));
        assert!(registry.is_empty());
        assert_eq!(
            registry.session_id_for_connection(ConnectionId(7)),
            Err(SessionRegistryError::UnknownConnection {
                connection_id: ConnectionId(7),
            })
        );
        Ok(())
    }
}
