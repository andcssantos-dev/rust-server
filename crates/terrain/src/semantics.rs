use aurenfall_contracts::{
    TERRAIN_CELL_BLOCKED, TERRAIN_CELL_BUILDABLE, TERRAIN_CELL_CONNECTOR_CORRIDOR, TERRAIN_CELL_WALKABLE,
    TERRAIN_CELL_WATER, TerrainAuthorityContractV1,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerrainSemanticSummary {
    pub total_cells: usize,
    pub walkable_cells: usize,
    pub buildable_cells: usize,
    pub blocked_cells: usize,
    pub water_cells: usize,
    pub connector_corridor_cells: usize,
}

impl TerrainSemanticSummary {
    #[must_use]
    pub fn from_contract(contract: &TerrainAuthorityContractV1) -> Self {
        let mut summary = Self {
            total_cells: contract.cell_flags.len(),
            ..Self::default()
        };

        for flags in &contract.cell_flags {
            if flags & TERRAIN_CELL_WALKABLE != 0 {
                summary.walkable_cells += 1;
            }
            if flags & TERRAIN_CELL_BUILDABLE != 0 {
                summary.buildable_cells += 1;
            }
            if flags & TERRAIN_CELL_BLOCKED != 0 {
                summary.blocked_cells += 1;
            }
            if flags & TERRAIN_CELL_WATER != 0 {
                summary.water_cells += 1;
            }
            if flags & TERRAIN_CELL_CONNECTOR_CORRIDOR != 0 {
                summary.connector_corridor_cells += 1;
            }
        }

        summary
    }

    #[must_use]
    pub const fn classified_ground_cells(self) -> usize {
        self.walkable_cells + self.blocked_cells
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::{TerrainRecipeV1Settings, generate_terrain_authority_v1};
    use aurenfall_contracts::FrontierQuadrantCoord;

    fn settings() -> TerrainRecipeV1Settings {
        TerrainRecipeV1Settings {
            quadrant_size_mm: 64_000,
            generator_version: 1,
            world_seed: 0xA11C_EFA1_1A11_CE01,
        }
    }

    #[test]
    fn generated_recipe_has_complete_ground_semantic_coverage() {
        let contract =
            generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 0, y: 0 }).unwrap();
        let summary = TerrainSemanticSummary::from_contract(&contract);

        assert_eq!(summary.total_cells, 64);
        assert_eq!(summary.classified_ground_cells(), summary.total_cells);
        assert!(summary.buildable_cells <= summary.walkable_cells);
        assert_eq!(summary.water_cells, 0);
        assert_eq!(summary.connector_corridor_cells, 0);
    }

    #[test]
    fn semantic_summary_is_deterministic_for_same_contract() {
        let contract =
            generate_terrain_authority_v1(settings(), FrontierQuadrantCoord { x: 3, y: -2 }).unwrap();

        assert_eq!(
            TerrainSemanticSummary::from_contract(&contract),
            TerrainSemanticSummary::from_contract(&contract)
        );
    }
}
