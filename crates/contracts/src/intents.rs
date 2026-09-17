use thiserror::Error;

pub const MOVE_INTENT_BYTES: usize = 12;
pub const MOVE_AXIS_MIN: i16 = -32_767;
pub const MOVE_AXIS_MAX: i16 = 32_767;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveIntent {
    pub sequence: u64,
    pub axis_x: i16,
    pub axis_y: i16,
}

impl MoveIntent {
    pub fn new(sequence: u64, axis_x: i16, axis_y: i16) -> Result<Self, IntentCodecError> {
        validate_axis("axis_x", axis_x)?;
        validate_axis("axis_y", axis_y)?;
        Ok(Self {
            sequence,
            axis_x,
            axis_y,
        })
    }

    #[must_use]
    pub fn encode(self) -> [u8; MOVE_INTENT_BYTES] {
        let mut bytes = [0_u8; MOVE_INTENT_BYTES];
        bytes[0..8].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.axis_x.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.axis_y.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IntentCodecError> {
        if bytes.len() != MOVE_INTENT_BYTES {
            return Err(IntentCodecError::InvalidLength {
                expected: MOVE_INTENT_BYTES,
                actual: bytes.len(),
            });
        }

        let sequence = u64::from_le_bytes(
            bytes[0..8]
                .try_into()
                .map_err(|_| IntentCodecError::InvalidLength {
                    expected: MOVE_INTENT_BYTES,
                    actual: bytes.len(),
                })?,
        );
        let axis_x = i16::from_le_bytes(
            bytes[8..10]
                .try_into()
                .map_err(|_| IntentCodecError::InvalidLength {
                    expected: MOVE_INTENT_BYTES,
                    actual: bytes.len(),
                })?,
        );
        let axis_y = i16::from_le_bytes(
            bytes[10..12]
                .try_into()
                .map_err(|_| IntentCodecError::InvalidLength {
                    expected: MOVE_INTENT_BYTES,
                    actual: bytes.len(),
                })?,
        );
        Self::new(sequence, axis_x, axis_y)
    }
}

fn validate_axis(name: &'static str, value: i16) -> Result<(), IntentCodecError> {
    if !(MOVE_AXIS_MIN..=MOVE_AXIS_MAX).contains(&value) {
        return Err(IntentCodecError::AxisOutOfRange { name, value });
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IntentCodecError {
    #[error("invalid intent payload length: expected {expected} bytes, got {actual}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("{name} value {value} is outside the supported signed movement range")]
    AxisOutOfRange { name: &'static str, value: i16 },
}

// =========================================================================
// MINE INTENT V1 (Compatível com Unreal Engine 5: 16 bytes)
// 8 bytes: resource_id (Little Endian)
// 8 bytes: sequence / action_sequence (Little Endian)
// =========================================================================

pub const MINE_INTENT_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MineIntent {
    pub resource_id: u64,
    pub sequence: u64,
}

impl MineIntent {
    #[must_use]
    pub const fn new(resource_id: u64, sequence: u64) -> Self {
        Self {
            resource_id,
            sequence,
        }
    }

    #[must_use]
    pub fn encode(self) -> [u8; MINE_INTENT_BYTES] {
        let mut bytes = [0_u8; MINE_INTENT_BYTES];
        // 1. Primeiros 8 bytes: Identificador da Rocha
        bytes[0..8].copy_from_slice(&self.resource_id.to_le_bytes());
        // 2. Últimos 8 bytes: Sequência da Ação
        bytes[8..16].copy_from_slice(&self.sequence.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IntentCodecError> {
        if bytes.len() != MINE_INTENT_BYTES {
            return Err(IntentCodecError::InvalidLength {
                expected: MINE_INTENT_BYTES,
                actual: bytes.len(),
            });
        }

        let resource_id = u64::from_le_bytes(
            bytes[0..8]
                .try_into()
                .map_err(|_| IntentCodecError::InvalidLength {
                    expected: MINE_INTENT_BYTES,
                    actual: bytes.len(),
                })?,
        );

        let sequence = u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .map_err(|_| IntentCodecError::InvalidLength {
                    expected: MINE_INTENT_BYTES,
                    actual: bytes.len(),
                })?,
        );

        Ok(Self::new(resource_id, sequence))
    }
}

// =========================================================================
// TESTES AUTOMATIZADOS
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_intent_roundtrips_without_authoritative_identity_fields() -> Result<(), IntentCodecError> {
        let intent = MoveIntent::new(42, -12_345, 23_456)?;
        let encoded = intent.encode();

        assert_eq!(encoded.len(), MOVE_INTENT_BYTES);
        assert_eq!(MoveIntent::decode(&encoded), Ok(intent));
        Ok(())
    }

    #[test]
    fn move_intent_rejects_asymmetric_i16_minimum() {
        assert_eq!(
            MoveIntent::new(1, i16::MIN, 0),
            Err(IntentCodecError::AxisOutOfRange {
                name: "axis_x",
                value: i16::MIN,
            })
        );
    }

    #[test]
    fn move_intent_rejects_wrong_payload_size() {
        assert_eq!(
            MoveIntent::decode(&[0_u8; MOVE_INTENT_BYTES - 1]),
            Err(IntentCodecError::InvalidLength {
                expected: MOVE_INTENT_BYTES,
                actual: MOVE_INTENT_BYTES - 1,
            })
        );
    }

    #[test]
    fn mine_intent_roundtrips_with_16_bytes() -> Result<(), IntentCodecError> {
        // Rocha 0xAF020001 (2936143873) e ação número 1
        let intent = MineIntent::new(2_936_143_873, 1);
        let encoded = intent.encode();
        
        assert_eq!(encoded.len(), MINE_INTENT_BYTES, "O tamanho do pacote deve ter exatamente 16 bytes");
        assert_eq!(MineIntent::decode(&encoded), Ok(intent));
        Ok(())
    }

    #[test]
    fn mine_intent_rejects_wrong_payload_size() {
        assert_eq!(
            MineIntent::decode(&[0_u8; 12]),
            Err(IntentCodecError::InvalidLength {
                expected: MINE_INTENT_BYTES,
                actual: 12,
            })
        );
    }
}