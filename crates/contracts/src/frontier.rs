use std::collections::BTreeSet;

use thiserror::Error;

pub const FRONTIER_MANIFEST_VERSION: u32 = 1;
pub const FRONTIER_MANIFEST_HEADER_BYTES: usize = 26;
pub const FRONTIER_MANIFEST_QUADRANT_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrontierQuadrantCoord {
    pub x: i64,
    pub y: i64,
}

impl FrontierQuadrantCoord {
    #[must_use]
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierManifest {
    pub manifest_version: u32,
    pub generator_version: u32,
    pub quadrant_size_mm: i64,
    pub revision: u64,
    pub quadrants: Vec<FrontierQuadrantCoord>,
}

impl FrontierManifest {
    pub fn new(
        generator_version: u32,
        quadrant_size_mm: i64,
        revision: u64,
        quadrants: Vec<FrontierQuadrantCoord>,
    ) -> Result<Self, FrontierManifestCodecError> {
        let manifest = Self {
            manifest_version: FRONTIER_MANIFEST_VERSION,
            generator_version,
            quadrant_size_mm,
            revision,
            quadrants,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn encode(&self) -> Result<Vec<u8>, FrontierManifestCodecError> {
        self.validate()?;
        let quadrant_count = u16::try_from(self.quadrants.len()).map_err(|_| {
            FrontierManifestCodecError::TooManyQuadrants {
                actual: self.quadrants.len(),
                maximum: usize::from(u16::MAX),
            }
        })?;
        let payload_len = payload_len(quadrant_count)?;
        let mut bytes = Vec::with_capacity(payload_len);
        bytes.extend_from_slice(&self.manifest_version.to_le_bytes());
        bytes.extend_from_slice(&self.generator_version.to_le_bytes());
        bytes.extend_from_slice(&self.quadrant_size_mm.to_le_bytes());
        bytes.extend_from_slice(&self.revision.to_le_bytes());
        bytes.extend_from_slice(&quadrant_count.to_le_bytes());
        for coord in &self.quadrants {
            bytes.extend_from_slice(&coord.x.to_le_bytes());
            bytes.extend_from_slice(&coord.y.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, FrontierManifestCodecError> {
        if payload.len() < FRONTIER_MANIFEST_HEADER_BYTES {
            return Err(FrontierManifestCodecError::PayloadTooShort {
                actual: payload.len(),
                minimum: FRONTIER_MANIFEST_HEADER_BYTES,
            });
        }

        let quadrant_count = decode_u16(payload, 24)?;
        let expected_len = payload_len(quadrant_count)?;
        if payload.len() != expected_len {
            return Err(FrontierManifestCodecError::InvalidLength {
                expected: expected_len,
                actual: payload.len(),
            });
        }

        let mut quadrants = Vec::with_capacity(usize::from(quadrant_count));
        let mut offset = FRONTIER_MANIFEST_HEADER_BYTES;
        for _ in 0..quadrant_count {
            let x = decode_i64(payload, offset)?;
            let y = decode_i64(payload, offset + 8)?;
            quadrants.push(FrontierQuadrantCoord::new(x, y));
            offset += FRONTIER_MANIFEST_QUADRANT_BYTES;
        }

        let manifest = Self {
            manifest_version: decode_u32(payload, 0)?,
            generator_version: decode_u32(payload, 4)?,
            quadrant_size_mm: decode_i64(payload, 8)?,
            revision: decode_u64(payload, 16)?,
            quadrants,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), FrontierManifestCodecError> {
        if self.manifest_version != FRONTIER_MANIFEST_VERSION {
            return Err(FrontierManifestCodecError::UnsupportedVersion {
                received: self.manifest_version,
                supported: FRONTIER_MANIFEST_VERSION,
            });
        }
        if self.generator_version == 0 {
            return Err(FrontierManifestCodecError::ZeroValue("generator_version"));
        }
        if self.quadrant_size_mm <= 0 {
            return Err(FrontierManifestCodecError::InvalidQuadrantSize {
                value: self.quadrant_size_mm,
            });
        }
        if self.revision == 0 {
            return Err(FrontierManifestCodecError::ZeroValue("revision"));
        }
        if self.quadrants.is_empty() {
            return Err(FrontierManifestCodecError::EmptyManifest);
        }
        if self.quadrants.len() > usize::from(u16::MAX) {
            return Err(FrontierManifestCodecError::TooManyQuadrants {
                actual: self.quadrants.len(),
                maximum: usize::from(u16::MAX),
            });
        }

        let mut unique = BTreeSet::new();
        for &coord in &self.quadrants {
            if !unique.insert(coord) {
                return Err(FrontierManifestCodecError::DuplicateQuadrant {
                    x: coord.x,
                    y: coord.y,
                });
            }
        }
        Ok(())
    }
}

fn payload_len(quadrant_count: u16) -> Result<usize, FrontierManifestCodecError> {
    usize::from(quadrant_count)
        .checked_mul(FRONTIER_MANIFEST_QUADRANT_BYTES)
        .and_then(|bytes| FRONTIER_MANIFEST_HEADER_BYTES.checked_add(bytes))
        .ok_or(FrontierManifestCodecError::LengthOverflow)
}

fn decode_u16(payload: &[u8], start: usize) -> Result<u16, FrontierManifestCodecError> {
    let bytes = payload
        .get(start..start + 2)
        .ok_or(FrontierManifestCodecError::InvalidLength {
            expected: start + 2,
            actual: payload.len(),
        })?;
    Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| {
        FrontierManifestCodecError::InvalidLength {
            expected: start + 2,
            actual: payload.len(),
        }
    })?))
}

fn decode_u32(payload: &[u8], start: usize) -> Result<u32, FrontierManifestCodecError> {
    let bytes = payload
        .get(start..start + 4)
        .ok_or(FrontierManifestCodecError::InvalidLength {
            expected: start + 4,
            actual: payload.len(),
        })?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
        FrontierManifestCodecError::InvalidLength {
            expected: start + 4,
            actual: payload.len(),
        }
    })?))
}

fn decode_u64(payload: &[u8], start: usize) -> Result<u64, FrontierManifestCodecError> {
    let bytes = payload
        .get(start..start + 8)
        .ok_or(FrontierManifestCodecError::InvalidLength {
            expected: start + 8,
            actual: payload.len(),
        })?;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
        FrontierManifestCodecError::InvalidLength {
            expected: start + 8,
            actual: payload.len(),
        }
    })?))
}

fn decode_i64(payload: &[u8], start: usize) -> Result<i64, FrontierManifestCodecError> {
    let bytes = payload
        .get(start..start + 8)
        .ok_or(FrontierManifestCodecError::InvalidLength {
            expected: start + 8,
            actual: payload.len(),
        })?;
    Ok(i64::from_le_bytes(bytes.try_into().map_err(|_| {
        FrontierManifestCodecError::InvalidLength {
            expected: start + 8,
            actual: payload.len(),
        }
    })?))
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FrontierManifestCodecError {
    #[error("frontier manifest payload length {actual} is below minimum {minimum}")]
    PayloadTooShort { actual: usize, minimum: usize },
    #[error("frontier manifest payload length {actual} does not equal expected {expected}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("frontier manifest encoded length overflowed usize")]
    LengthOverflow,
    #[error("frontier manifest version {received} is unsupported; expected {supported}")]
    UnsupportedVersion { received: u32, supported: u32 },
    #[error("frontier manifest field {0} must be non-zero")]
    ZeroValue(&'static str),
    #[error("frontier manifest quadrant size must be positive, got {value} mm")]
    InvalidQuadrantSize { value: i64 },
    #[error("frontier manifest must contain at least one revealed quadrant")]
    EmptyManifest,
    #[error("frontier manifest contains {actual} quadrants; maximum is {maximum}")]
    TooManyQuadrants { actual: usize, maximum: usize },
    #[error("frontier manifest contains duplicate quadrant ({x}, {y})")]
    DuplicateQuadrant { x: i64, y: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn four_by_four() -> Vec<FrontierQuadrantCoord> {
        let mut quadrants = Vec::with_capacity(16);
        for y in -1..=2 {
            for x in -1..=2 {
                quadrants.push(FrontierQuadrantCoord::new(x, y));
            }
        }
        quadrants
    }

    #[test]
    fn frontier_manifest_roundtrips_exact_server_owned_quadrants() -> Result<(), FrontierManifestCodecError> {
        let manifest = FrontierManifest::new(1, 512_000, 1, four_by_four())?;
        let encoded = manifest.encode()?;
        let decoded = FrontierManifest::decode(&encoded)?;

        assert_eq!(decoded, manifest);
        assert_eq!(decoded.quadrants.len(), 16);
        assert_eq!(decoded.quadrants[0], FrontierQuadrantCoord::new(-1, -1));
        assert_eq!(decoded.quadrants[15], FrontierQuadrantCoord::new(2, 2));
        assert_eq!(
            encoded.len(),
            FRONTIER_MANIFEST_HEADER_BYTES + 16 * FRONTIER_MANIFEST_QUADRANT_BYTES
        );
        Ok(())
    }

    #[test]
    fn frontier_manifest_rejects_duplicate_quadrants() {
        let duplicate = FrontierQuadrantCoord::new(0, 0);
        assert!(matches!(
            FrontierManifest::new(1, 512_000, 1, vec![duplicate, duplicate]),
            Err(FrontierManifestCodecError::DuplicateQuadrant { x: 0, y: 0 })
        ));
    }

    #[test]
    fn frontier_manifest_rejects_invalid_geometry_and_revision() {
        assert!(matches!(
            FrontierManifest::new(1, 0, 1, vec![FrontierQuadrantCoord::new(0, 0)]),
            Err(FrontierManifestCodecError::InvalidQuadrantSize { value: 0 })
        ));
        assert!(matches!(
            FrontierManifest::new(1, 512_000, 0, vec![FrontierQuadrantCoord::new(0, 0)]),
            Err(FrontierManifestCodecError::ZeroValue("revision"))
        ));
    }

    #[test]
    fn frontier_manifest_rejects_truncated_and_trailing_payloads() -> Result<(), FrontierManifestCodecError> {
        let manifest = FrontierManifest::new(1, 512_000, 1, four_by_four())?;
        let encoded = manifest.encode()?;

        assert!(FrontierManifest::decode(&encoded[..encoded.len() - 1]).is_err());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(FrontierManifest::decode(&trailing).is_err());
        Ok(())
    }
}
