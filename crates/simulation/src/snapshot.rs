use aurenfall_core::{CharacterId, IntentSequence, ServerTick, WorldPositionMm};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementSnapshot {
    character_id: CharacterId,
    server_tick: ServerTick,
    position: WorldPositionMm,
    last_processed_input_sequence: Option<IntentSequence>,
    active_movement_sequence: Option<IntentSequence>,
}

impl MovementSnapshot {
    #[must_use]
    pub const fn new(
        character_id: CharacterId,
        server_tick: ServerTick,
        position: WorldPositionMm,
        last_processed_input_sequence: Option<IntentSequence>,
        active_movement_sequence: Option<IntentSequence>,
    ) -> Self {
        Self {
            character_id,
            server_tick,
            position,
            last_processed_input_sequence,
            active_movement_sequence,
        }
    }

    #[must_use]
    pub const fn character_id(self) -> CharacterId {
        self.character_id
    }

    #[must_use]
    pub const fn server_tick(self) -> ServerTick {
        self.server_tick
    }

    #[must_use]
    pub const fn position(self) -> WorldPositionMm {
        self.position
    }

    #[must_use]
    pub const fn last_processed_input_sequence(self) -> Option<IntentSequence> {
        self.last_processed_input_sequence
    }

    #[must_use]
    pub const fn active_movement_sequence(self) -> Option<IntentSequence> {
        self.active_movement_sequence
    }
}
