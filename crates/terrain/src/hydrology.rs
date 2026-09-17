use aurenfall_contracts::{
    FrontierQuadrantCoord, TERRAIN_CELL_BLOCKED, TERRAIN_CELL_WATER, TerrainAuthorityContractV1,
};
use thiserror::Error;

const LAKE_REGION_SIDE: i64 = 48;
const LAKE_HALF_REGION: i64 = LAKE_REGION_SIDE / 2;
const RIVER_BAND_SPACING: i64 = 96;
const RIVER_HALF_BAND: i64 = RIVER_BAND_SPACING / 2;
const RIVER_ANCHOR_SPACING: i64 = 32;

const LAKE_TAG: u64 = 0x1A4E_0001;
const RIVER_BAND_TAG: u64 = 0xA17E_0001;
const RIVER_MEANDER_TAG: u64 = 0xA17E_1001;

const LAKE_BASIN_DEPTH_MM: i32 = 2_000;
const RIVER_CHANNEL_DEPTH_MM: i32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HydrologyFieldV1Settings {
    pub world_seed: u64,
    pub generator_version: u32,
}

impl HydrologyFieldV1Settings {
    pub fn validate(self) -> Result<Self, HydrologyFieldV1Error> {
        if self.generator_version == 0 {
            return Err(HydrologyFieldV1Error::InvalidGeneratorVersion);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrologyKindV1 {
    Dry,
    Lake,
    River,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HydrologySampleV1 {
    pub kind: HydrologyKindV1,
    pub water_surface_mm: Option<i32>,
    pub flow_hint_x: i8,
    pub flow_hint_y: i8,
}

impl HydrologySampleV1 {
    #[must_use]
    pub const fn dry() -> Self {
        Self {
            kind: HydrologyKindV1::Dry,
            water_surface_mm: None,
            flow_hint_x: 0,
            flow_hint_y: 0,
        }
    }

    #[must_use]
    pub const fn lake(water_surface_mm: i32) -> Self {
        Self {
            kind: HydrologyKindV1::Lake,
            water_surface_mm: Some(water_surface_mm),
            flow_hint_x: 0,
            flow_hint_y: 0,
        }
    }

    #[must_use]
    pub const fn river(water_surface_mm: i32) -> Self {
        Self {
            kind: HydrologyKindV1::River,
            water_surface_mm: Some(water_surface_mm),
            flow_hint_x: 0,
            flow_hint_y: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HydrologyFieldV1Error {
    #[error("hydrology generator version must be non-zero")]
    InvalidGeneratorVersion,
    #[error("hydrology lattice coordinate overflow for quadrant ({x},{y}) local ({local_x},{local_y})")]
    LatticeCoordinateOverflow {
        x: i64,
        y: i64,
        local_x: i64,
        local_y: i64,
    },
    #[error("samples per quadrant must be positive, got {0}")]
    InvalidSamplesPerQuadrant(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HydrologyTerrainIntegrationError {
    #[error("terrain authority is invalid before hydrology integration: {0}")]
    InvalidTerrainAuthority(String),
    #[error(
        "terrain generator version {terrain_generator_version} does not match hydrology generator version {hydrology_generator_version}"
    )]
    GeneratorVersionMismatch {
        terrain_generator_version: u32,
        hydrology_generator_version: u32,
    },
    #[error(transparent)]
    Hydrology(#[from] HydrologyFieldV1Error),
    #[error("terrain authority is invalid after hydrology integration: {0}")]
    InvalidIntegratedTerrain(String),
}

#[derive(Debug, Clone, Copy)]
struct LakeCandidate {
    center_x: i64,
    center_y: i64,
    radius: i64,
    water_surface_mm: i32,
    identity_hash: u64,
}

#[derive(Debug, Clone, Copy)]
struct RiverCandidate {
    center_x: i64,
    half_width: i64,
    water_surface_mm: i32,
    identity_hash: u64,
}

pub fn sample_hydrology_v1(
    settings: HydrologyFieldV1Settings,
    global_x: i64,
    global_y: i64,
) -> Result<HydrologySampleV1, HydrologyFieldV1Error> {
    let settings = settings.validate()?;

    if let Some(lake) = select_lake_candidate(settings, global_x, global_y) {
        return Ok(HydrologySampleV1::lake(lake.water_surface_mm));
    }

    if let Some(river) = select_river_candidate(settings, global_x, global_y) {
        return Ok(HydrologySampleV1::river(river.water_surface_mm));
    }

    Ok(HydrologySampleV1::dry())
}

pub fn sample_hydrology_for_quadrant_local_v1(
    settings: HydrologyFieldV1Settings,
    quadrant: FrontierQuadrantCoord,
    local_x: i64,
    local_y: i64,
    samples_per_quadrant: i64,
) -> Result<HydrologySampleV1, HydrologyFieldV1Error> {
    if samples_per_quadrant <= 0 {
        return Err(HydrologyFieldV1Error::InvalidSamplesPerQuadrant(
            samples_per_quadrant,
        ));
    }

    let base_x = quadrant.x.checked_mul(samples_per_quadrant).ok_or(
        HydrologyFieldV1Error::LatticeCoordinateOverflow {
            x: quadrant.x,
            y: quadrant.y,
            local_x,
            local_y,
        },
    )?;
    let base_y = quadrant.y.checked_mul(samples_per_quadrant).ok_or(
        HydrologyFieldV1Error::LatticeCoordinateOverflow {
            x: quadrant.x,
            y: quadrant.y,
            local_x,
            local_y,
        },
    )?;
    let global_x = base_x
        .checked_add(local_x)
        .ok_or(HydrologyFieldV1Error::LatticeCoordinateOverflow {
            x: quadrant.x,
            y: quadrant.y,
            local_x,
            local_y,
        })?;
    let global_y = base_y
        .checked_add(local_y)
        .ok_or(HydrologyFieldV1Error::LatticeCoordinateOverflow {
            x: quadrant.x,
            y: quadrant.y,
            local_x,
            local_y,
        })?;

    sample_hydrology_v1(settings, global_x, global_y)
}

pub fn apply_hydrology_to_terrain_v1(
    settings: HydrologyFieldV1Settings,
    terrain: &TerrainAuthorityContractV1,
) -> Result<TerrainAuthorityContractV1, HydrologyTerrainIntegrationError> {
    terrain
        .validate()
        .map_err(|error| HydrologyTerrainIntegrationError::InvalidTerrainAuthority(error.to_string()))?;
    let settings = settings.validate()?;
    if terrain.generator_version != settings.generator_version {
        return Err(HydrologyTerrainIntegrationError::GeneratorVersionMismatch {
            terrain_generator_version: terrain.generator_version,
            hydrology_generator_version: settings.generator_version,
        });
    }

    let side = usize::from(terrain.control_grid_side);
    let samples_per_quadrant = i64::from(terrain.control_grid_side - 1);
    let mut integrated = terrain.clone();
    let mut wet_vertices = vec![false; terrain.elevation_samples_mm.len()];

    for local_y in 0..side {
        for local_x in 0..side {
            let sample = sample_hydrology_for_quadrant_local_v1(
                settings,
                terrain.quadrant_coord,
                local_x as i64,
                local_y as i64,
                samples_per_quadrant,
            )?;
            let Some(water_surface_mm) = sample.water_surface_mm else {
                continue;
            };

            let depth_mm = match sample.kind {
                HydrologyKindV1::Lake => LAKE_BASIN_DEPTH_MM,
                HydrologyKindV1::River => RIVER_CHANNEL_DEPTH_MM,
                HydrologyKindV1::Dry => continue,
            };
            let index = local_y * side + local_x;
            let carved_ceiling_mm = water_surface_mm.saturating_sub(depth_mm);
            integrated.elevation_samples_mm[index] =
                integrated.elevation_samples_mm[index].min(carved_ceiling_mm);
            wet_vertices[index] = true;
        }
    }

    let cell_side = side - 1;
    for cell_y in 0..cell_side {
        for cell_x in 0..cell_side {
            let i00 = cell_y * side + cell_x;
            let i10 = i00 + 1;
            let i01 = i00 + side;
            let i11 = i01 + 1;
            if wet_vertices[i00] || wet_vertices[i10] || wet_vertices[i01] || wet_vertices[i11] {
                integrated.cell_flags[cell_y * cell_side + cell_x] =
                    TERRAIN_CELL_WATER | TERRAIN_CELL_BLOCKED;
            }
        }
    }

    integrated
        .validate()
        .map_err(|error| HydrologyTerrainIntegrationError::InvalidIntegratedTerrain(error.to_string()))?;
    Ok(integrated)
}

fn select_lake_candidate(
    settings: HydrologyFieldV1Settings,
    global_x: i64,
    global_y: i64,
) -> Option<LakeCandidate> {
    let region_x = global_x.div_euclid(LAKE_REGION_SIDE);
    let region_y = global_y.div_euclid(LAKE_REGION_SIDE);
    let mut best: Option<(i128, u64, LakeCandidate)> = None;

    for candidate_region_y in region_y.saturating_sub(1)..=region_y.saturating_add(1) {
        for candidate_region_x in region_x.saturating_sub(1)..=region_x.saturating_add(1) {
            let Some(candidate) = lake_candidate(settings, candidate_region_x, candidate_region_y) else {
                continue;
            };
            let dx = i128::from(global_x) - i128::from(candidate.center_x);
            let dy = i128::from(global_y) - i128::from(candidate.center_y);
            let distance_sq = dx * dx + dy * dy;
            let radius_sq = i128::from(candidate.radius) * i128::from(candidate.radius);
            if distance_sq > radius_sq {
                continue;
            }

            let key = (distance_sq, candidate.identity_hash);
            if best
                .as_ref()
                .is_none_or(|(best_distance, best_hash, _)| key < (*best_distance, *best_hash))
            {
                best = Some((distance_sq, candidate.identity_hash, candidate));
            }
        }
    }

    best.map(|(_, _, candidate)| candidate)
}

fn lake_candidate(settings: HydrologyFieldV1Settings, region_x: i64, region_y: i64) -> Option<LakeCandidate> {
    let hash = hash_xy(settings, LAKE_TAG, region_x, region_y);
    if hash & 3 != 0 {
        return None;
    }

    let offset_x = signed_bucket(hash >> 8, -16, 16);
    let offset_y = signed_bucket(hash >> 16, -16, 16);
    let radius = 5 + ((hash >> 32) % 7) as i64;
    let water_surface_mm = -2_000 + ((hash >> 40) % 4_001) as i32;
    let center_x = region_x
        .checked_mul(LAKE_REGION_SIDE)?
        .checked_add(LAKE_HALF_REGION)?
        .checked_add(offset_x)?;
    let center_y = region_y
        .checked_mul(LAKE_REGION_SIDE)?
        .checked_add(LAKE_HALF_REGION)?
        .checked_add(offset_y)?;

    Some(LakeCandidate {
        center_x,
        center_y,
        radius,
        water_surface_mm,
        identity_hash: hash,
    })
}

fn select_river_candidate(
    settings: HydrologyFieldV1Settings,
    global_x: i64,
    global_y: i64,
) -> Option<RiverCandidate> {
    let band_x = global_x.div_euclid(RIVER_BAND_SPACING);
    let mut best: Option<(u64, u64, RiverCandidate)> = None;

    for candidate_band in band_x.saturating_sub(1)..=band_x.saturating_add(1) {
        let candidate = river_candidate(settings, candidate_band, global_y);
        let distance = global_x.abs_diff(candidate.center_x);
        if distance > candidate.half_width as u64 {
            continue;
        }

        let key = (distance, candidate.identity_hash);
        if best
            .as_ref()
            .is_none_or(|(best_distance, best_hash, _)| key < (*best_distance, *best_hash))
        {
            best = Some((distance, candidate.identity_hash, candidate));
        }
    }

    best.map(|(_, _, candidate)| candidate)
}

fn river_candidate(settings: HydrologyFieldV1Settings, band_x: i64, global_y: i64) -> RiverCandidate {
    let band_hash = hash_xy(settings, RIVER_BAND_TAG, band_x, 0);
    let base_center_x_i128 = i128::from(band_x) * i128::from(RIVER_BAND_SPACING)
        + i128::from(RIVER_HALF_BAND)
        + i128::from(signed_bucket(band_hash >> 8, -16, 16));
    let base_center_x = base_center_x_i128.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
    let half_width = 2 + ((band_hash >> 24) % 3) as i64;
    let base_surface_mm = -1_500 + ((band_hash >> 32) % 3_001) as i64;

    let anchor_y = global_y.div_euclid(RIVER_ANCHOR_SPACING);
    let fraction = global_y.rem_euclid(RIVER_ANCHOR_SPACING);
    let south_offset = river_anchor_offset(settings, band_x, anchor_y);
    let north_offset = river_anchor_offset(settings, band_x, anchor_y.saturating_add(1));
    let interpolated_offset =
        (south_offset * (RIVER_ANCHOR_SPACING - fraction) + north_offset * fraction) / RIVER_ANCHOR_SPACING;

    let center_x = i128::from(base_center_x)
        .saturating_add(i128::from(interpolated_offset))
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
    let water_surface_mm = (i128::from(base_surface_mm) - i128::from(global_y) * 25)
        .clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32;

    RiverCandidate {
        center_x,
        half_width,
        water_surface_mm,
        identity_hash: band_hash,
    }
}

fn river_anchor_offset(settings: HydrologyFieldV1Settings, band_x: i64, anchor_y: i64) -> i64 {
    let hash = hash_xy(settings, RIVER_MEANDER_TAG ^ band_x as u64, band_x, anchor_y);
    signed_bucket(hash >> 16, -12, 12)
}

fn signed_bucket(value: u64, min: i64, max: i64) -> i64 {
    debug_assert!(min <= max);
    let span = (max - min + 1) as u64;
    min + (value % span) as i64
}

fn hash_xy(settings: HydrologyFieldV1Settings, tag: u64, x: i64, y: i64) -> u64 {
    let mut state = settings.world_seed ^ (u64::from(settings.generator_version) << 32) ^ tag;
    state = mix64(state ^ x as u64);
    mix64(state ^ (y as u64).rotate_left(29))
}

fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::{
        TerrainAuthorityArtifactV1, TerrainRecipeV1Settings, TerrainSemanticSummary,
        generate_terrain_authority_v1,
    };
    use aurenfall_contracts::{TERRAIN_CELL_BUILDABLE, TERRAIN_CELL_WALKABLE};

    const SAMPLES_PER_QUADRANT: i64 = 8;

    fn settings() -> HydrologyFieldV1Settings {
        HydrologyFieldV1Settings {
            world_seed: 0xA11C_EFA1_1A11_CE01,
            generator_version: 1,
        }
    }

    fn terrain_settings() -> TerrainRecipeV1Settings {
        TerrainRecipeV1Settings {
            quadrant_size_mm: 64_000,
            generator_version: settings().generator_version,
            world_seed: settings().world_seed,
        }
    }

    #[test]
    fn same_inputs_same_hydrology_sample() {
        let first = sample_hydrology_v1(settings(), 33, -128).unwrap();
        let replay = sample_hydrology_v1(settings(), 33, -128).unwrap();
        assert_eq!(first, replay);
    }

    #[test]
    fn same_global_coordinate_independent_of_quadrant_partition() {
        let west = sample_hydrology_for_quadrant_local_v1(
            settings(),
            FrontierQuadrantCoord { x: 0, y: -1 },
            SAMPLES_PER_QUADRANT,
            3,
            SAMPLES_PER_QUADRANT,
        )
        .unwrap();
        let east = sample_hydrology_for_quadrant_local_v1(
            settings(),
            FrontierQuadrantCoord { x: 1, y: -1 },
            0,
            3,
            SAMPLES_PER_QUADRANT,
        )
        .unwrap();
        assert_eq!(west, east);
    }

    #[test]
    fn adjacent_quadrant_shared_boundary_hydrology_matches() {
        let west = FrontierQuadrantCoord { x: 4, y: -3 };
        let east = FrontierQuadrantCoord { x: 5, y: -3 };
        for local_y in 0..=SAMPLES_PER_QUADRANT {
            let west_edge = sample_hydrology_for_quadrant_local_v1(
                settings(),
                west,
                SAMPLES_PER_QUADRANT,
                local_y,
                SAMPLES_PER_QUADRANT,
            )
            .unwrap();
            let east_edge =
                sample_hydrology_for_quadrant_local_v1(settings(), east, 0, local_y, SAMPLES_PER_QUADRANT)
                    .unwrap();
            assert_eq!(west_edge, east_edge);
        }
    }

    #[test]
    fn lake_fixture_is_reachable() {
        let sample = sample_hydrology_v1(settings(), 33, -128).unwrap();
        assert_eq!(sample.kind, HydrologyKindV1::Lake);
        assert_eq!(sample.water_surface_mm, Some(-1_047));
        assert_eq!((sample.flow_hint_x, sample.flow_hint_y), (0, 0));
    }

    #[test]
    fn river_fixture_is_reachable() {
        let sample = sample_hydrology_v1(settings(), -46, -128).unwrap();
        assert_eq!(sample.kind, HydrologyKindV1::River);
        assert_eq!(sample.water_surface_mm, Some(1_865));
        assert_eq!((sample.flow_hint_x, sample.flow_hint_y), (0, 1));
    }

    #[test]
    fn water_surface_guidance_is_deterministic() {
        let lake = sample_hydrology_v1(settings(), 33, -128).unwrap();
        let river = sample_hydrology_v1(settings(), -46, -128).unwrap();
        assert_eq!(lake.water_surface_mm, Some(-1_047));
        assert_eq!(river.water_surface_mm, Some(1_865));
    }

    #[test]
    fn invalid_generator_version_is_rejected() {
        let error = sample_hydrology_v1(
            HydrologyFieldV1Settings {
                world_seed: 7,
                generator_version: 0,
            },
            0,
            0,
        )
        .unwrap_err();
        assert_eq!(error, HydrologyFieldV1Error::InvalidGeneratorVersion);
    }

    #[test]
    fn hydrology_terrain_integration_is_deterministic() {
        let terrain =
            generate_terrain_authority_v1(terrain_settings(), FrontierQuadrantCoord { x: 4, y: -16 })
                .unwrap();
        let first = apply_hydrology_to_terrain_v1(settings(), &terrain).unwrap();
        let replay = apply_hydrology_to_terrain_v1(settings(), &terrain).unwrap();
        assert_eq!(first, replay);
    }

    #[test]
    fn hydrology_lake_integration_carves_basin_and_marks_water() {
        let terrain =
            generate_terrain_authority_v1(terrain_settings(), FrontierQuadrantCoord { x: 4, y: -16 })
                .unwrap();
        let original_vertex = terrain.elevation_samples_mm[1];
        let integrated = apply_hydrology_to_terrain_v1(settings(), &terrain).unwrap();
        let sample = sample_hydrology_v1(settings(), 33, -128).unwrap();
        let water_surface_mm = sample.water_surface_mm.unwrap();

        assert_eq!(sample.kind, HydrologyKindV1::Lake);
        assert!(integrated.elevation_samples_mm[1] <= water_surface_mm - LAKE_BASIN_DEPTH_MM);
        assert!(integrated.elevation_samples_mm[1] <= original_vertex);

        let summary = TerrainSemanticSummary::from_contract(&integrated);
        assert!(summary.water_cells > 0);
        for flags in integrated.cell_flags {
            if flags & TERRAIN_CELL_WATER != 0 {
                assert_ne!(flags & TERRAIN_CELL_BLOCKED, 0);
                assert_eq!(flags & TERRAIN_CELL_WALKABLE, 0);
                assert_eq!(flags & TERRAIN_CELL_BUILDABLE, 0);
            }
        }
    }

    #[test]
    fn hydrology_river_integration_carves_channel_and_marks_water() {
        let terrain =
            generate_terrain_authority_v1(terrain_settings(), FrontierQuadrantCoord { x: -6, y: -16 })
                .unwrap();
        let integrated = apply_hydrology_to_terrain_v1(settings(), &terrain).unwrap();
        let sample = sample_hydrology_v1(settings(), -46, -128).unwrap();
        let water_surface_mm = sample.water_surface_mm.unwrap();

        assert_eq!(sample.kind, HydrologyKindV1::River);
        assert!(integrated.elevation_samples_mm[2] <= water_surface_mm - RIVER_CHANNEL_DEPTH_MM);
        assert!(TerrainSemanticSummary::from_contract(&integrated).water_cells > 0);
    }

    #[test]
    fn hydrology_integrated_adjacent_terrain_edges_remain_identical() {
        let west = generate_terrain_authority_v1(terrain_settings(), FrontierQuadrantCoord { x: 4, y: -16 })
            .unwrap();
        let east = generate_terrain_authority_v1(terrain_settings(), FrontierQuadrantCoord { x: 5, y: -16 })
            .unwrap();
        let west = apply_hydrology_to_terrain_v1(settings(), &west).unwrap();
        let east = apply_hydrology_to_terrain_v1(settings(), &east).unwrap();
        let side = usize::from(west.control_grid_side);

        for y in 0..side {
            assert_eq!(
                west.elevation_samples_mm[y * side + side - 1],
                east.elevation_samples_mm[y * side]
            );
        }
    }

    #[test]
    fn hydrology_integrated_water_receives_zero_environment_family_mask() {
        let terrain =
            generate_terrain_authority_v1(terrain_settings(), FrontierQuadrantCoord { x: 4, y: -16 })
                .unwrap();
        let integrated = apply_hydrology_to_terrain_v1(settings(), &terrain).unwrap();
        let artifact = TerrainAuthorityArtifactV1::from_contract(integrated.clone());
        let policy = artifact
            .environment_presentation_policy_v1(settings().world_seed)
            .unwrap();
        let mut water_cells = 0;

        for (flags, mask) in integrated.cell_flags.iter().zip(policy.cell_family_masks.iter()) {
            if flags & TERRAIN_CELL_WATER != 0 {
                water_cells += 1;
                assert_eq!(*mask, 0);
            }
        }
        assert!(water_cells > 0);
    }

    #[test]
    fn hydrology_terrain_integration_rejects_generator_version_mismatch() {
        let terrain =
            generate_terrain_authority_v1(terrain_settings(), FrontierQuadrantCoord { x: 0, y: 0 }).unwrap();
        let error = apply_hydrology_to_terrain_v1(
            HydrologyFieldV1Settings {
                world_seed: settings().world_seed,
                generator_version: 2,
            },
            &terrain,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            HydrologyTerrainIntegrationError::GeneratorVersionMismatch { .. }
        ));
    }
}
