use aurenfall_core::{AccountId, CharacterId, ConnectionId, IntentSequence, SessionId, ZoneId};
use thiserror::Error;

use crate::{AuthorizedIntent, IntentPayload};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    ProtocolAdmitted,
    Authenticated,
    CharacterBound,
    WorldActive,
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldBinding {
    account_id: AccountId,
    character_id: CharacterId,
    zone_id: ZoneId,
}

impl WorldBinding {
    #[must_use]
    pub const fn account_id(self) -> AccountId {
        self.account_id
    }

    #[must_use]
    pub const fn character_id(self) -> CharacterId {
        self.character_id
    }

    #[must_use]
    pub const fn zone_id(self) -> ZoneId {
        self.zone_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    ProtocolAdmitted,
    Authenticated {
        account_id: AccountId,
    },
    CharacterBound {
        account_id: AccountId,
        character_id: CharacterId,
        zone_id: ZoneId,
    },
    WorldActive {
        account_id: AccountId,
        character_id: CharacterId,
        zone_id: ZoneId,
    },
    Closing,
}

impl SessionState {
    const fn phase(self) -> SessionPhase {
        match self {
            Self::ProtocolAdmitted => SessionPhase::ProtocolAdmitted,
            Self::Authenticated { .. } => SessionPhase::Authenticated,
            Self::CharacterBound { .. } => SessionPhase::CharacterBound,
            Self::WorldActive { .. } => SessionPhase::WorldActive,
            Self::Closing => SessionPhase::Closing,
        }
    }

    const fn world_binding(self) -> Option<WorldBinding> {
        match self {
            Self::CharacterBound {
                account_id,
                character_id,
                zone_id,
            }
            | Self::WorldActive {
                account_id,
                character_id,
                zone_id,
            } => Some(WorldBinding {
                account_id,
                character_id,
                zone_id,
            }),
            Self::ProtocolAdmitted | Self::Authenticated { .. } | Self::Closing => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AuthoritativeSession {
    session_id: SessionId,
    connection_id: ConnectionId,
    state: SessionState,
}

impl AuthoritativeSession {
    pub fn new(session_id: SessionId, connection_id: ConnectionId) -> Result<Self, SessionStateError> {
        require_nonzero("session_id", session_id.0)?;
        require_nonzero("connection_id", connection_id.0)?;
        Ok(Self {
            session_id,
            connection_id,
            state: SessionState::ProtocolAdmitted,
        })
    }

    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.state.phase()
    }

    #[must_use]
    pub const fn world_binding(&self) -> Option<WorldBinding> {
        self.state.world_binding()
    }

    pub fn authenticate(&mut self, account_id: AccountId) -> Result<(), SessionStateError> {
        require_nonzero("account_id", account_id.0)?;
        if self.state != SessionState::ProtocolAdmitted {
            return Err(SessionStateError::InvalidTransition {
                from: self.phase(),
                action: "authenticate",
            });
        }
        self.state = SessionState::Authenticated { account_id };
        Ok(())
    }

    pub fn bind_character(
        &mut self,
        character_id: CharacterId,
        zone_id: ZoneId,
    ) -> Result<(), SessionStateError> {
        require_nonzero("character_id", character_id.0)?;
        require_nonzero("zone_id", zone_id.0)?;
        let SessionState::Authenticated { account_id } = self.state else {
            return Err(SessionStateError::InvalidTransition {
                from: self.phase(),
                action: "bind_character",
            });
        };
        self.state = SessionState::CharacterBound {
            account_id,
            character_id,
            zone_id,
        };
        Ok(())
    }

    pub fn activate_world(&mut self) -> Result<(), SessionStateError> {
        let SessionState::CharacterBound {
            account_id,
            character_id,
            zone_id,
        } = self.state
        else {
            return Err(SessionStateError::InvalidTransition {
                from: self.phase(),
                action: "activate_world",
            });
        };
        self.state = SessionState::WorldActive {
            account_id,
            character_id,
            zone_id,
        };
        Ok(())
    }

    pub fn begin_closing(&mut self) {
        self.state = SessionState::Closing;
    }

    pub fn authorize_intent(
        &self,
        sequence: IntentSequence,
        payload: IntentPayload,
    ) -> Result<AuthorizedIntent, SessionAuthorityError> {
        let SessionState::WorldActive {
            account_id,
            character_id,
            zone_id,
        } = self.state
        else {
            return Err(SessionAuthorityError::NotWorldActive { phase: self.phase() });
        };

        Ok(AuthorizedIntent::new(
            self.session_id,
            account_id,
            character_id,
            zone_id,
            sequence,
            payload,
        ))
    }
}

fn require_nonzero(name: &'static str, value: u64) -> Result<(), SessionStateError> {
    if value == 0 {
        return Err(SessionStateError::UnassignedIdentity { name });
    }
    Ok(())
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SessionStateError {
    #[error("{name} must be server-assigned and non-zero")]
    UnassignedIdentity { name: &'static str },
    #[error("cannot {action} while session is in phase {from:?}")]
    InvalidTransition {
        from: SessionPhase,
        action: &'static str,
    },
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SessionAuthorityError {
    #[error("session cannot authorize gameplay intent while in phase {phase:?}")]
    NotWorldActive { phase: SessionPhase },
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn admitted_session() -> Result<AuthoritativeSession, SessionStateError> {
        AuthoritativeSession::new(SessionId(11), ConnectionId(7))
    }

    #[test]
    fn session_requires_ordered_authority_transitions() -> Result<(), SessionStateError> {
        let mut session = admitted_session()?;
        assert_eq!(session.phase(), SessionPhase::ProtocolAdmitted);

        assert_eq!(
            session.bind_character(CharacterId(99), ZoneId(3)),
            Err(SessionStateError::InvalidTransition {
                from: SessionPhase::ProtocolAdmitted,
                action: "bind_character",
            })
        );

        session.authenticate(AccountId(41))?;
        assert_eq!(session.phase(), SessionPhase::Authenticated);
        session.bind_character(CharacterId(99), ZoneId(3))?;
        assert_eq!(session.phase(), SessionPhase::CharacterBound);
        assert_eq!(
            session.world_binding(),
            Some(WorldBinding {
                account_id: AccountId(41),
                character_id: CharacterId(99),
                zone_id: ZoneId(3),
            })
        );
        session.activate_world()?;
        assert_eq!(session.phase(), SessionPhase::WorldActive);
        assert_eq!(
            session.world_binding().map(WorldBinding::character_id),
            Some(CharacterId(99))
        );
        Ok(())
    }

    #[test]
    fn authorized_intent_derives_identity_from_server_session() -> TestResult {
        let mut session = admitted_session()?;
        session.authenticate(AccountId(41))?;
        session.bind_character(CharacterId(99), ZoneId(3))?;
        session.activate_world()?;

        let intent = session.authorize_intent(
            IntentSequence(55),
            IntentPayload::Move {
                axis_x: 1_000,
                axis_y: -2_000,
            },
        )?;

        assert_eq!(intent.session_id(), SessionId(11));
        assert_eq!(intent.account_id(), AccountId(41));
        assert_eq!(intent.character_id(), CharacterId(99));
        assert_eq!(intent.zone_id(), ZoneId(3));
        assert_eq!(intent.sequence(), IntentSequence(55));
        Ok(())
    }

    #[test]
    fn closing_session_rejects_gameplay_intent_and_hides_world_binding() -> TestResult {
        let mut session = admitted_session()?;
        session.authenticate(AccountId(41))?;
        session.bind_character(CharacterId(99), ZoneId(3))?;
        session.activate_world()?;
        session.begin_closing();

        assert_eq!(session.world_binding(), None);
        assert_eq!(
            session.authorize_intent(IntentSequence(56), IntentPayload::Move { axis_x: 0, axis_y: 0 },),
            Err(SessionAuthorityError::NotWorldActive {
                phase: SessionPhase::Closing,
            })
        );
        Ok(())
    }

    #[test]
    fn runtime_session_identity_must_be_nonzero() {
        assert_eq!(
            AuthoritativeSession::new(SessionId(0), ConnectionId(7)),
            Err(SessionStateError::UnassignedIdentity { name: "session_id" })
        );
        assert_eq!(
            AuthoritativeSession::new(SessionId(11), ConnectionId(0)),
            Err(SessionStateError::UnassignedIdentity {
                name: "connection_id"
            })
        );
    }
}
