use thiserror::Error;

pub const SELF_MOVEMENT_SNAPSHOT_V2_BYTES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MovementFlagsV2(pub u8);

impl MovementFlagsV2 {
    pub const NONE: Self = Self(0);
    pub const SPRINTING: Self = Self(1 << 0);
    pub const ACCELERATING: Self = Self(1 << 1);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfMovementSnapshotV2 {
    pub server_tick: u64,
    pub x_mm: i64,
    pub y_mm: i64,
    pub z_mm: i64,
    pub last_processed_input_sequence: Option<u64>,
    pub active_movement_sequence: Option<u64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotV2CodecError {
    #[error("payload too short, expected at least {expected} bytes, got {actual}")]
    PayloadTooShort { expected: usize, actual: usize },
    #[error("invalid sequence marker: {0}")]
    InvalidMarker(u8),
}

impl SelfMovementSnapshotV2 {
    #[must_use]
    pub fn encode_wire(&self) -> [u8; SELF_MOVEMENT_SNAPSHOT_V2_BYTES] {
        let mut bytes = [0u8; SELF_MOVEMENT_SNAPSHOT_V2_BYTES];

        // 1. Posições e Tick do Servidor (0..32)
        bytes[0..8].copy_from_slice(&self.server_tick.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.x_mm.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.y_mm.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.z_mm.to_le_bytes());

        // 2. Sequência de Entrada Processada (32..41)
        encode_optional_sequence(&mut bytes[32..41], self.last_processed_input_sequence);

        // 3. Sequência de Movimento Ativa (41..50)
        encode_optional_sequence(&mut bytes[41..50], self.active_movement_sequence);

        bytes
    }

    pub fn decode_wire(payload: &[u8]) -> Result<Self, SnapshotV2CodecError> {
        if payload.len() != SELF_MOVEMENT_SNAPSHOT_V2_BYTES {
            return Err(SnapshotV2CodecError::PayloadTooShort {
                expected: SELF_MOVEMENT_SNAPSHOT_V2_BYTES,
                actual: payload.len(),
            });
        }

        let server_tick = u64::from_le_bytes(payload[0..8].try_into().unwrap());
        let x_mm = i64::from_le_bytes(payload[8..16].try_into().unwrap());
        let y_mm = i64::from_le_bytes(payload[16..24].try_into().unwrap());
        let z_mm = i64::from_le_bytes(payload[24..32].try_into().unwrap());

        let last_processed_input_sequence = decode_optional_sequence(&payload[32..41])?;
        let active_movement_sequence = decode_optional_sequence(&payload[41..50])?;

        Ok(Self {
            server_tick,
            x_mm,
            y_mm,
            z_mm,
            last_processed_input_sequence,
            active_movement_sequence,
        })
    }
}

fn encode_optional_sequence(target: &mut [u8], seq: Option<u64>) {
    if let Some(val) = seq {
        target[0] = 1;
        target[1..9].copy_from_slice(&val.to_le_bytes());
    } else {
        target[0] = 0;
        target[1..9].fill(0);
    }
}

fn decode_optional_sequence(source: &[u8]) -> Result<Option<u64>, SnapshotV2CodecError> {
    if source.is_empty() {
        return Ok(None);
    }
    match source[0] {
        0 => Ok(None),
        1 => {
            if source.len() < 9 {
                return Err(SnapshotV2CodecError::PayloadTooShort {
                    expected: 9,
                    actual: source.len(),
                });
            }
            Ok(Some(u64::from_le_bytes(source[1..9].try_into().unwrap())))
        }
        marker => Err(SnapshotV2CodecError::InvalidMarker(marker)),
    }
}