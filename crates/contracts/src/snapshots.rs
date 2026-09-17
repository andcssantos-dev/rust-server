///use crate::bootstrap::FrameHeader;
use thiserror::Error;

pub const SELF_MOVEMENT_SNAPSHOT_BYTES: usize = 34;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfMovementSnapshot {
    pub server_tick: u64,
    pub x_mm: i64,
    pub y_mm: i64,
    pub z_mm: i64,
    pub sequence: Option<u64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotCodecError {
    #[error("payload too short, expected at least {expected} bytes, got {actual}")]
    PayloadTooShort { expected: usize, actual: usize },
    #[error("invalid sequence marker: {0}")]
    InvalidMarker(u8),
}

impl SelfMovementSnapshot {
    #[must_use]
    pub fn new(server_tick: u64, x_mm: i64, y_mm: i64, z_mm: i64, sequence: Option<u64>) -> Self {
        Self {
            server_tick,
            x_mm,
            y_mm,
            z_mm,
            sequence,
        }
    }

    #[must_use]
    pub fn encode(&self) -> [u8; SELF_MOVEMENT_SNAPSHOT_BYTES] {
        let mut bytes = [0u8; SELF_MOVEMENT_SNAPSHOT_BYTES];
        bytes[0..8].copy_from_slice(&self.server_tick.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.x_mm.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.y_mm.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.z_mm.to_le_bytes());
        match self.sequence {
            Some(seq) => {
                bytes[32] = 1;
                bytes[33] = (seq & 0xFF) as u8;
            }
            None => {
                bytes[32] = 0;
                bytes[33] = 0;
            }
        }
        bytes
    }

    pub fn decode(payload: &[u8]) -> Result<Self, SnapshotCodecError> {
        if payload.len() < SELF_MOVEMENT_SNAPSHOT_BYTES {
            return Err(SnapshotCodecError::PayloadTooShort {
                expected: SELF_MOVEMENT_SNAPSHOT_BYTES,
                actual: payload.len(),
            });
        }
        let server_tick = u64::from_le_bytes(payload[0..8].try_into().unwrap());
        let x_mm = i64::from_le_bytes(payload[8..16].try_into().unwrap());
        let y_mm = i64::from_le_bytes(payload[16..24].try_into().unwrap());
        let z_mm = i64::from_le_bytes(payload[24..32].try_into().unwrap());
        let sequence = match payload[32] {
            0 => None,
            1 => Some(u64::from(payload[33])),
            marker => return Err(SnapshotCodecError::InvalidMarker(marker)),
        };
        Ok(Self {
            server_tick,
            x_mm,
            y_mm,
            z_mm,
            sequence,
        })
    }
}