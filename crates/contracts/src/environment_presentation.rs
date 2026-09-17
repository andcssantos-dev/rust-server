use crate::FrontierQuadrantCoord;
use crate::terrain_authority::{TERRAIN_CONTROL_GRID_MAX_SIDE, TERRAIN_CONTROL_GRID_MIN_SIDE};
use thiserror::Error;

pub const ENVIRONMENT_PRESENTATION_POLICY_VERSION: u16 = 1;
pub const ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES: usize = 38;

pub const ENVIRONMENT_FAMILY_GROUND_COVER: u16 = 1 << 0;
pub const ENVIRONMENT_FAMILY_SMALL_PLANT: u16 = 1 << 1;
pub const ENVIRONMENT_FAMILY_SHRUB: u16 = 1 << 2;
pub const ENVIRONMENT_FAMILY_TREE: u16 = 1 << 3;
pub const ENVIRONMENT_FAMILY_ROCK: u16 = 1 << 4;
pub const ENVIRONMENT_FAMILY_KNOWN_MASK: u16 = ENVIRONMENT_FAMILY_GROUND_COVER
    | ENVIRONMENT_FAMILY_SMALL_PLANT
    | ENVIRONMENT_FAMILY_SHRUB
    | ENVIRONMENT_FAMILY_TREE
    | ENVIRONMENT_FAMILY_ROCK;

/// Public Rust-authored constraints for cosmetic environmental presentation.
///
/// This contract says which visual families UE may choose from on each already-public
/// terrain semantic cell. It does not grant gameplay meaning to those visuals: collision,
/// harvesting, resources, AI habitat, persistence, traversal and discovery remain separate
/// authoritative concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentPresentationPolicyV1 {
    pub version: u16,
    pub quadrant_coord: FrontierQuadrantCoord,
    /// Terrain generator version this policy is aligned with.
    pub terrain_generator_version: u32,
    /// Independent version for the Rust environmental-policy generator.
    pub policy_generator_version: u32,
    /// Server-owned deterministic seed UE may use only for cosmetic selection inside
    /// the family mask supplied for a cell.
    pub policy_seed: u64,
    /// Must match the TerrainAuthority control-grid side for the same Quadrant.
    pub control_grid_side: u16,
    /// Row-major `(control_grid_side - 1)^2` masks of allowed presentation families.
    /// A zero mask is valid and explicitly means "no environmental clutter family".
    pub cell_family_masks: Vec<u16>,
}

impl EnvironmentPresentationPolicyV1 {
    #[must_use]
    pub fn expected_cell_count(&self) -> usize {
        let side = usize::from(self.control_grid_side.saturating_sub(1));
        side * side
    }

    pub fn validate(&self) -> Result<(), EnvironmentPresentationPolicyError> {
        if self.version != ENVIRONMENT_PRESENTATION_POLICY_VERSION {
            return Err(EnvironmentPresentationPolicyError::UnsupportedVersion(
                self.version,
            ));
        }
        if self.terrain_generator_version == 0 {
            return Err(EnvironmentPresentationPolicyError::InvalidTerrainGeneratorVersion);
        }
        if self.policy_generator_version == 0 {
            return Err(EnvironmentPresentationPolicyError::InvalidPolicyGeneratorVersion);
        }
        if !(TERRAIN_CONTROL_GRID_MIN_SIDE..=TERRAIN_CONTROL_GRID_MAX_SIDE).contains(&self.control_grid_side)
        {
            return Err(EnvironmentPresentationPolicyError::InvalidControlGridSide(
                self.control_grid_side,
            ));
        }

        let expected_cells = self.expected_cell_count();
        if self.cell_family_masks.len() != expected_cells {
            return Err(EnvironmentPresentationPolicyError::CellFamilyMaskCount {
                expected: expected_cells,
                actual: self.cell_family_masks.len(),
            });
        }

        for (index, mask) in self.cell_family_masks.iter().copied().enumerate() {
            if mask & !ENVIRONMENT_FAMILY_KNOWN_MASK != 0 {
                return Err(EnvironmentPresentationPolicyError::UnknownFamilyBits { index, mask });
            }
        }

        Ok(())
    }

    pub fn encode_wire(&self) -> Result<Vec<u8>, EnvironmentPresentationWireCodecError> {
        self.validate()?;
        let cell_count = u16::try_from(self.cell_family_masks.len()).map_err(|_| {
            EnvironmentPresentationWireCodecError::CountOverflow {
                field: "cell_family_masks",
                actual: self.cell_family_masks.len(),
            }
        })?;
        let payload_len = wire_payload_len(cell_count)?;
        let mut bytes = Vec::with_capacity(payload_len);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.quadrant_coord.x.to_le_bytes());
        bytes.extend_from_slice(&self.quadrant_coord.y.to_le_bytes());
        bytes.extend_from_slice(&self.terrain_generator_version.to_le_bytes());
        bytes.extend_from_slice(&self.policy_generator_version.to_le_bytes());
        bytes.extend_from_slice(&self.policy_seed.to_le_bytes());
        bytes.extend_from_slice(&self.control_grid_side.to_le_bytes());
        bytes.extend_from_slice(&cell_count.to_le_bytes());
        for mask in &self.cell_family_masks {
            bytes.extend_from_slice(&mask.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn decode_wire(payload: &[u8]) -> Result<Self, EnvironmentPresentationWireCodecError> {
        if payload.len() < ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES {
            return Err(EnvironmentPresentationWireCodecError::PayloadTooShort {
                actual: payload.len(),
                minimum: ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES,
            });
        }

        let cell_count = decode_u16(payload, 36)?;
        let expected_len = wire_payload_len(cell_count)?;
        if payload.len() != expected_len {
            return Err(EnvironmentPresentationWireCodecError::InvalidLength {
                expected: expected_len,
                actual: payload.len(),
            });
        }

        let mut cell_family_masks = Vec::with_capacity(usize::from(cell_count));
        let mut offset = ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES;
        for _ in 0..cell_count {
            cell_family_masks.push(decode_u16(payload, offset)?);
            offset += 2;
        }

        let policy = Self {
            version: decode_u16(payload, 0)?,
            quadrant_coord: FrontierQuadrantCoord::new(decode_i64(payload, 2)?, decode_i64(payload, 10)?),
            terrain_generator_version: decode_u32(payload, 18)?,
            policy_generator_version: decode_u32(payload, 22)?,
            policy_seed: decode_u64(payload, 26)?,
            control_grid_side: decode_u16(payload, 34)?,
            cell_family_masks,
        };
        policy.validate()?;
        Ok(policy)
    }
}

fn wire_payload_len(cell_count: u16) -> Result<usize, EnvironmentPresentationWireCodecError> {
    usize::from(cell_count)
        .checked_mul(2)
        .and_then(|mask_bytes| ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES.checked_add(mask_bytes))
        .ok_or(EnvironmentPresentationWireCodecError::LengthOverflow)
}

fn decode_u16(payload: &[u8], start: usize) -> Result<u16, EnvironmentPresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 2)
            .ok_or(EnvironmentPresentationWireCodecError::InvalidLength {
                expected: start + 2,
                actual: payload.len(),
            })?;
    Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| {
        EnvironmentPresentationWireCodecError::InvalidLength {
            expected: start + 2,
            actual: payload.len(),
        }
    })?))
}

fn decode_u32(payload: &[u8], start: usize) -> Result<u32, EnvironmentPresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 4)
            .ok_or(EnvironmentPresentationWireCodecError::InvalidLength {
                expected: start + 4,
                actual: payload.len(),
            })?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
        EnvironmentPresentationWireCodecError::InvalidLength {
            expected: start + 4,
            actual: payload.len(),
        }
    })?))
}

fn decode_u64(payload: &[u8], start: usize) -> Result<u64, EnvironmentPresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 8)
            .ok_or(EnvironmentPresentationWireCodecError::InvalidLength {
                expected: start + 8,
                actual: payload.len(),
            })?;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
        EnvironmentPresentationWireCodecError::InvalidLength {
            expected: start + 8,
            actual: payload.len(),
        }
    })?))
}

fn decode_i64(payload: &[u8], start: usize) -> Result<i64, EnvironmentPresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 8)
            .ok_or(EnvironmentPresentationWireCodecError::InvalidLength {
                expected: start + 8,
                actual: payload.len(),
            })?;
    Ok(i64::from_le_bytes(bytes.try_into().map_err(|_| {
        EnvironmentPresentationWireCodecError::InvalidLength {
            expected: start + 8,
            actual: payload.len(),
        }
    })?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentPresentationPolicyError {
    UnsupportedVersion(u16),
    InvalidTerrainGeneratorVersion,
    InvalidPolicyGeneratorVersion,
    InvalidControlGridSide(u16),
    CellFamilyMaskCount { expected: usize, actual: usize },
    UnknownFamilyBits { index: usize, mask: u16 },
}

impl std::fmt::Display for EnvironmentPresentationPolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for EnvironmentPresentationPolicyError {}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EnvironmentPresentationWireCodecError {
    #[error(transparent)]
    InvalidPolicy(#[from] EnvironmentPresentationPolicyError),
    #[error("environment presentation payload length {actual} is below minimum {minimum}")]
    PayloadTooShort { actual: usize, minimum: usize },
    #[error("environment presentation payload length {actual} does not equal expected {expected}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("environment presentation payload length overflowed usize")]
    LengthOverflow,
    #[error("environment presentation field {field} contains {actual} entries and cannot fit in u16")]
    CountOverflow { field: &'static str, actual: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_policy() -> EnvironmentPresentationPolicyV1 {
        EnvironmentPresentationPolicyV1 {
            version: ENVIRONMENT_PRESENTATION_POLICY_VERSION,
            quadrant_coord: FrontierQuadrantCoord::new(4, -7),
            terrain_generator_version: 1,
            policy_generator_version: 1,
            policy_seed: 0x000A_11CE_C05E,
            control_grid_side: 3,
            cell_family_masks: vec![
                ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_SMALL_PLANT,
                ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_SHRUB,
                0,
                ENVIRONMENT_FAMILY_TREE | ENVIRONMENT_FAMILY_ROCK,
            ],
        }
    }

    #[test]
    fn valid_policy_preserves_server_owned_allowed_families() {
        assert_eq!(valid_policy().validate(), Ok(()));
    }

    #[test]
    fn zero_mask_explicitly_allows_no_environmental_family() {
        let mut policy = valid_policy();
        policy.cell_family_masks.fill(0);
        assert_eq!(policy.validate(), Ok(()));
    }

    #[test]
    fn rejects_shape_mismatch() {
        let mut policy = valid_policy();
        policy.cell_family_masks.pop();
        assert!(matches!(
            policy.validate(),
            Err(EnvironmentPresentationPolicyError::CellFamilyMaskCount { .. })
        ));
    }

    #[test]
    fn rejects_unknown_family_bits() {
        let mut policy = valid_policy();
        policy.cell_family_masks[0] |= 1 << 15;
        assert!(matches!(
            policy.validate(),
            Err(EnvironmentPresentationPolicyError::UnknownFamilyBits { .. })
        ));
    }

    #[test]
    fn wire_policy_roundtrips_exact_allowed_family_masks() -> Result<(), EnvironmentPresentationWireCodecError>
    {
        let policy = valid_policy();
        let encoded = policy.encode_wire()?;
        let decoded = EnvironmentPresentationPolicyV1::decode_wire(&encoded)?;
        assert_eq!(decoded, policy);
        assert_eq!(encoded.len(), ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES + 4 * 2);
        Ok(())
    }

    #[test]
    fn wire_policy_rejects_truncated_and_trailing_payloads()
    -> Result<(), EnvironmentPresentationWireCodecError> {
        let encoded = valid_policy().encode_wire()?;
        assert!(EnvironmentPresentationPolicyV1::decode_wire(&encoded[..encoded.len() - 1]).is_err());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(EnvironmentPresentationPolicyV1::decode_wire(&trailing).is_err());
        Ok(())
    }

    #[test]
    fn wire_policy_revalidates_family_bits_after_decode() -> Result<(), EnvironmentPresentationWireCodecError>
    {
        let mut encoded = valid_policy().encode_wire()?;
        let first_mask_offset = ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES;
        encoded[first_mask_offset..first_mask_offset + 2].copy_from_slice(&(1_u16 << 15).to_le_bytes());
        assert!(matches!(
            EnvironmentPresentationPolicyV1::decode_wire(&encoded),
            Err(EnvironmentPresentationWireCodecError::InvalidPolicy(
                EnvironmentPresentationPolicyError::UnknownFamilyBits { .. }
            ))
        ));
        Ok(())
    }
}
