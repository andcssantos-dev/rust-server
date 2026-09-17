use aurenfall_core::{AccountId, CharacterId, IntentSequence, SessionId, ZoneId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentPayload {
    Move { axis_x: i16, axis_y: i16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizedIntent {
    session_id: SessionId,
    account_id: AccountId,
    character_id: CharacterId,
    zone_id: ZoneId,
    sequence: IntentSequence,
    payload: IntentPayload,
}

impl AuthorizedIntent {
    pub(crate) fn new(
        session_id: SessionId,
        account_id: AccountId,
        character_id: CharacterId,
        zone_id: ZoneId,
        sequence: IntentSequence,
        payload: IntentPayload,
    ) -> Self {
        Self {
            session_id,
            account_id,
            character_id,
            zone_id,
            sequence,
            payload,
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn account_id(&self) -> AccountId {
        self.account_id
    }

    #[must_use]
    pub const fn character_id(&self) -> CharacterId {
        self.character_id
    }

    #[must_use]
    pub const fn zone_id(&self) -> ZoneId {
        self.zone_id
    }

    #[must_use]
    pub const fn sequence(&self) -> IntentSequence {
        self.sequence
    }

    #[must_use]
    pub const fn payload(&self) -> IntentPayload {
        self.payload
    }
}
