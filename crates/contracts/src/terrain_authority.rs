use crate::FrontierQuadrantCoord;
use thiserror::Error;

pub const TERRAIN_AUTHORITY_CONTRACT_VERSION: u16 = 1;
pub const TERRAIN_CONTROL_GRID_MIN_SIDE: u16 = 2;
pub const TERRAIN_CONTROL_GRID_MAX_SIDE: u16 = 65;
pub const TERRAIN_AUTHORITY_WIRE_HEADER_BYTES: usize = 45;
pub const TERRAIN_AUTHORITY_HEADER_BYTES: usize = 45;

pub const TERRAIN_CELL_WALKABLE: u8 = 1 << 0;
pub const TERRAIN_CELL_BUILDABLE: u8 = 1 << 1;
pub const TERRAIN_CELL_WATER: u8 = 1 << 2;
pub const TERRAIN_CELL_BLOCKED: u8 = 1 << 3;
pub const TERRAIN_CELL_CONNECTOR_CORRIDOR: u8 = 1 << 4;
pub const TERRAIN_CELL_KNOWN_MASK: u8 = TERRAIN_CELL_WALKABLE
    | TERRAIN_CELL_BUILDABLE
    | TERRAIN_CELL_WATER
    | TERRAIN_CELL_BLOCKED
    | TERRAIN_CELL_CONNECTOR_CORRIDOR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainAuthorityContractV1 {
    pub version: u16,
    pub quadrant_coord: FrontierQuadrantCoord,
    pub quadrant_size_mm: i64,
    pub generator_version: u32,
    pub recipe_seed: u64,
    pub control_grid_side: u16,
    pub biome_id: u8,
    pub elevation_samples_mm: Vec<i32>,
    pub cell_flags: Vec<u8>,
}

impl TerrainAuthorityContractV1 {
    #[must_use]
    pub fn expected_elevation_sample_count(&self) -> usize {
        usize::from(self.control_grid_side) * usize::from(self.control_grid_side)
    }

    #[must_use]
    pub fn expected_cell_count(&self) -> usize {
        let side = usize::from(self.control_grid_side.saturating_sub(1));
        side * side
    }

    pub fn validate(&self) -> Result<(), TerrainAuthorityContractError> {
        if self.version != TERRAIN_AUTHORITY_CONTRACT_VERSION {
            return Err(TerrainAuthorityContractError::UnsupportedVersion(self.version));
        }
        if self.quadrant_size_mm <= 0 {
            return Err(TerrainAuthorityContractError::InvalidQuadrantSize(
                self.quadrant_size_mm,
            ));
        }
        if self.generator_version == 0 {
            return Err(TerrainAuthorityContractError::InvalidGeneratorVersion);
        }
        if !(TERRAIN_CONTROL_GRID_MIN_SIDE..=TERRAIN_CONTROL_GRID_MAX_SIDE).contains(&self.control_grid_side)
        {
            return Err(TerrainAuthorityContractError::InvalidControlGridSide(
                self.control_grid_side,
            ));
        }

        let expected_samples = self.expected_elevation_sample_count();
        if self.elevation_samples_mm.len() != expected_samples {
            return Err(TerrainAuthorityContractError::ElevationSampleCount {
                expected: expected_samples,
                actual: self.elevation_samples_mm.len(),
            });
        }

        let expected_cells = self.expected_cell_count();
        if self.cell_flags.len() != expected_cells {
            return Err(TerrainAuthorityContractError::CellFlagCount {
                expected: expected_cells,
                actual: self.cell_flags.len(),
            });
        }

        for (index, flags) in self.cell_flags.iter().copied().enumerate() {
            if flags & !TERRAIN_CELL_KNOWN_MASK != 0 {
                return Err(TerrainAuthorityContractError::UnknownCellFlags { index, flags });
            }

            let walkable = flags & TERRAIN_CELL_WALKABLE != 0;
            let buildable = flags & TERRAIN_CELL_BUILDABLE != 0;
            let water = flags & TERRAIN_CELL_WATER != 0;
            let blocked = flags & TERRAIN_CELL_BLOCKED != 0;

            if walkable && blocked {
                return Err(TerrainAuthorityContractError::ContradictoryCellFlags { index, flags });
            }
            if buildable && (!walkable || blocked || water) {
                return Err(TerrainAuthorityContractError::InvalidBuildableCell { index, flags });
            }
        }

        Ok(())
    }

    pub fn encode_wire(&self) -> Result<Vec<u8>, TerrainAuthorityContractError> {
        self.validate()?;

        let sample_count = self.elevation_samples_mm.len();
        let cell_count = self.cell_flags.len();
        let total_bytes = TERRAIN_AUTHORITY_HEADER_BYTES + (sample_count * 4) + cell_count;

        let mut bytes = Vec::with_capacity(total_bytes);

        // Header de 45 bytes (0..45) idêntico ao esperado pelo C++ da Unreal Engine
        bytes.extend_from_slice(&self.version.to_le_bytes()); // 0..2
        bytes.extend_from_slice(&self.quadrant_coord.x.to_le_bytes()); // 2..10
        bytes.extend_from_slice(&self.quadrant_coord.y.to_le_bytes()); // 10..18
        bytes.extend_from_slice(&self.quadrant_size_mm.to_le_bytes()); // 18..26
        bytes.extend_from_slice(&self.generator_version.to_le_bytes()); // 26..30
        bytes.extend_from_slice(&self.recipe_seed.to_le_bytes()); // 30..38
        bytes.extend_from_slice(&self.control_grid_side.to_le_bytes()); // 38..40
        bytes.extend_from_slice(&(sample_count as u16).to_le_bytes()); // 40..42
        bytes.extend_from_slice(&(cell_count as u16).to_le_bytes()); // 42..44
        bytes.push(self.biome_id); // 44 (45º byte!)

        // Payloads de Elevação (4 bytes cada i32 Little-Endian)
        for &sample in &self.elevation_samples_mm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        // Flags de Célula (1 byte cada u8)
        bytes.extend_from_slice(&self.cell_flags);

        Ok(bytes)
    }

    pub fn decode_wire(payload: &[u8]) -> Result<Self, TerrainAuthorityWireCodecError> {
        if payload.len() < TERRAIN_AUTHORITY_HEADER_BYTES {
            return Err(TerrainAuthorityWireCodecError::PayloadTooShort {
                actual: payload.len(),
                minimum: TERRAIN_AUTHORITY_HEADER_BYTES,
            });
        }

        let version = u16::from_le_bytes(payload[0..2].try_into().unwrap());
        let q_x = i64::from_le_bytes(payload[2..10].try_into().unwrap());
        let q_y = i64::from_le_bytes(payload[10..18].try_into().unwrap());
        let quadrant_size_mm = i64::from_le_bytes(payload[18..26].try_into().unwrap());
        let generator_version = u32::from_le_bytes(payload[26..30].try_into().unwrap());
        let recipe_seed = u64::from_le_bytes(payload[30..38].try_into().unwrap());
        let control_grid_side = u16::from_le_bytes(payload[38..40].try_into().unwrap());
        let sample_count = u16::from_le_bytes(payload[40..42].try_into().unwrap()) as usize;
        let cell_count = u16::from_le_bytes(payload[42..44].try_into().unwrap()) as usize;
        let biome_id = payload[44];

        let expected_total = TERRAIN_AUTHORITY_HEADER_BYTES + (sample_count * 4) + cell_count;
        if payload.len() != expected_total {
            return Err(TerrainAuthorityWireCodecError::InvalidLength {
                expected: expected_total,
                actual: payload.len(),
            });
        }

        let mut offset = TERRAIN_AUTHORITY_HEADER_BYTES;
        let mut elevation_samples_mm = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            let sample = i32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
            elevation_samples_mm.push(sample);
            offset += 4;
        }

        let cell_flags = payload[offset..offset + cell_count].to_vec();

        let contract = Self {
            version,
            quadrant_coord: FrontierQuadrantCoord::new(q_x, q_y),
            generator_version,
            quadrant_size_mm,
            control_grid_side,
            recipe_seed,
            biome_id,
            elevation_samples_mm,
            cell_flags,
        };

        contract.validate()?;
        Ok(contract)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerrainAuthorityContractError {
    UnsupportedVersion(u16),
    InvalidQuadrantSize(i64),
    InvalidGeneratorVersion,
    InvalidControlGridSide(u16),
    ElevationSampleCount { expected: usize, actual: usize },
    CellFlagCount { expected: usize, actual: usize },
    UnknownCellFlags { index: usize, flags: u8 },
    ContradictoryCellFlags { index: usize, flags: u8 },
    InvalidBuildableCell { index: usize, flags: u8 },
}

impl std::fmt::Display for TerrainAuthorityContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TerrainAuthorityContractError {}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TerrainAuthorityWireCodecError {
    #[error(transparent)]
    InvalidContract(#[from] TerrainAuthorityContractError),
    #[error("terrain authority payload length {actual} is below minimum {minimum}")]
    PayloadTooShort { actual: usize, minimum: usize },
    #[error("terrain authority payload length {actual} does not equal expected {expected}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("terrain authority payload length overflowed usize")]
    LengthOverflow,
    #[error("terrain authority field {field} contains {actual} entries and cannot fit in u16")]
    CountOverflow { field: &'static str, actual: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_contract() -> TerrainAuthorityContractV1 {
        TerrainAuthorityContractV1 {
            version: TERRAIN_AUTHORITY_CONTRACT_VERSION,
            quadrant_coord: FrontierQuadrantCoord { x: 4, y: -7 },
            quadrant_size_mm: 64_000,
            generator_version: 1,
            recipe_seed: 0xA11CE,
            control_grid_side: 3,
            biome_id: 1,
            elevation_samples_mm: vec![0, 100, 200, -100, 0, 100, -200, -100, 0],
            cell_flags: vec![
                TERRAIN_CELL_WALKABLE | TERRAIN_CELL_BUILDABLE,
                TERRAIN_CELL_WALKABLE,
                TERRAIN_CELL_WATER | TERRAIN_CELL_BLOCKED,
                TERRAIN_CELL_WALKABLE | TERRAIN_CELL_CONNECTOR_CORRIDOR,
            ],
        }
    }

    #[test]
    fn valid_contract_preserves_server_owned_semantic_grid() {
        assert_eq!(valid_contract().validate(), Ok(()));
    }

    #[test]
    fn rejects_shape_mismatch() {
        let mut contract = valid_contract();
        contract.elevation_samples_mm.pop();
        assert!(matches!(
            contract.validate(),
            Err(TerrainAuthorityContractError::ElevationSampleCount { .. })
        ));
    }

    #[test]
    fn rejects_walkable_and_blocked_cell() {
        let mut contract = valid_contract();
        contract.cell_flags[0] = TERRAIN_CELL_WALKABLE | TERRAIN_CELL_BLOCKED;
        assert!(matches!(
            contract.validate(),
            Err(TerrainAuthorityContractError::ContradictoryCellFlags { .. })
        ));
    }

    #[test]
    fn rejects_buildable_water_cell() {
        let mut contract = valid_contract();
        contract.cell_flags[0] = TERRAIN_CELL_WALKABLE | TERRAIN_CELL_BUILDABLE | TERRAIN_CELL_WATER;
        assert!(matches!(
            contract.validate(),
            Err(TerrainAuthorityContractError::InvalidBuildableCell { .. })
        ));
    }

    #[test]
    fn wire_contract_roundtrips_exact_authoritative_content() -> Result<(), TerrainAuthorityWireCodecError> {
        let contract = valid_contract();
        let encoded = contract.encode_wire()?;
        let decoded = TerrainAuthorityContractV1::decode_wire(&encoded)?;
        assert_eq!(decoded, contract);
        assert_eq!(encoded.len(), TERRAIN_AUTHORITY_WIRE_HEADER_BYTES + (9 * 4) + 4);
        Ok(())
    }

    #[test]
    fn wire_contract_rejects_truncated_and_trailing_payloads() -> Result<(), TerrainAuthorityWireCodecError> {
        let encoded = valid_contract().encode_wire()?;
        assert!(TerrainAuthorityContractV1::decode_wire(&encoded[..encoded.len() - 1]).is_err());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(TerrainAuthorityContractV1::decode_wire(&trailing).is_err());
        Ok(())
    }

    #[test]
    fn wire_contract_revalidates_semantics_after_decode() -> Result<(), TerrainAuthorityWireCodecError> {
        let mut encoded = valid_contract().encode_wire()?;
        let first_cell_offset = TERRAIN_AUTHORITY_WIRE_HEADER_BYTES + (9 * 4);
        encoded[first_cell_offset] = TERRAIN_CELL_WALKABLE | TERRAIN_CELL_BLOCKED;
        assert!(matches!(
            TerrainAuthorityContractV1::decode_wire(&encoded),
            Err(TerrainAuthorityWireCodecError::InvalidContract(
                TerrainAuthorityContractError::ContradictoryCellFlags { .. }
            ))
        ));
        Ok(())
    }
}