use aurenfall_contracts::{
    ENVIRONMENT_FAMILY_GROUND_COVER, ENVIRONMENT_FAMILY_ROCK, ENVIRONMENT_FAMILY_SHRUB,
    ENVIRONMENT_FAMILY_SMALL_PLANT, ENVIRONMENT_FAMILY_TREE, ENVIRONMENT_PRESENTATION_POLICY_VERSION,
    EnvironmentPresentationPolicyV1, TERRAIN_CELL_BLOCKED, TERRAIN_CELL_BUILDABLE,
    TERRAIN_CELL_CONNECTOR_CORRIDOR, TERRAIN_CELL_WALKABLE, TERRAIN_CELL_WATER, TerrainAuthorityContractV1,
};
use blake3::Hasher;
use thiserror::Error;

pub const LAB_ENVIRONMENT_POLICY_GENERATOR_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnvironmentPresentationGenerationError {
    #[error("terrain authority is invalid: {0}")]
    InvalidTerrainAuthority(String),
    #[error("environment policy lattice coordinate overflow for quadrant ({x},{y})")]
    LatticeCoordinateOverflow { x: i64, y: i64 },
    #[error("generated environment presentation policy failed validation: {0}")]
    InvalidGeneratedPolicy(String),
}

#[must_use]
pub fn derive_environment_policy_seed(world_seed: u64, terrain: &TerrainAuthorityContractV1) -> u64 {
    let mut hasher = Hasher::new();
    hasher.update(b"aurenfall/environment-policy-seed/v1");
    hasher.update(&world_seed.to_le_bytes());
    hasher.update(&terrain.generator_version.to_le_bytes());
    hasher.update(&LAB_ENVIRONMENT_POLICY_GENERATOR_VERSION.to_le_bytes());
    hasher.update(&terrain.recipe_seed.to_le_bytes());
    hasher.update(&terrain.quadrant_coord.x.to_le_bytes());
    hasher.update(&terrain.quadrant_coord.y.to_le_bytes());
    digest_prefix_u64(hasher.finalize())
}

pub fn generate_environment_presentation_policy_v1(
    world_seed: u64,
    terrain: &TerrainAuthorityContractV1,
) -> Result<EnvironmentPresentationPolicyV1, EnvironmentPresentationGenerationError> {
    terrain.validate().map_err(|error| {
        EnvironmentPresentationGenerationError::InvalidTerrainAuthority(error.to_string())
    })?;

    let cell_side = terrain.control_grid_side - 1;
    let cells_per_side = i64::from(cell_side);
    let base_x = terrain.quadrant_coord.x.checked_mul(cells_per_side).ok_or(
        EnvironmentPresentationGenerationError::LatticeCoordinateOverflow {
            x: terrain.quadrant_coord.x,
            y: terrain.quadrant_coord.y,
        },
    )?;
    let base_y = terrain.quadrant_coord.y.checked_mul(cells_per_side).ok_or(
        EnvironmentPresentationGenerationError::LatticeCoordinateOverflow {
            x: terrain.quadrant_coord.x,
            y: terrain.quadrant_coord.y,
        },
    )?;

    let mut cell_family_masks = Vec::with_capacity(terrain.cell_flags.len());
    for local_y in 0..cell_side {
        for local_x in 0..cell_side {
            let global_x = base_x.checked_add(i64::from(local_x)).ok_or(
                EnvironmentPresentationGenerationError::LatticeCoordinateOverflow {
                    x: terrain.quadrant_coord.x,
                    y: terrain.quadrant_coord.y,
                },
            )?;
            let global_y = base_y.checked_add(i64::from(local_y)).ok_or(
                EnvironmentPresentationGenerationError::LatticeCoordinateOverflow {
                    x: terrain.quadrant_coord.x,
                    y: terrain.quadrant_coord.y,
                },
            )?;
            let index = usize::from(local_y) * usize::from(cell_side) + usize::from(local_x);
            let morphology =
                derive_global_cell_morphology(world_seed, terrain.generator_version, global_x, global_y);
            cell_family_masks.push(derive_allowed_family_mask(terrain.cell_flags[index], morphology));
        }
    }

    let policy = EnvironmentPresentationPolicyV1 {
        version: ENVIRONMENT_PRESENTATION_POLICY_VERSION,
        quadrant_coord: terrain.quadrant_coord,
        terrain_generator_version: terrain.generator_version,
        policy_generator_version: LAB_ENVIRONMENT_POLICY_GENERATOR_VERSION,
        policy_seed: derive_environment_policy_seed(world_seed, terrain),
        control_grid_side: terrain.control_grid_side,
        cell_family_masks,
    };
    policy
        .validate()
        .map_err(|error| EnvironmentPresentationGenerationError::InvalidGeneratedPolicy(error.to_string()))?;
    Ok(policy)
}

fn derive_global_cell_morphology(
    world_seed: u64,
    terrain_generator_version: u32,
    global_x: i64,
    global_y: i64,
) -> u64 {
    let mut hasher = Hasher::new();
    hasher.update(b"aurenfall/environment-policy-cell/v1");
    hasher.update(&world_seed.to_le_bytes());
    hasher.update(&terrain_generator_version.to_le_bytes());
    hasher.update(&LAB_ENVIRONMENT_POLICY_GENERATOR_VERSION.to_le_bytes());
    hasher.update(&global_x.to_le_bytes());
    hasher.update(&global_y.to_le_bytes());
    digest_prefix_u64(hasher.finalize())
}

fn derive_allowed_family_mask(flags: u8, morphology: u64) -> u16 {
    if flags & TERRAIN_CELL_WATER != 0 {
        return 0;
    }

    if flags & TERRAIN_CELL_BLOCKED != 0 {
        return match morphology & 0b11 {
            0 => ENVIRONMENT_FAMILY_ROCK,
            1 => ENVIRONMENT_FAMILY_SHRUB | ENVIRONMENT_FAMILY_ROCK,
            2 => ENVIRONMENT_FAMILY_TREE | ENVIRONMENT_FAMILY_ROCK,
            _ => ENVIRONMENT_FAMILY_SHRUB | ENVIRONMENT_FAMILY_TREE | ENVIRONMENT_FAMILY_ROCK,
        };
    }

    if flags & TERRAIN_CELL_CONNECTOR_CORRIDOR != 0 {
        return ENVIRONMENT_FAMILY_GROUND_COVER;
    }

    if flags & TERRAIN_CELL_BUILDABLE != 0 {
        return match morphology & 0b11 {
            0 => 0,
            1 => ENVIRONMENT_FAMILY_GROUND_COVER,
            _ => ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_SMALL_PLANT,
        };
    }

    if flags & TERRAIN_CELL_WALKABLE != 0 {
        return match morphology & 0b111 {
            0 => 0,
            1 => ENVIRONMENT_FAMILY_GROUND_COVER,
            2 => ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_SMALL_PLANT,
            3 => ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_SHRUB,
            4 => ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_SMALL_PLANT | ENVIRONMENT_FAMILY_SHRUB,
            5 => ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_TREE,
            6 => ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_ROCK,
            _ => {
                ENVIRONMENT_FAMILY_GROUND_COVER
                    | ENVIRONMENT_FAMILY_SMALL_PLANT
                    | ENVIRONMENT_FAMILY_SHRUB
                    | ENVIRONMENT_FAMILY_TREE
                    | ENVIRONMENT_FAMILY_ROCK
            }
        };
    }

    0
}

fn digest_prefix_u64(digest: blake3::Hash) -> u64 {
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_contracts::{
        FrontierQuadrantCoord, TERRAIN_AUTHORITY_CONTRACT_VERSION, TERRAIN_CELL_BLOCKED,
        TERRAIN_CELL_BUILDABLE, TERRAIN_CELL_CONNECTOR_CORRIDOR, TERRAIN_CELL_WALKABLE,
        TERRAIN_CELL_WATER,
    };

    fn terrain_fixture() -> TerrainAuthorityContractV1 {
        TerrainAuthorityContractV1 {
            version: TERRAIN_AUTHORITY_CONTRACT_VERSION,
            quadrant_coord: FrontierQuadrantCoord::new(0, 0),
            generator_version: 1,
            quadrant_size_mm: 64_000,
            control_grid_side: 3,
            recipe_seed: 0xA11CE,
            biome_id: 1,
            elevation_samples_mm: vec![0; 9],
            cell_flags: vec![
                TERRAIN_CELL_WALKABLE | TERRAIN_CELL_BUILDABLE,
                TERRAIN_CELL_WALKABLE,
                TERRAIN_CELL_WATER | TERRAIN_CELL_BLOCKED,
                TERRAIN_CELL_WALKABLE | TERRAIN_CELL_CONNECTOR_CORRIDOR,
            ],
        }
    }

    #[test]
    fn same_inputs_generate_identical_policy() -> Result<(), EnvironmentPresentationGenerationError> {
        let terrain = terrain_fixture();
        let first = generate_environment_presentation_policy_v1(0xA11C_EFA1_1A11_CE01, &terrain)?;
        let second = generate_environment_presentation_policy_v1(0xA11C_EFA1_1A11_CE01, &terrain)?;
        assert_eq!(first, second);
        Ok(())
    }

    #[test]
    fn policy_preserves_terrain_lattice_identity() -> Result<(), EnvironmentPresentationGenerationError> {
        let terrain = terrain_fixture();
        let policy = generate_environment_presentation_policy_v1(7, &terrain)?;
        assert_eq!(policy.quadrant_coord, terrain.quadrant_coord);
        assert_eq!(policy.terrain_generator_version, terrain.generator_version);
        assert_eq!(policy.control_grid_side, terrain.control_grid_side);
        assert_eq!(policy.cell_family_masks.len(), terrain.cell_flags.len());
        assert_eq!(
            policy.policy_generator_version,
            LAB_ENVIRONMENT_POLICY_GENERATOR_VERSION
        );
        Ok(())
    }

    #[test]
    fn lab_semantic_rules_preserve_clear_water_and_corridors()
    -> Result<(), EnvironmentPresentationGenerationError> {
        let terrain = terrain_fixture();
        let policy = generate_environment_presentation_policy_v1(11, &terrain)?;
        assert_eq!(policy.cell_family_masks[2], 0);
        assert_eq!(policy.cell_family_masks[3], ENVIRONMENT_FAMILY_GROUND_COVER);
        Ok(())
    }

    #[test]
    fn buildable_cells_do_not_allow_tree_shrub_or_rock() -> Result<(), EnvironmentPresentationGenerationError>
    {
        let terrain = terrain_fixture();
        let policy = generate_environment_presentation_policy_v1(13, &terrain)?;
        let forbidden = ENVIRONMENT_FAMILY_SHRUB | ENVIRONMENT_FAMILY_TREE | ENVIRONMENT_FAMILY_ROCK;
        assert_eq!(policy.cell_family_masks[0] & forbidden, 0);
        Ok(())
    }

    #[test]
    fn blocked_cells_never_allow_ground_cover_or_small_plants() {
        let forbidden = ENVIRONMENT_FAMILY_GROUND_COVER | ENVIRONMENT_FAMILY_SMALL_PLANT;
        for morphology in 0..4 {
            let mask = derive_allowed_family_mask(TERRAIN_CELL_BLOCKED, morphology);
            assert_eq!(mask & forbidden, 0);
            assert_ne!(mask & ENVIRONMENT_FAMILY_ROCK, 0);
        }
    }

    #[test]
    fn different_world_seed_changes_server_owned_policy_seed() {
        let terrain = terrain_fixture();
        assert_ne!(
            derive_environment_policy_seed(1, &terrain),
            derive_environment_policy_seed(2, &terrain)
        );
    }

    #[test]
    fn invalid_terrain_is_rejected_before_policy_generation() {
        let mut terrain = terrain_fixture();
        terrain.cell_flags.pop();
        assert!(matches!(
            generate_environment_presentation_policy_v1(1, &terrain),
            Err(EnvironmentPresentationGenerationError::InvalidTerrainAuthority(_))
        ));
    }

    #[test]
    fn extreme_quadrant_identity_fails_instead_of_aliasing_global_cells() {
        let mut terrain = terrain_fixture();
        terrain.quadrant_coord = FrontierQuadrantCoord { x: i64::MAX, y: 0 };
        assert!(matches!(
            generate_environment_presentation_policy_v1(1, &terrain),
            Err(EnvironmentPresentationGenerationError::LatticeCoordinateOverflow { .. })
        ));
    }
}
