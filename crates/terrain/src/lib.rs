mod artifact;
mod hydrology;
mod semantics;
mod water_surface_presentation;

pub use artifact::TerrainAuthorityArtifactV1;
pub use hydrology::{
    HydrologyFieldV1Error, HydrologyFieldV1Settings, HydrologyKindV1, HydrologySampleV1,
    HydrologyTerrainIntegrationError, apply_hydrology_to_terrain_v1, sample_hydrology_for_quadrant_local_v1,
    sample_hydrology_v1,
};
pub use semantics::TerrainSemanticSummary;
pub use water_surface_presentation::{
    WaterSurfacePresentationGenerationError, generate_water_surface_presentation_v1,
};

use aurenfall_contracts::{
    FrontierQuadrantCoord, TERRAIN_AUTHORITY_CONTRACT_VERSION, TERRAIN_CELL_BLOCKED, TERRAIN_CELL_BUILDABLE,
    TERRAIN_CELL_WALKABLE, TerrainAuthorityContractV1,
};
use thiserror::Error;

pub const TERRAIN_RECIPE_V1_CONTROL_GRID_SIDE: u16 = 9;
pub const TERRAIN_RECIPE_V1_MAX_RELIEF_MM: i32 = 18_000;

// Authoritative coarse semantic thresholds expressed as rise/run per mille.
// They describe the TerrainAuthority control surface, not the final UE tessellation.
pub const TERRAIN_RECIPE_V1_BUILDABLE_MAX_GRADE_PER_MILLE: i64 = 150;
pub const TERRAIN_RECIPE_V1_WALKABLE_MAX_GRADE_PER_MILLE: i64 = 700;

const FIXED_ONE: i64 = 1 << 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainRecipeV1Settings {
    pub quadrant_size_mm: i64,
    pub generator_version: u32,
    pub world_seed: u64,
}

impl TerrainRecipeV1Settings {
    pub fn validate(self) -> Result<Self, TerrainRecipeError> {
        if self.quadrant_size_mm <= 0 {
            return Err(TerrainRecipeError::InvalidQuadrantSize(self.quadrant_size_mm));
        }
        if self.generator_version == 0 {
            return Err(TerrainRecipeError::InvalidGeneratorVersion);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TerrainRecipeError {
    #[error("quadrant size must be positive, got {0}")]
    InvalidQuadrantSize(i64),
    #[error("generator version must be non-zero")]
    InvalidGeneratorVersion,
    #[error("terrain lattice coordinate overflow for quadrant ({x},{y})")]
    LatticeCoordinateOverflow { x: i64, y: i64 },
    #[error("generated terrain contract failed validation: {0}")]
    InvalidGeneratedContract(String),
}

#[must_use]
pub fn derive_quadrant_recipe_seed(
    world_seed: u64,
    generator_version: u32,
    coord: FrontierQuadrantCoord,
) -> u64 {
    let mut state = world_seed ^ (u64::from(generator_version) << 32);
    state = mix64(state ^ coord.x as u64);
    mix64(state ^ (coord.y as u64).rotate_left(29))
}

#[must_use]
pub fn derive_quadrant_biome_id(world_seed: u64, coord: FrontierQuadrantCoord) -> u8 {
    // Lista padrão de biomas suportados atualmente no biomes.yaml
    const DEFAULT_BIOME_IDS: [u8; 2] = [1, 2];
    derive_quadrant_biome_from_pool(world_seed, coord, &DEFAULT_BIOME_IDS)
}

#[must_use]
pub fn derive_quadrant_biome_from_pool(
    world_seed: u64,
    coord: FrontierQuadrantCoord,
    available_biome_ids: &[u8],
) -> u8 {
    if available_biome_ids.is_empty() {
        return 1; // Fallback de segurança para o bioma padrão
    }

    // Mistura determinística com dispersão de 64 bits
    let mut state = world_seed.wrapping_add(0x517C_C1B7_2722_0A95);
    state = mix64(state ^ (coord.x as u64));
    state = mix64(state ^ ((coord.y as u64).rotate_left(31)));

    let index = (state as usize) % available_biome_ids.len();
    available_biome_ids[index]
}

pub fn generate_terrain_authority_v1(
    settings: TerrainRecipeV1Settings,
    coord: FrontierQuadrantCoord,
) -> Result<TerrainAuthorityContractV1, TerrainRecipeError> {
    let settings = settings.validate()?;
    let side = usize::from(TERRAIN_RECIPE_V1_CONTROL_GRID_SIDE);
    let samples_per_quadrant = i64::from(TERRAIN_RECIPE_V1_CONTROL_GRID_SIDE - 1);
    let base_x =
        coord
            .x
            .checked_mul(samples_per_quadrant)
            .ok_or(TerrainRecipeError::LatticeCoordinateOverflow {
                x: coord.x,
                y: coord.y,
            })?;
    let base_y =
        coord
            .y
            .checked_mul(samples_per_quadrant)
            .ok_or(TerrainRecipeError::LatticeCoordinateOverflow {
                x: coord.x,
                y: coord.y,
            })?;

    let mut elevation_samples_mm = Vec::with_capacity(side * side);
    for local_y in 0..side {
        for local_x in 0..side {
            let global_x =
                base_x
                    .checked_add(local_x as i64)
                    .ok_or(TerrainRecipeError::LatticeCoordinateOverflow {
                        x: coord.x,
                        y: coord.y,
                    })?;
            let global_y =
                base_y
                    .checked_add(local_y as i64)
                    .ok_or(TerrainRecipeError::LatticeCoordinateOverflow {
                        x: coord.x,
                        y: coord.y,
                    })?;
            elevation_samples_mm.push(authoritative_height_mm(
                settings.world_seed,
                settings.generator_version,
                global_x,
                global_y,
            ));
        }
    }

    let cell_side = side - 1;
    let mut cell_flags = Vec::with_capacity(cell_side * cell_side);
    for cell_y in 0..cell_side {
        for cell_x in 0..cell_side {
            let i00 = cell_y * side + cell_x;
            let i10 = i00 + 1;
            let i01 = i00 + side;
            let i11 = i01 + 1;
            cell_flags.push(derive_cell_flags(
                elevation_samples_mm[i00],
                elevation_samples_mm[i10],
                elevation_samples_mm[i01],
                elevation_samples_mm[i11],
                settings.quadrant_size_mm,
                cell_side as i64,
            ));
        }
    }

    let recipe_seed = derive_quadrant_recipe_seed(settings.world_seed, settings.generator_version, coord);
    let biome_id = derive_quadrant_biome_id(settings.world_seed, coord);

    let contract = TerrainAuthorityContractV1 {
        version: TERRAIN_AUTHORITY_CONTRACT_VERSION,
        quadrant_coord: coord,
        generator_version: settings.generator_version,
        quadrant_size_mm: settings.quadrant_size_mm,
        control_grid_side: TERRAIN_RECIPE_V1_CONTROL_GRID_SIDE,
        recipe_seed,
        biome_id,
        elevation_samples_mm,
        cell_flags,
    };
    contract
        .validate()
        .map_err(|error| TerrainRecipeError::InvalidGeneratedContract(error.to_string()))?;
    Ok(contract)
}

fn derive_cell_flags(
    h00: i32,
    h10: i32,
    h01: i32,
    h11: i32,
    quadrant_size_mm: i64,
    cells_per_side: i64,
) -> u8 {
    // Estimate the steepest coarse bilinear gradient from opposing cell edges.
    // Compare squared rise/run in integer space so server semantics stay deterministic.
    let dx = i64::from((h10 - h00).abs()).max(i64::from((h11 - h01).abs()));
    let dy = i64::from((h01 - h00).abs()).max(i64::from((h11 - h10).abs()));
    let rise_sq = i128::from(dx) * i128::from(dx) + i128::from(dy) * i128::from(dy);
    let cell_scale_sq = i128::from(cells_per_side) * i128::from(cells_per_side);
    let normalized_rise_sq = rise_sq * 1_000_000_i128 * cell_scale_sq;
    let run_sq = i128::from(quadrant_size_mm) * i128::from(quadrant_size_mm);

    let buildable_limit_sq = run_sq
        * i128::from(TERRAIN_RECIPE_V1_BUILDABLE_MAX_GRADE_PER_MILLE)
        * i128::from(TERRAIN_RECIPE_V1_BUILDABLE_MAX_GRADE_PER_MILLE);
    if normalized_rise_sq <= buildable_limit_sq {
        return TERRAIN_CELL_WALKABLE | TERRAIN_CELL_BUILDABLE;
    }

    let walkable_limit_sq = run_sq
        * i128::from(TERRAIN_RECIPE_V1_WALKABLE_MAX_GRADE_PER_MILLE)
        * i128::from(TERRAIN_RECIPE_V1_WALKABLE_MAX_GRADE_PER_MILLE);
    if normalized_rise_sq <= walkable_limit_sq {
        TERRAIN_CELL_WALKABLE
    } else {
        TERRAIN_CELL_BLOCKED
    }
}

fn authoritative_height_mm(world_seed: u64, generator_version: u32, x: i64, y: i64) -> i32 {
    // Integer-only multi-scale value noise. The global control sample coordinate is
    // the sole spatial input, so neighboring Quadrants evaluate shared edge samples
    // from exactly the same integers. Fixed-point interpolation avoids platform float
    // drift while removing the old div_euclid plateaus/steps.
    let broad = interpolated_lattice_value(world_seed, generator_version, x, y, 32);
    let regional =
        interpolated_lattice_value(world_seed ^ 0x9E37_79B9_7F4A_7C15, generator_version, x, y, 16);
    let local = interpolated_lattice_value(world_seed ^ 0xD1B5_4A32_D192_ED03, generator_version, x, y, 8);

    let weighted = i64::from(broad) * 5 + i64::from(regional) * 3 + i64::from(local) * 2;
    let normalized = weighted / 10;
    ((normalized * i64::from(TERRAIN_RECIPE_V1_MAX_RELIEF_MM)) / 32_768).clamp(
        -i64::from(TERRAIN_RECIPE_V1_MAX_RELIEF_MM),
        i64::from(TERRAIN_RECIPE_V1_MAX_RELIEF_MM),
    ) as i32
}

fn interpolated_lattice_value(seed: u64, generator_version: u32, x: i64, y: i64, spacing: i64) -> i32 {
    debug_assert!(spacing > 0);

    let cell_x = x.div_euclid(spacing);
    let cell_y = y.div_euclid(spacing);
    let frac_x = x.rem_euclid(spacing);
    let frac_y = y.rem_euclid(spacing);
    let tx = smoothstep_q16((frac_x * FIXED_ONE) / spacing);
    let ty = smoothstep_q16((frac_y * FIXED_ONE) / spacing);

    let v00 = i64::from(signed_lattice_value(seed, generator_version, cell_x, cell_y));
    let v10 = i64::from(signed_lattice_value(seed, generator_version, cell_x + 1, cell_y));
    let v01 = i64::from(signed_lattice_value(seed, generator_version, cell_x, cell_y + 1));
    let v11 = i64::from(signed_lattice_value(
        seed,
        generator_version,
        cell_x + 1,
        cell_y + 1,
    ));

    let south = lerp_q16(v00, v10, tx);
    let north = lerp_q16(v01, v11, tx);
    lerp_q16(south, north, ty).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn smoothstep_q16(t: i64) -> i64 {
    let t = t.clamp(0, FIXED_ONE);
    let t2 = (i128::from(t) * i128::from(t)) / i128::from(FIXED_ONE);
    let three_minus_two_t = i128::from(3 * FIXED_ONE - 2 * t);
    ((t2 * three_minus_two_t) / i128::from(FIXED_ONE)) as i64
}

fn lerp_q16(a: i64, b: i64, t: i64) -> i64 {
    a + (((b - a) * t) / FIXED_ONE)
}

fn signed_lattice_value(seed: u64, generator_version: u32, x: i64, y: i64) -> i32 {
    let mut state = seed ^ (u64::from(generator_version) << 17);
    state = mix64(state ^ x as u64);
    state = mix64(state ^ (y as u64).rotate_left(23));
    let magnitude = ((state >> 17) & 0xFFFF) as i32;
    magnitude - 32_768
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn settings() -> TerrainRecipeV1Settings {
        TerrainRecipeV1Settings {
            quadrant_size_mm: 64_000,
            generator_version: 1,
            world_seed: 0xA11C_EFA1_1A11_CE01,
        }
    }

    #[test]
    fn same_inputs_generate_identical_contract() {
        let coord = FrontierQuadrantCoord { x: 2, y: -3 };
        let first = generate_terrain_authority_v1(settings(), coord).unwrap();
        let second = generate_terrain_authority_v1(settings(), coord).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn adjacent_east_west_edges_are_exactly_identical() {
        let west = generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 0, y: 0 }).unwrap();
        let east = generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 1, y: 0 }).unwrap();
        let side = usize::from(TERRAIN_RECIPE_V1_CONTROL_GRID_SIDE);
        for y in 0..side {
            assert_eq!(
                west.elevation_samples_mm[y * side + side - 1],
                east.elevation_samples_mm[y * side]
            );
        }
    }

    #[test]
    fn adjacent_north_south_edges_are_exactly_identical() {
        let south = generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 0, y: 0 }).unwrap();
        let north = generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 0, y: 1 }).unwrap();
        let side = usize::from(TERRAIN_RECIPE_V1_CONTROL_GRID_SIDE);
        let south_edge = &south.elevation_samples_mm[(side - 1) * side..side * side];
        let north_edge = &north.elevation_samples_mm[..side];
        assert_eq!(south_edge, north_edge);
    }

    #[test]
    fn authoritative_height_changes_smoothly_inside_a_lattice_span() {
        let values: Vec<i32> = (0..8)
            .map(|x| authoritative_height_mm(settings().world_seed, settings().generator_version, x, 3))
            .collect();
        let distinct = values.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(
            distinct >= 6,
            "expected interpolated terrain samples, got {values:?}"
        );
    }

    #[test]
    fn flat_cell_is_walkable_and_buildable() {
        let flags = derive_cell_flags(100, 100, 100, 100, 64_000, 8);
        assert_eq!(flags, TERRAIN_CELL_WALKABLE | TERRAIN_CELL_BUILDABLE);
    }

    #[test]
    fn moderate_grade_is_walkable_but_not_buildable() {
        let flags = derive_cell_flags(0, 2_000, 0, 2_000, 64_000, 8);
        assert_eq!(flags, TERRAIN_CELL_WALKABLE);
    }

    #[test]
    fn steep_grade_is_blocked() {
        let flags = derive_cell_flags(0, 8_000, 0, 8_000, 64_000, 8);
        assert_eq!(flags, TERRAIN_CELL_BLOCKED);
    }

    #[test]
    fn generated_cell_semantics_never_contradict_contract_rules() {
        let contract =
            generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 0, y: 0 }).unwrap();
        for flags in contract.cell_flags {
            assert_ne!(flags & TERRAIN_CELL_WALKABLE, flags & TERRAIN_CELL_BLOCKED);
            if flags & TERRAIN_CELL_BUILDABLE != 0 {
                assert_ne!(flags & TERRAIN_CELL_WALKABLE, 0);
                assert_eq!(flags & TERRAIN_CELL_BLOCKED, 0);
            }
        }
    }

    #[test]
    fn extreme_quadrant_identity_fails_instead_of_aliasing_lattice_coordinates() {
        let error = generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: i64::MAX, y: 0 })
            .unwrap_err();
        assert!(matches!(
            error,
            TerrainRecipeError::LatticeCoordinateOverflow { .. }
        ));
    }

    #[test]
    fn quadrant_recipe_seed_changes_with_identity() {
        let a = derive_quadrant_recipe_seed(settings().world_seed, 1, FrontierQuadrantCoord { x: 0, y: 0 });
        let b = derive_quadrant_recipe_seed(settings().world_seed, 1, FrontierQuadrantCoord { x: 1, y: 0 });
        let c = derive_quadrant_recipe_seed(settings().world_seed, 2, FrontierQuadrantCoord { x: 0, y: 0 });
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn quadrant_biome_is_deterministic() {
        let coord = FrontierQuadrantCoord { x: 5, y: -12 };
        let biome_a = derive_quadrant_biome_id(0xA11C_EFA1_1A11_CE01, coord);
        let biome_b = derive_quadrant_biome_id(0xA11C_EFA1_1A11_CE01, coord);
        assert_eq!(biome_a, biome_b);
        assert!(matches!(biome_a, 1 | 2 | 3));
    }
}
