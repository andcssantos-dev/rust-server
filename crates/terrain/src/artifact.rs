#[path = "environment_presentation.rs"]
mod environment_presentation;

use aurenfall_contracts::{EnvironmentPresentationPolicyV1, TerrainAuthorityContractV1};

use self::environment_presentation::generate_environment_presentation_policy_v1;

const TERRAIN_AUTHORITY_ARTIFACT_DOMAIN_V1: &[u8] = b"AURENFALL_TERRAIN_AUTHORITY_ARTIFACT_V1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainAuthorityArtifactV1 {
    contract: TerrainAuthorityContractV1,
    canonical_bytes: Vec<u8>,
    fingerprint: [u8; 32],
}

impl TerrainAuthorityArtifactV1 {
    pub fn from_contract(contract: TerrainAuthorityContractV1) -> Self {
        let canonical_bytes = encode_canonical_contract(&contract);
        let fingerprint = *blake3::hash(&canonical_bytes).as_bytes();
        Self {
            contract,
            canonical_bytes,
            fingerprint,
        }
    }

    #[must_use]
    pub const fn contract(&self) -> &TerrainAuthorityContractV1 {
        &self.contract
    }

    pub fn environment_presentation_policy_v1(
        &self,
        world_seed: u64,
    ) -> Result<EnvironmentPresentationPolicyV1, String> {
        generate_environment_presentation_policy_v1(world_seed, &self.contract)
            .map_err(|error| error.to_string())
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    #[must_use]
    pub const fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    #[must_use]
    pub fn byte_len(&self) -> u64 {
        self.canonical_bytes.len() as u64
    }
}

fn encode_canonical_contract(contract: &TerrainAuthorityContractV1) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        TERRAIN_AUTHORITY_ARTIFACT_DOMAIN_V1.len()
            + 2
            + 8
            + 8
            + 8
            + 4
            + 8
            + 2
            + 4
            + contract.elevation_samples_mm.len() * 4
            + 4
            + contract.cell_flags.len(),
    );

    bytes.extend_from_slice(TERRAIN_AUTHORITY_ARTIFACT_DOMAIN_V1);
    bytes.extend_from_slice(&contract.version.to_le_bytes());
    bytes.extend_from_slice(&contract.quadrant_coord.x.to_le_bytes());
    bytes.extend_from_slice(&contract.quadrant_coord.y.to_le_bytes());
    bytes.extend_from_slice(&contract.quadrant_size_mm.to_le_bytes());
    bytes.extend_from_slice(&contract.generator_version.to_le_bytes());
    bytes.extend_from_slice(&contract.recipe_seed.to_le_bytes());
    bytes.extend_from_slice(&contract.control_grid_side.to_le_bytes());
    bytes.extend_from_slice(&(contract.elevation_samples_mm.len() as u32).to_le_bytes());
    for elevation in &contract.elevation_samples_mm {
        bytes.extend_from_slice(&elevation.to_le_bytes());
    }
    bytes.extend_from_slice(&(contract.cell_flags.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&contract.cell_flags);
    bytes
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use aurenfall_contracts::FrontierQuadrantCoord;

    use crate::{TerrainRecipeV1Settings, generate_terrain_authority_v1};

    use super::*;

    fn settings() -> TerrainRecipeV1Settings {
        TerrainRecipeV1Settings {
            quadrant_size_mm: 64_000,
            generator_version: 1,
            world_seed: 0xA11C_EFA1_1A11_CE55,
        }
    }

    #[test]
    fn canonical_artifact_is_bit_stable_for_same_contract() {
        let contract = generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 3, y: -2 })
            .expect("terrain contract should generate");
        let first = TerrainAuthorityArtifactV1::from_contract(contract.clone());
        let second = TerrainAuthorityArtifactV1::from_contract(contract);

        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(first.canonical_bytes(), second.canonical_bytes());
        assert_eq!(first.byte_len(), first.canonical_bytes().len() as u64);
    }

    #[test]
    fn canonical_artifact_changes_with_quadrant_identity() {
        let first = TerrainAuthorityArtifactV1::from_contract(
            generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 0, y: 0 })
                .expect("terrain contract should generate"),
        );
        let second = TerrainAuthorityArtifactV1::from_contract(
            generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 1, y: 0 })
                .expect("terrain contract should generate"),
        );

        assert_ne!(first.fingerprint(), second.fingerprint());
        assert_ne!(first.canonical_bytes(), second.canonical_bytes());
    }

    #[test]
    fn retained_artifact_generates_matching_environment_policy() {
        let artifact = TerrainAuthorityArtifactV1::from_contract(
            generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 2, y: 1 })
                .expect("terrain contract should generate"),
        );
        let policy = artifact
            .environment_presentation_policy_v1(settings().world_seed)
            .expect("environment presentation policy should generate");

        assert_eq!(policy.quadrant_coord, artifact.contract().quadrant_coord);
        assert_eq!(
            policy.terrain_generator_version,
            artifact.contract().generator_version
        );
        assert_eq!(policy.control_grid_side, artifact.contract().control_grid_side);
        assert_eq!(
            policy.cell_family_masks.len(),
            artifact.contract().cell_flags.len()
        );
    }
}
