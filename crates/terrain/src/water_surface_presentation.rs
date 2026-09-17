use crate::{
    HydrologyFieldV1Error, HydrologyFieldV1Settings, HydrologyKindV1, sample_hydrology_for_quadrant_local_v1,
};
use aurenfall_contracts::{
    TERRAIN_CELL_WATER, TerrainAuthorityContractV1, WATER_SURFACE_PRESENTATION_VERSION,
    WaterSurfacePresentationError, WaterSurfacePresentationV1, WaterSurfaceSampleV1,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WaterSurfacePresentationGenerationError {
    #[error("terrain authority is invalid before water presentation generation: {0}")]
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
    #[error("wet hydrology sample {index} of kind {kind:?} has no water surface")]
    MissingHydrologySurface { index: usize, kind: HydrologyKindV1 },
    #[error(
        "water sample {index} surface {water_surface_mm} mm is below terrain elevation {terrain_elevation_mm} mm"
    )]
    SurfaceBelowTerrain {
        index: usize,
        water_surface_mm: i32,
        terrain_elevation_mm: i32,
    },
    #[error(
        "terrain WATER semantic mismatch at cell {index}: expected_water={expected_water} actual_water={actual_water}"
    )]
    TerrainWaterSemanticMismatch {
        index: usize,
        expected_water: bool,
        actual_water: bool,
    },
    #[error("generated water surface presentation is invalid: {0}")]
    InvalidPresentation(String),
}

pub fn generate_water_surface_presentation_v1(
    settings: HydrologyFieldV1Settings,
    terrain: &TerrainAuthorityContractV1,
) -> Result<WaterSurfacePresentationV1, WaterSurfacePresentationGenerationError> {
    terrain.validate().map_err(|error| {
        WaterSurfacePresentationGenerationError::InvalidTerrainAuthority(error.to_string())
    })?;
    let settings = settings.validate()?;
    if terrain.generator_version != settings.generator_version {
        return Err(
            WaterSurfacePresentationGenerationError::GeneratorVersionMismatch {
                terrain_generator_version: terrain.generator_version,
                hydrology_generator_version: settings.generator_version,
            },
        );
    }

    let side = usize::from(terrain.control_grid_side);
    let samples_per_quadrant = i64::from(terrain.control_grid_side - 1);
    let mut samples = Vec::with_capacity(side * side);

    for local_y in 0..side {
        for local_x in 0..side {
            let index = local_y * side + local_x;
            let hydrology = sample_hydrology_for_quadrant_local_v1(
                settings,
                terrain.quadrant_coord,
                local_x as i64,
                local_y as i64,
                samples_per_quadrant,
            )?;

            let sample = match hydrology.kind {
                HydrologyKindV1::Dry => WaterSurfaceSampleV1::dry(),
                HydrologyKindV1::Lake => {
                    let water_surface_mm = hydrology.water_surface_mm.ok_or(
                        WaterSurfacePresentationGenerationError::MissingHydrologySurface {
                            index,
                            kind: HydrologyKindV1::Lake,
                        },
                    )?;
                    WaterSurfaceSampleV1::lake(water_surface_mm)
                }
                HydrologyKindV1::River => {
                    let water_surface_mm = hydrology.water_surface_mm.ok_or(
                        WaterSurfacePresentationGenerationError::MissingHydrologySurface {
                            index,
                            kind: HydrologyKindV1::River,
                        },
                    )?;
                    WaterSurfaceSampleV1::river(
                        water_surface_mm,
                        hydrology.flow_hint_x,
                        hydrology.flow_hint_y,
                    )
                }
            };

            if sample.is_wet() {
                let terrain_elevation_mm = terrain.elevation_samples_mm[index];
                if sample.water_surface_mm < terrain_elevation_mm {
                    return Err(WaterSurfacePresentationGenerationError::SurfaceBelowTerrain {
                        index,
                        water_surface_mm: sample.water_surface_mm,
                        terrain_elevation_mm,
                    });
                }
            }

            samples.push(sample);
        }
    }

    let cell_side = side - 1;
    for cell_y in 0..cell_side {
        for cell_x in 0..cell_side {
            let i00 = cell_y * side + cell_x;
            let i10 = i00 + 1;
            let i01 = i00 + side;
            let i11 = i01 + 1;
            let expected_water = samples[i00].is_wet()
                || samples[i10].is_wet()
                || samples[i01].is_wet()
                || samples[i11].is_wet();
            let index = cell_y * cell_side + cell_x;
            let actual_water = terrain.cell_flags[index] & TERRAIN_CELL_WATER != 0;
            if expected_water != actual_water {
                return Err(
                    WaterSurfacePresentationGenerationError::TerrainWaterSemanticMismatch {
                        index,
                        expected_water,
                        actual_water,
                    },
                );
            }
        }
    }

    let presentation = WaterSurfacePresentationV1 {
        version: WATER_SURFACE_PRESENTATION_VERSION,
        quadrant_coord: terrain.quadrant_coord,
        terrain_generator_version: terrain.generator_version,
        hydrology_generator_version: settings.generator_version,
        control_grid_side: terrain.control_grid_side,
        samples,
    };
    presentation
        .validate()
        .map_err(|error: WaterSurfacePresentationError| {
            WaterSurfacePresentationGenerationError::InvalidPresentation(error.to_string())
        })?;
    Ok(presentation)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use crate::{TerrainRecipeV1Settings, apply_hydrology_to_terrain_v1, generate_terrain_authority_v1};
    use aurenfall_contracts::{FrontierQuadrantCoord, WaterSurfaceKindV1};
    fn hydrology_settings() -> HydrologyFieldV1Settings {
        HydrologyFieldV1Settings {
            world_seed: 0xA11C_EFA1_1A11_CE01,
            generator_version: 1,
        }
    }

    fn integrated_terrain(coord: FrontierQuadrantCoord) -> TerrainAuthorityContractV1 {
        let settings = hydrology_settings();
        let base = generate_terrain_authority_v1(
            TerrainRecipeV1Settings {
                quadrant_size_mm: 64_000,
                generator_version: settings.generator_version,
                world_seed: settings.world_seed,
            },
            coord,
        )
        .unwrap();
        apply_hydrology_to_terrain_v1(settings, &base).unwrap()
    }

    #[test]
    fn same_inputs_generate_identical_water_surface_presentation() {
        let terrain = integrated_terrain(FrontierQuadrantCoord::new(4, -16));
        let first = generate_water_surface_presentation_v1(hydrology_settings(), &terrain).unwrap();
        let replay = generate_water_surface_presentation_v1(hydrology_settings(), &terrain).unwrap();
        assert_eq!(first, replay);
    }

    #[test]
    fn known_lake_fixture_preserves_surface_kind_and_height() {
        let terrain = integrated_terrain(FrontierQuadrantCoord::new(4, -16));
        let presentation = generate_water_surface_presentation_v1(hydrology_settings(), &terrain).unwrap();
        let sample = presentation.samples[1];
        assert_eq!(sample.kind, WaterSurfaceKindV1::Lake);
        assert_eq!(sample.water_surface_mm, -1_047);
        assert_eq!((sample.flow_hint_x, sample.flow_hint_y), (0, 0));
    }

    #[test]
    fn known_river_fixture_preserves_surface_kind_height_and_flow() {
        let terrain = integrated_terrain(FrontierQuadrantCoord::new(-6, -16));
        let presentation = generate_water_surface_presentation_v1(hydrology_settings(), &terrain).unwrap();
        let sample = presentation.samples[2];
        assert_eq!(sample.kind, WaterSurfaceKindV1::River);
        assert_eq!(sample.water_surface_mm, 1_865);
        assert_eq!((sample.flow_hint_x, sample.flow_hint_y), (0, 1));
    }

    #[test]
    fn adjacent_quadrants_share_exact_water_surface_edge_samples() {
        let west = integrated_terrain(FrontierQuadrantCoord::new(4, -16));
        let east = integrated_terrain(FrontierQuadrantCoord::new(5, -16));
        let west_presentation = generate_water_surface_presentation_v1(hydrology_settings(), &west).unwrap();
        let east_presentation = generate_water_surface_presentation_v1(hydrology_settings(), &east).unwrap();
        let side = usize::from(west.control_grid_side);
        for y in 0..side {
            assert_eq!(
                west_presentation.samples[y * side + side - 1],
                east_presentation.samples[y * side]
            );
        }
    }

    #[test]
    fn water_surface_samples_match_integrated_terrain_water_semantics() {
        let terrain = integrated_terrain(FrontierQuadrantCoord::new(-6, -16));
        let presentation = generate_water_surface_presentation_v1(hydrology_settings(), &terrain).unwrap();
        let side = usize::from(terrain.control_grid_side);
        let cell_side = side - 1;
        for cell_y in 0..cell_side {
            for cell_x in 0..cell_side {
                let i00 = cell_y * side + cell_x;
                let i10 = i00 + 1;
                let i01 = i00 + side;
                let i11 = i01 + 1;
                let expected_water = presentation.samples[i00].is_wet()
                    || presentation.samples[i10].is_wet()
                    || presentation.samples[i01].is_wet()
                    || presentation.samples[i11].is_wet();
                let actual_water = terrain.cell_flags[cell_y * cell_side + cell_x] & TERRAIN_CELL_WATER != 0;
                assert_eq!(expected_water, actual_water);
            }
        }
    }

    #[test]
    fn dry_unintegrated_terrain_is_rejected_when_hydrology_requires_water() {
        let settings = hydrology_settings();
        let base = generate_terrain_authority_v1(
            TerrainRecipeV1Settings {
                quadrant_size_mm: 64_000,
                generator_version: settings.generator_version,
                world_seed: settings.world_seed,
            },
            FrontierQuadrantCoord::new(4, -16),
        )
        .unwrap();
        assert!(matches!(
            generate_water_surface_presentation_v1(settings, &base),
            Err(WaterSurfacePresentationGenerationError::SurfaceBelowTerrain { .. })
                | Err(WaterSurfacePresentationGenerationError::TerrainWaterSemanticMismatch { .. })
        ));
    }

    #[test]
    fn generator_version_mismatch_is_rejected() {
        let terrain = integrated_terrain(FrontierQuadrantCoord::new(4, -16));
        let mut settings = hydrology_settings();
        settings.generator_version = 2;
        assert!(matches!(
            generate_water_surface_presentation_v1(settings, &terrain),
            Err(WaterSurfacePresentationGenerationError::GeneratorVersionMismatch { .. })
        ));
    }
}
