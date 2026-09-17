use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

const QUADRANT_SEED_DOMAIN_V1: &[u8] = b"AURENFALL_QUADRANT_V1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SectorCoord {
    pub x: i64,
    pub y: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuadrantCoord {
    x: i64,
    y: i64,
}

impl QuadrantCoord {
    #[must_use]
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> i64 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i64 {
        self.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuadrantSizeMm(i64);

impl QuadrantSizeMm {
    pub fn new(value: i64) -> Result<Self, WorldGridError> {
        if value <= 0 {
            return Err(WorldGridError::InvalidQuadrantSize { value });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn value(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorldPositionMm {
    x: i64,
    y: i64,
    z: i64,
}

impl WorldPositionMm {
    pub const ORIGIN: Self = Self::new(0, 0, 0);

    #[must_use]
    pub const fn new(x: i64, y: i64, z: i64) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub const fn x(self) -> i64 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i64 {
        self.y
    }

    #[must_use]
    pub const fn z(self) -> i64 {
        self.z
    }

    #[must_use]
    pub fn quadrant_coord(self, quadrant_size: QuadrantSizeMm) -> QuadrantCoord {
        QuadrantCoord::new(
            self.x.div_euclid(quadrant_size.value()),
            self.y.div_euclid(quadrant_size.value()),
        )
    }

    #[must_use]
    pub fn checked_translate_horizontal(self, delta_x_mm: i64, delta_y_mm: i64) -> Option<Self> {
        Some(Self {
            x: self.x.checked_add(delta_x_mm)?,
            y: self.y.checked_add(delta_y_mm)?,
            z: self.z,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniverseSeed([u8; 32]);

impl UniverseSeed {
    #[must_use]
    pub fn from_phrase(phrase: &str) -> Self {
        Self(*blake3::hash(phrase.as_bytes()).as_bytes())
    }

    #[must_use]
    pub fn quadrant_seed(&self, coord: QuadrantCoord, generator_version: u32) -> QuadrantSeed {
        let mut hasher = blake3::Hasher::new_keyed(&self.0);
        hasher.update(QUADRANT_SEED_DOMAIN_V1);
        hasher.update(&generator_version.to_le_bytes());
        hasher.update(&coord.x().to_le_bytes());
        hasher.update(&coord.y().to_le_bytes());
        QuadrantSeed(*hasher.finalize().as_bytes())
    }

    #[must_use]
    pub fn sector_seed(&self, x: i64, y: i64, generator_version: u32) -> SectorSeed {
        let mut hasher = blake3::Hasher::new_keyed(&self.0);
        hasher.update(&x.to_le_bytes());
        hasher.update(&y.to_le_bytes());
        hasher.update(&generator_version.to_le_bytes());
        SectorSeed(*hasher.finalize().as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuadrantSeed([u8; 32]);

impl QuadrantSeed {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for QuadrantSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectorSeed([u8; 32]);

impl fmt::Display for SectorSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldGridError {
    InvalidQuadrantSize { value: i64 },
}

impl fmt::Display for WorldGridError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuadrantSize { value } => {
                write!(formatter, "quadrant size must be positive, got {value} mm")
            }
        }
    }
}

impl Error for WorldGridError {}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTOTYPE_QUADRANT_SIZE_MM: i64 = 512_000;

    fn quadrant_size() -> Result<QuadrantSizeMm, WorldGridError> {
        QuadrantSizeMm::new(PROTOTYPE_QUADRANT_SIZE_MM)
    }

    #[test]
    fn quadrant_size_rejects_zero_and_negative_values() {
        assert_eq!(
            QuadrantSizeMm::new(0),
            Err(WorldGridError::InvalidQuadrantSize { value: 0 })
        );
        assert_eq!(
            QuadrantSizeMm::new(-1),
            Err(WorldGridError::InvalidQuadrantSize { value: -1 })
        );
    }

    #[test]
    fn world_position_maps_origin_and_positive_boundaries() -> Result<(), WorldGridError> {
        let size = quadrant_size()?;
        assert_eq!(
            WorldPositionMm::new(0, 0, 999).quadrant_coord(size),
            QuadrantCoord::new(0, 0)
        );
        assert_eq!(
            WorldPositionMm::new(511_999, 511_999, -999).quadrant_coord(size),
            QuadrantCoord::new(0, 0)
        );
        assert_eq!(
            WorldPositionMm::new(512_000, 0, 0).quadrant_coord(size),
            QuadrantCoord::new(1, 0)
        );
        assert_eq!(
            WorldPositionMm::new(1_024_000, 0, 0).quadrant_coord(size),
            QuadrantCoord::new(2, 0)
        );
        Ok(())
    }

    #[test]
    fn world_position_uses_euclidean_division_for_negative_coordinates() -> Result<(), WorldGridError> {
        let size = quadrant_size()?;
        assert_eq!(
            WorldPositionMm::new(-1, 0, 0).quadrant_coord(size),
            QuadrantCoord::new(-1, 0)
        );
        assert_eq!(
            WorldPositionMm::new(-511_999, 0, 0).quadrant_coord(size),
            QuadrantCoord::new(-1, 0)
        );
        assert_eq!(
            WorldPositionMm::new(-512_000, 0, 0).quadrant_coord(size),
            QuadrantCoord::new(-1, 0)
        );
        assert_eq!(
            WorldPositionMm::new(-512_001, 0, 0).quadrant_coord(size),
            QuadrantCoord::new(-2, 0)
        );
        assert_eq!(
            WorldPositionMm::new(-1_024_000, 0, 0).quadrant_coord(size),
            QuadrantCoord::new(-2, 0)
        );
        Ok(())
    }

    #[test]
    fn quadrant_mapping_supports_mixed_and_extreme_coordinates() -> Result<(), WorldGridError> {
        let size = quadrant_size()?;
        assert_eq!(
            WorldPositionMm::new(512_000, -1, 123).quadrant_coord(size),
            QuadrantCoord::new(1, -1)
        );
        assert_eq!(
            WorldPositionMm::new(-1, 512_000, -123).quadrant_coord(size),
            QuadrantCoord::new(-1, 1)
        );
        assert_eq!(
            WorldPositionMm::new(i64::MAX, i64::MIN, 0).quadrant_coord(size),
            QuadrantCoord::new(
                i64::MAX.div_euclid(PROTOTYPE_QUADRANT_SIZE_MM),
                i64::MIN.div_euclid(PROTOTYPE_QUADRANT_SIZE_MM),
            )
        );
        Ok(())
    }

    #[test]
    fn quadrant_seed_is_repeatable_versioned_and_coordinate_sensitive() {
        let universe = UniverseSeed::from_phrase("test-universe");
        let coord = QuadrantCoord::new(10, -4);
        let a = universe.quadrant_seed(coord, 1);
        let replay = universe.quadrant_seed(coord, 1);
        let other_coord = universe.quadrant_seed(QuadrantCoord::new(11, -4), 1);
        let next_version = universe.quadrant_seed(coord, 2);

        assert_eq!(a, replay);
        assert_ne!(a, other_coord);
        assert_ne!(a, next_version);
    }

    #[test]
    fn quadrant_seed_varies_between_universes() {
        let coord = QuadrantCoord::new(123_456, -987_654);
        let a = UniverseSeed::from_phrase("universe-a").quadrant_seed(coord, 1);
        let b = UniverseSeed::from_phrase("universe-b").quadrant_seed(coord, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn quadrant_seed_encoding_is_domain_separated_and_explicit() {
        let universe = UniverseSeed::from_phrase("encoding-vector");
        let coord = QuadrantCoord::new(-7, 42);
        let generator_version = 9;

        let derived = universe.quadrant_seed(coord, generator_version);
        let mut hasher = blake3::Hasher::new_keyed(&universe.0);
        hasher.update(b"AURENFALL_QUADRANT_V1");
        hasher.update(&generator_version.to_le_bytes());
        hasher.update(&coord.x().to_le_bytes());
        hasher.update(&coord.y().to_le_bytes());

        assert_eq!(derived.as_bytes(), hasher.finalize().as_bytes());
    }

    #[test]
    fn sector_generation_is_deterministic_and_versioned() {
        let universe = UniverseSeed::from_phrase("test-universe");
        let a = universe.sector_seed(10, -4, 1);
        let replay = universe.sector_seed(10, -4, 1);
        let next_version = universe.sector_seed(10, -4, 2);
        assert_eq!(a, replay);
        assert_ne!(a, next_version);
    }

    #[test]
    fn world_position_translation_is_checked_and_preserves_height() {
        let position = WorldPositionMm::new(1_000, -2_000, 350);
        assert_eq!(
            position.checked_translate_horizontal(250, 500),
            Some(WorldPositionMm::new(1_250, -1_500, 350))
        );
        assert_eq!(
            WorldPositionMm::new(i64::MAX, 0, 0).checked_translate_horizontal(1, 0),
            None
        );
    }
}
