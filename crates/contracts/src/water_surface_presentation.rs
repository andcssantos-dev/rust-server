use crate::FrontierQuadrantCoord;
use crate::terrain_authority::{TERRAIN_CONTROL_GRID_MAX_SIDE, TERRAIN_CONTROL_GRID_MIN_SIDE};
use thiserror::Error;

pub const WATER_SURFACE_PRESENTATION_VERSION: u16 = 1;
pub const WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES: usize = 30;
pub const WATER_SURFACE_PRESENTATION_SAMPLE_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WaterSurfaceKindV1 {
    Dry = 0,
    Lake = 1,
    River = 2,
}

impl TryFrom<u8> for WaterSurfaceKindV1 {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Dry),
            1 => Ok(Self::Lake),
            2 => Ok(Self::River),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterSurfaceSampleV1 {
    pub kind: WaterSurfaceKindV1,
    pub flow_hint_x: i8,
    pub flow_hint_y: i8,
    pub water_surface_mm: i32,
}

impl WaterSurfaceSampleV1 {
    #[must_use]
    pub const fn dry() -> Self {
        Self {
            kind: WaterSurfaceKindV1::Dry,
            flow_hint_x: 0,
            flow_hint_y: 0,
            water_surface_mm: 0,
        }
    }

    #[must_use]
    pub const fn lake(water_surface_mm: i32) -> Self {
        Self {
            kind: WaterSurfaceKindV1::Lake,
            flow_hint_x: 0,
            flow_hint_y: 0,
            water_surface_mm,
        }
    }

    #[must_use]
    pub const fn river(water_surface_mm: i32, flow_hint_x: i8, flow_hint_y: i8) -> Self {
        Self {
            kind: WaterSurfaceKindV1::River,
            flow_hint_x,
            flow_hint_y,
            water_surface_mm,
        }
    }

    #[must_use]
    pub const fn is_wet(self) -> bool {
        !matches!(self.kind, WaterSurfaceKindV1::Dry)
    }
}

/// Public server-authored water-surface presentation truth for one Revealed Quadrant.
///
/// Samples align one-to-one with TerrainAuthority control-grid vertices. UE may use
/// these values to construct water visuals, but may not invent wetness, surface height,
/// water kind or flow direction. Gameplay traversal and water interaction remain
/// separate authoritative concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterSurfacePresentationV1 {
    pub version: u16,
    pub quadrant_coord: FrontierQuadrantCoord,
    pub terrain_generator_version: u32,
    pub hydrology_generator_version: u32,
    pub control_grid_side: u16,
    pub samples: Vec<WaterSurfaceSampleV1>,
}

impl WaterSurfacePresentationV1 {
    #[must_use]
    pub fn expected_sample_count(&self) -> usize {
        let side = usize::from(self.control_grid_side);
        side * side
    }

    pub fn validate(&self) -> Result<(), WaterSurfacePresentationError> {
        if self.version != WATER_SURFACE_PRESENTATION_VERSION {
            return Err(WaterSurfacePresentationError::UnsupportedVersion(self.version));
        }
        if self.terrain_generator_version == 0 {
            return Err(WaterSurfacePresentationError::InvalidTerrainGeneratorVersion);
        }
        if self.hydrology_generator_version == 0 {
            return Err(WaterSurfacePresentationError::InvalidHydrologyGeneratorVersion);
        }
        if !(TERRAIN_CONTROL_GRID_MIN_SIDE..=TERRAIN_CONTROL_GRID_MAX_SIDE).contains(&self.control_grid_side)
        {
            return Err(WaterSurfacePresentationError::InvalidControlGridSide(
                self.control_grid_side,
            ));
        }

        let expected_samples = self.expected_sample_count();
        if self.samples.len() != expected_samples {
            return Err(WaterSurfacePresentationError::SampleCount {
                expected: expected_samples,
                actual: self.samples.len(),
            });
        }

        for (index, sample) in self.samples.iter().copied().enumerate() {
            match sample.kind {
                WaterSurfaceKindV1::Dry => {
                    if sample.water_surface_mm != 0 || sample.flow_hint_x != 0 || sample.flow_hint_y != 0 {
                        return Err(WaterSurfacePresentationError::NonCanonicalDrySample { index });
                    }
                }
                WaterSurfaceKindV1::Lake => {
                    if sample.flow_hint_x != 0 || sample.flow_hint_y != 0 {
                        return Err(WaterSurfacePresentationError::LakeHasFlow { index });
                    }
                }
                WaterSurfaceKindV1::River => {}
            }
        }

        Ok(())
    }

    pub fn encode_wire(&self) -> Result<Vec<u8>, WaterSurfacePresentationWireCodecError> {
        self.validate()?;
        let sample_count = u16::try_from(self.samples.len()).map_err(|_| {
            WaterSurfacePresentationWireCodecError::CountOverflow {
                field: "samples",
                actual: self.samples.len(),
            }
        })?;
        let payload_len = wire_payload_len(sample_count)?;
        let mut bytes = Vec::with_capacity(payload_len);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.quadrant_coord.x.to_le_bytes());
        bytes.extend_from_slice(&self.quadrant_coord.y.to_le_bytes());
        bytes.extend_from_slice(&self.terrain_generator_version.to_le_bytes());
        bytes.extend_from_slice(&self.hydrology_generator_version.to_le_bytes());
        bytes.extend_from_slice(&self.control_grid_side.to_le_bytes());
        bytes.extend_from_slice(&sample_count.to_le_bytes());

        for sample in &self.samples {
            bytes.push(sample.kind as u8);
            bytes.push(sample.flow_hint_x as u8);
            bytes.push(sample.flow_hint_y as u8);
            bytes.push(0);
            bytes.extend_from_slice(&sample.water_surface_mm.to_le_bytes());
        }

        Ok(bytes)
    }

    pub fn decode_wire(payload: &[u8]) -> Result<Self, WaterSurfacePresentationWireCodecError> {
        if payload.len() < WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES {
            return Err(WaterSurfacePresentationWireCodecError::PayloadTooShort {
                actual: payload.len(),
                minimum: WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES,
            });
        }

        let sample_count = decode_u16(payload, 28)?;
        let expected_len = wire_payload_len(sample_count)?;
        if payload.len() != expected_len {
            return Err(WaterSurfacePresentationWireCodecError::InvalidLength {
                expected: expected_len,
                actual: payload.len(),
            });
        }

        let mut samples = Vec::with_capacity(usize::from(sample_count));
        let mut offset = WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES;
        for index in 0..usize::from(sample_count) {
            let kind_raw = decode_u8(payload, offset)?;
            let kind = WaterSurfaceKindV1::try_from(kind_raw).map_err(|value| {
                WaterSurfacePresentationWireCodecError::UnknownSampleKind { index, value }
            })?;
            let flow_hint_x = decode_u8(payload, offset + 1)? as i8;
            let flow_hint_y = decode_u8(payload, offset + 2)? as i8;
            let reserved = decode_u8(payload, offset + 3)?;
            if reserved != 0 {
                return Err(WaterSurfacePresentationWireCodecError::NonZeroReservedByte {
                    index,
                    value: reserved,
                });
            }
            let water_surface_mm = decode_i32(payload, offset + 4)?;
            samples.push(WaterSurfaceSampleV1 {
                kind,
                flow_hint_x,
                flow_hint_y,
                water_surface_mm,
            });
            offset += WATER_SURFACE_PRESENTATION_SAMPLE_BYTES;
        }

        let presentation = Self {
            version: decode_u16(payload, 0)?,
            quadrant_coord: FrontierQuadrantCoord::new(decode_i64(payload, 2)?, decode_i64(payload, 10)?),
            terrain_generator_version: decode_u32(payload, 18)?,
            hydrology_generator_version: decode_u32(payload, 22)?,
            control_grid_side: decode_u16(payload, 26)?,
            samples,
        };
        presentation.validate()?;
        Ok(presentation)
    }
}

fn wire_payload_len(sample_count: u16) -> Result<usize, WaterSurfacePresentationWireCodecError> {
    usize::from(sample_count)
        .checked_mul(WATER_SURFACE_PRESENTATION_SAMPLE_BYTES)
        .and_then(|sample_bytes| WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES.checked_add(sample_bytes))
        .ok_or(WaterSurfacePresentationWireCodecError::LengthOverflow)
}

fn decode_u8(payload: &[u8], start: usize) -> Result<u8, WaterSurfacePresentationWireCodecError> {
    payload
        .get(start)
        .copied()
        .ok_or(WaterSurfacePresentationWireCodecError::InvalidLength {
            expected: start + 1,
            actual: payload.len(),
        })
}

fn decode_u16(payload: &[u8], start: usize) -> Result<u16, WaterSurfacePresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 2)
            .ok_or(WaterSurfacePresentationWireCodecError::InvalidLength {
                expected: start + 2,
                actual: payload.len(),
            })?;
    Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| {
        WaterSurfacePresentationWireCodecError::InvalidLength {
            expected: start + 2,
            actual: payload.len(),
        }
    })?))
}

fn decode_u32(payload: &[u8], start: usize) -> Result<u32, WaterSurfacePresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 4)
            .ok_or(WaterSurfacePresentationWireCodecError::InvalidLength {
                expected: start + 4,
                actual: payload.len(),
            })?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
        WaterSurfacePresentationWireCodecError::InvalidLength {
            expected: start + 4,
            actual: payload.len(),
        }
    })?))
}

fn decode_i32(payload: &[u8], start: usize) -> Result<i32, WaterSurfacePresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 4)
            .ok_or(WaterSurfacePresentationWireCodecError::InvalidLength {
                expected: start + 4,
                actual: payload.len(),
            })?;
    Ok(i32::from_le_bytes(bytes.try_into().map_err(|_| {
        WaterSurfacePresentationWireCodecError::InvalidLength {
            expected: start + 4,
            actual: payload.len(),
        }
    })?))
}

fn decode_i64(payload: &[u8], start: usize) -> Result<i64, WaterSurfacePresentationWireCodecError> {
    let bytes =
        payload
            .get(start..start + 8)
            .ok_or(WaterSurfacePresentationWireCodecError::InvalidLength {
                expected: start + 8,
                actual: payload.len(),
            })?;
    Ok(i64::from_le_bytes(bytes.try_into().map_err(|_| {
        WaterSurfacePresentationWireCodecError::InvalidLength {
            expected: start + 8,
            actual: payload.len(),
        }
    })?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaterSurfacePresentationError {
    UnsupportedVersion(u16),
    InvalidTerrainGeneratorVersion,
    InvalidHydrologyGeneratorVersion,
    InvalidControlGridSide(u16),
    SampleCount { expected: usize, actual: usize },
    NonCanonicalDrySample { index: usize },
    LakeHasFlow { index: usize },
}

impl std::fmt::Display for WaterSurfacePresentationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for WaterSurfacePresentationError {}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WaterSurfacePresentationWireCodecError {
    #[error(transparent)]
    InvalidPresentation(#[from] WaterSurfacePresentationError),
    #[error("water surface presentation payload length {actual} is below minimum {minimum}")]
    PayloadTooShort { actual: usize, minimum: usize },
    #[error("water surface presentation payload length {actual} does not equal expected {expected}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("water surface presentation payload length overflowed usize")]
    LengthOverflow,
    #[error("water surface presentation field {field} contains {actual} entries and cannot fit in u16")]
    CountOverflow { field: &'static str, actual: usize },
    #[error("water surface presentation sample {index} has unknown kind {value}")]
    UnknownSampleKind { index: usize, value: u8 },
    #[error("water surface presentation sample {index} has non-zero reserved byte {value}")]
    NonZeroReservedByte { index: usize, value: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_presentation() -> WaterSurfacePresentationV1 {
        WaterSurfacePresentationV1 {
            version: WATER_SURFACE_PRESENTATION_VERSION,
            quadrant_coord: FrontierQuadrantCoord::new(-11, -15),
            terrain_generator_version: 1,
            hydrology_generator_version: 1,
            control_grid_side: 3,
            samples: vec![
                WaterSurfaceSampleV1::dry(),
                WaterSurfaceSampleV1::lake(-1_047),
                WaterSurfaceSampleV1::dry(),
                WaterSurfaceSampleV1::river(1_865, 0, 1),
                WaterSurfaceSampleV1::lake(-1_047),
                WaterSurfaceSampleV1::dry(),
                WaterSurfaceSampleV1::dry(),
                WaterSurfaceSampleV1::river(1_840, 0, 1),
                WaterSurfaceSampleV1::dry(),
            ],
        }
    }

    #[test]
    fn valid_presentation_preserves_server_owned_surface_truth() {
        assert_eq!(valid_presentation().validate(), Ok(()));
    }

    #[test]
    fn dry_sample_must_be_canonical() {
        let mut presentation = valid_presentation();
        presentation.samples[0].water_surface_mm = 1;
        assert!(matches!(
            presentation.validate(),
            Err(WaterSurfacePresentationError::NonCanonicalDrySample { index: 0 })
        ));
    }

    #[test]
    fn lake_sample_cannot_claim_flow() {
        let mut presentation = valid_presentation();
        presentation.samples[1].flow_hint_y = 1;
        assert!(matches!(
            presentation.validate(),
            Err(WaterSurfacePresentationError::LakeHasFlow { index: 1 })
        ));
    }

    #[test]
    fn rejects_shape_mismatch() {
        let mut presentation = valid_presentation();
        presentation.samples.pop();
        assert!(matches!(
            presentation.validate(),
            Err(WaterSurfacePresentationError::SampleCount { .. })
        ));
    }

    #[test]
    fn wire_roundtrips_exact_surface_samples() -> Result<(), WaterSurfacePresentationWireCodecError> {
        let presentation = valid_presentation();
        let encoded = presentation.encode_wire()?;
        let decoded = WaterSurfacePresentationV1::decode_wire(&encoded)?;
        assert_eq!(decoded, presentation);
        assert_eq!(
            encoded.len(),
            WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES
                + presentation.samples.len() * WATER_SURFACE_PRESENTATION_SAMPLE_BYTES
        );
        Ok(())
    }

    #[test]
    fn wire_rejects_unknown_kind_reserved_trailing_and_truncated_payloads()
    -> Result<(), WaterSurfacePresentationWireCodecError> {
        let encoded = valid_presentation().encode_wire()?;

        let mut unknown_kind = encoded.clone();
        unknown_kind[WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES] = 9;
        assert!(matches!(
            WaterSurfacePresentationV1::decode_wire(&unknown_kind),
            Err(WaterSurfacePresentationWireCodecError::UnknownSampleKind { .. })
        ));

        let mut reserved = encoded.clone();
        reserved[WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES + 3] = 1;
        assert!(matches!(
            WaterSurfacePresentationV1::decode_wire(&reserved),
            Err(WaterSurfacePresentationWireCodecError::NonZeroReservedByte { .. })
        ));

        assert!(WaterSurfacePresentationV1::decode_wire(&encoded[..encoded.len() - 1]).is_err());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(WaterSurfacePresentationV1::decode_wire(&trailing).is_err());
        Ok(())
    }
}
