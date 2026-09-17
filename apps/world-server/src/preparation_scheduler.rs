use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use aurenfall_contracts::{
    EnvironmentPresentationPolicyV1, FrontierQuadrantCoord, TerrainAuthorityContractV1,
    WaterSurfacePresentationV1,
};
use aurenfall_core::{QuadrantCoord, UniverseSeed};
use aurenfall_domain::{
    PreparationFingerprint, PreparedQuadrantArtifact, QuadrantPreparationAbandonOutcome,
    QuadrantPreparationCompletionOutcome, QuadrantPreparationIdentity, QuadrantPreparationRequestOutcome,
};
use aurenfall_preparation::{
    PreparationExecutionError, PreparationSubmissionOutcome, PreparationWorkerEvent,
    PreparationWorkerIngress, PreparationWorkerResults, PreparationWorkerSettings, PreparationWorkerTask,
    QuadrantPreparationExecutor, spawn_preparation_worker,
};
use aurenfall_terrain::{
    HydrologyFieldV1Settings, TerrainAuthorityArtifactV1, TerrainRecipeV1Settings,
    apply_hydrology_to_terrain_v1, generate_terrain_authority_v1, generate_water_surface_presentation_v1,
};
use tracing::{info, warn};

use crate::frontier_runtime::FrontierRuntime;

type PreparedTerrainArtifacts = Arc<Mutex<HashMap<QuadrantPreparationIdentity, TerrainAuthorityArtifactV1>>>;

#[derive(Debug, Clone, Copy)]
pub struct PreparationSchedulerSettings {
    evaluation_interval: Duration,
    max_submissions_per_tick: usize,
    max_prepared_quadrants: usize,
    retry_backoff: Duration,
    max_retry_attempts: u32,
}

impl PreparationSchedulerSettings {
    pub fn new(
        evaluation_interval: Duration,
        max_submissions_per_tick: usize,
        max_prepared_quadrants: usize,
        retry_backoff: Duration,
        max_retry_attempts: u32,
    ) -> anyhow::Result<Self> {
        if evaluation_interval.is_zero() {
            bail!("preparation scheduler evaluation interval must be greater than zero");
        }
        if max_submissions_per_tick == 0 {
            bail!("preparation scheduler max submissions per tick must be greater than zero");
        }
        if max_prepared_quadrants == 0 {
            bail!("preparation scheduler prepared budget must be greater than zero");
        }
        if retry_backoff.is_zero() {
            bail!("preparation scheduler retry backoff must be greater than zero");
        }
        if max_retry_attempts == 0 {
            bail!("preparation scheduler max retry attempts must be greater than zero");
        }
        Ok(Self {
            evaluation_interval,
            max_submissions_per_tick,
            max_prepared_quadrants,
            retry_backoff,
            max_retry_attempts,
        })
    }

    #[must_use]
    pub const fn evaluation_interval(self) -> Duration {
        self.evaluation_interval
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PreparationSchedulerReport {
    pub completed: usize,
    pub failed: usize,
    pub submitted: usize,
    pub backpressure: usize,
    pub exhausted: usize,
    pub prepared_private: usize,
    pub in_flight: usize,
}

impl PreparationSchedulerReport {
    #[must_use]
    pub const fn has_activity(self) -> bool {
        self.completed > 0
            || self.failed > 0
            || self.submitted > 0
            || self.backpressure > 0
            || self.exhausted > 0
    }
}

#[derive(Debug, Clone, Copy)]
struct RetryState {
    failures: u32,
    retry_after: Instant,
    exhausted: bool,
}

#[derive(Debug)]
struct TerrainPreparationExecutor {
    quadrant_size_mm: i64,
    world_seed: u64,
    available_biome_ids: Vec<u8>,
    prepared_terrain_artifacts: PreparedTerrainArtifacts,
}

impl QuadrantPreparationExecutor for TerrainPreparationExecutor {
    fn execute(
        &self,
        identity: QuadrantPreparationIdentity,
    ) -> Result<PreparedQuadrantArtifact, PreparationExecutionError> {
        let coord = identity.coord();
        let contract = generate_hydrology_integrated_terrain_authority_v1(
            self.quadrant_size_mm,
            identity.generator_version(),
            self.world_seed,
            FrontierQuadrantCoord {
                x: coord.x(),
                y: coord.y(),
            },
            &self.available_biome_ids,
        )
        .map_err(|error| PreparationExecutionError::new(error.to_string()))?;
        let terrain_artifact = TerrainAuthorityArtifactV1::from_contract(contract);
        let artifact_fingerprint = PreparationFingerprint::new(terrain_artifact.fingerprint())
            .map_err(|error| PreparationExecutionError::new(error.to_string()))?;
        let prepared_artifact =
            PreparedQuadrantArtifact::new(identity, artifact_fingerprint, terrain_artifact.byte_len())
                .map_err(|error| PreparationExecutionError::new(error.to_string()))?;

        let mut retained = self
            .prepared_terrain_artifacts
            .lock()
            .map_err(|_| PreparationExecutionError::new("prepared terrain artifact store mutex poisoned"))?;
        retained.insert(identity, terrain_artifact);
        Ok(prepared_artifact)
    }
}

pub(crate) fn terrain_world_seed_v1(universe_seed: &UniverseSeed) -> u64 {
    let root = universe_seed.quadrant_seed(QuadrantCoord::new(0, 0), 1);
    let bytes = root.as_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

pub(crate) fn generate_hydrology_integrated_terrain_authority_v1(
    quadrant_size_mm: i64,
    generator_version: u32,
    world_seed: u64,
    coord: FrontierQuadrantCoord,
    available_biome_ids: &[u8],
) -> anyhow::Result<TerrainAuthorityContractV1> {
    let mut terrain = generate_terrain_authority_v1(
        TerrainRecipeV1Settings {
            quadrant_size_mm,
            generator_version,
            world_seed,
        },
        coord,
    )
    .context("failed to generate deterministic base terrain authority")?;

    // Aplica o sorteio determinístico do pool carregado do YAML
    if !available_biome_ids.is_empty() {
        terrain.biome_id = aurenfall_terrain::derive_quadrant_biome_from_pool(
            world_seed,
            coord,
            available_biome_ids,
        );
    }

    apply_hydrology_to_terrain_v1(
        HydrologyFieldV1Settings {
            world_seed,
            generator_version,
        },
        &terrain,
    )
    .map_err(anyhow::Error::msg)
    .context("failed to integrate deterministic hydrology into terrain authority")
}

#[derive(Debug)]
pub struct PreparationScheduler {
    settings: PreparationSchedulerSettings,
    universe_seed: UniverseSeed,
    quadrant_size_mm: i64,
    generator_version: u32,
    ingress: PreparationWorkerIngress,
    results: PreparationWorkerResults,
    _worker_task: PreparationWorkerTask,
    retries: HashMap<QuadrantCoord, RetryState>,
    prepared_terrain_artifacts: PreparedTerrainArtifacts,
}

impl PreparationScheduler {
    pub fn spawn(
        worker_settings: PreparationWorkerSettings,
        settings: PreparationSchedulerSettings,
        universe_seed: UniverseSeed,
        quadrant_size_mm: i64,
        generator_version: u32,
        available_biome_ids: Vec<u8>,
    ) -> Self {
        let terrain_world_seed = terrain_world_seed_v1(&universe_seed);
        let prepared_terrain_artifacts = Arc::new(Mutex::new(HashMap::with_capacity(
            settings.max_prepared_quadrants,
        )));
        let executor = TerrainPreparationExecutor {
            quadrant_size_mm,
            world_seed: terrain_world_seed,
            available_biome_ids,
            prepared_terrain_artifacts: Arc::clone(&prepared_terrain_artifacts),
        };
        let (ingress, results, worker_task) = spawn_preparation_worker(worker_settings, Arc::new(executor));
        Self {
            settings,
            universe_seed,
            quadrant_size_mm,
            generator_version,
            ingress,
            results,
            _worker_task: worker_task,
            retries: HashMap::new(),
            prepared_terrain_artifacts,
        }
    }

    #[must_use]
    pub const fn evaluation_interval(&self) -> Duration {
        self.settings.evaluation_interval()
    }

    #[cfg(test)]
    pub fn prepared_terrain_artifact(
        &self,
        identity: QuadrantPreparationIdentity,
    ) -> anyhow::Result<Option<TerrainAuthorityArtifactV1>> {
        let retained = self
            .prepared_terrain_artifacts
            .lock()
            .map_err(|_| anyhow::anyhow!("prepared terrain artifact store mutex poisoned"))?;
        Ok(retained.get(&identity).cloned())
    }

    pub fn terrain_authority_for_revealed(
        &self,
        coord: FrontierQuadrantCoord,
    ) -> anyhow::Result<TerrainAuthorityContractV1> {
        {
            let retained = self
                .prepared_terrain_artifacts
                .lock()
                .map_err(|_| anyhow::anyhow!("prepared terrain artifact store mutex poisoned"))?;
            if let Some(artifact) = retained.iter().find_map(|(identity, artifact)| {
                let identity_coord = identity.coord();
                (identity.generator_version() == self.generator_version
                    && identity_coord.x() == coord.x
                    && identity_coord.y() == coord.y)
                    .then_some(artifact)
            }) {
                return Ok(artifact.contract().clone());
            }
        }

        generate_hydrology_integrated_terrain_authority_v1(
            self.quadrant_size_mm,
            self.generator_version,
            terrain_world_seed_v1(&self.universe_seed),
            coord,
            &[],
        )
        .context(
            "failed to resolve deterministic hydrology-integrated terrain authority for revealed quadrant",
        )
    }

    pub fn presentation_contracts_for_revealed(
        &self,
        coord: FrontierQuadrantCoord,
    ) -> anyhow::Result<(TerrainAuthorityContractV1, EnvironmentPresentationPolicyV1)> {
        let terrain = self.terrain_authority_for_revealed(coord)?;
        let policy = TerrainAuthorityArtifactV1::from_contract(terrain.clone())
            .environment_presentation_policy_v1(terrain_world_seed_v1(&self.universe_seed))
            .map_err(anyhow::Error::msg)
            .context("failed to resolve deterministic EnvironmentPresentationPolicy for revealed quadrant")?;

        if policy.quadrant_coord != terrain.quadrant_coord
            || policy.terrain_generator_version != terrain.generator_version
            || policy.control_grid_side != terrain.control_grid_side
            || policy.cell_family_masks.len() != terrain.cell_flags.len()
        {
            bail!("revealed terrain and environment presentation policy lattice identity diverged");
        }

        Ok((terrain, policy))
    }
    pub fn presentation_contracts_with_water_for_revealed(
        &self,
        coord: FrontierQuadrantCoord,
    ) -> anyhow::Result<(
        TerrainAuthorityContractV1,
        EnvironmentPresentationPolicyV1,
        WaterSurfacePresentationV1,
    )> {
        let (terrain, policy) = self.presentation_contracts_for_revealed(coord)?;

        let water_surface = generate_water_surface_presentation_v1(
            HydrologyFieldV1Settings {
                world_seed: terrain_world_seed_v1(&self.universe_seed),
                generator_version: self.generator_version,
            },
            &terrain,
        )
        .map_err(anyhow::Error::msg)
        .context("failed to resolve deterministic WaterSurfacePresentation for revealed quadrant")?;

        if water_surface.quadrant_coord != terrain.quadrant_coord
            || water_surface.terrain_generator_version != terrain.generator_version
            || water_surface.hydrology_generator_version != terrain.generator_version
            || water_surface.control_grid_side != terrain.control_grid_side
            || water_surface.samples.len() != terrain.elevation_samples_mm.len()
        {
            bail!("revealed terrain and water surface presentation lattice identity diverged");
        }

        if policy.quadrant_coord != water_surface.quadrant_coord
            || policy.terrain_generator_version != water_surface.terrain_generator_version
            || policy.control_grid_side != water_surface.control_grid_side
        {
            bail!("revealed environment policy and water surface presentation identity diverged");
        }

        Ok((terrain, policy, water_surface))
    }
    pub fn evaluate(
        &mut self,
        frontier: &mut FrontierRuntime,
        now: Instant,
    ) -> anyhow::Result<PreparationSchedulerReport> {
        let mut report = PreparationSchedulerReport::default();
        self.consume_results(frontier, now, &mut report)?;
        self.schedule_candidates(frontier, now, &mut report)?;
        report.prepared_private = frontier.prepared_len();
        report.in_flight = frontier.preparation_in_flight_len();
        Ok(report)
    }

    fn consume_results(
        &mut self,
        frontier: &mut FrontierRuntime,
        now: Instant,
        report: &mut PreparationSchedulerReport,
    ) -> anyhow::Result<()> {
        loop {
            let event = match self.results.try_recv() {
                Ok(event) => event,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    bail!("preparation worker result queue disconnected");
                }
            };

            let revision_before = frontier.revision();
            match event {
                PreparationWorkerEvent::Completed(artifact) => {
                    let identity = artifact.identity();
                    let coord = identity.coord();
                    self.verify_retained_terrain_artifact(artifact)?;
                    let completion_outcome = match frontier.complete_preparation(artifact)? {
                        QuadrantPreparationCompletionOutcome::Prepared { .. } => "prepared",
                        QuadrantPreparationCompletionOutcome::AlreadyPrepared => "already_prepared",
                    };
                    self.retries.remove(&coord);
                    report.completed += 1;

                    if frontier.revision() != revision_before {
                        bail!("quadrant preparation mutated public frontier revision");
                    }

                    info!(
                        quadrant_x = coord.x(),
                        quadrant_y = coord.y(),
                        generator_version = identity.generator_version(),
                        generation_inputs_fingerprint = ?identity.generation_inputs_fingerprint(),
                        artifact_fingerprint = ?artifact.artifact_fingerprint(),
                        artifact_bytes = artifact.artifact_bytes(),
                        artifact_kind = "terrain_authority_v1",
                        retained_private = true,
                        outcome = completion_outcome,
                        public_frontier_revision = frontier.revision(),
                        public_revealed_quadrants = frontier.revealed_len(),
                        "authoritative private quadrant preparation result committed"
                    );
                }
                PreparationWorkerEvent::Failed { identity, error } => {
                    let coord = identity.coord();
                    self.remove_retained_terrain_artifact(identity)?;
                    match frontier.fail_preparation(identity)? {
                        QuadrantPreparationAbandonOutcome::Removed => {}
                        QuadrantPreparationAbandonOutcome::NotFound => {
                            bail!(
                                "failed preparation result had no authoritative reservation at ({}, {})",
                                coord.x(),
                                coord.y()
                            );
                        }
                        QuadrantPreparationAbandonOutcome::AlreadyPrepared => {
                            bail!(
                                "failed preparation result conflicted with prepared state at ({}, {})",
                                coord.x(),
                                coord.y()
                            );
                        }
                    }

                    let retry = self.retries.entry(coord).or_insert(RetryState {
                        failures: 0,
                        retry_after: now,
                        exhausted: false,
                    });
                    retry.failures = retry.failures.saturating_add(1);
                    let retry_outcome = if retry.failures > self.settings.max_retry_attempts {
                        retry.exhausted = true;
                        report.exhausted += 1;
                        "exhausted"
                    } else {
                        retry.retry_after = now + self.settings.retry_backoff;
                        "scheduled"
                    };
                    let retry_failures = retry.failures;
                    report.failed += 1;

                    if frontier.revision() != revision_before {
                        bail!("quadrant preparation failure mutated public frontier revision");
                    }

                    warn!(
                        quadrant_x = coord.x(),
                        quadrant_y = coord.y(),
                        generator_version = identity.generator_version(),
                        generation_inputs_fingerprint = ?identity.generation_inputs_fingerprint(),
                        worker_error = %error,
                        retry_failures,
                        retry_outcome,
                        public_frontier_revision = frontier.revision(),
                        "authoritative private quadrant preparation worker failure observed"
                    );
                }
            }
        }
        Ok(())
    }

    fn verify_retained_terrain_artifact(&self, artifact: PreparedQuadrantArtifact) -> anyhow::Result<()> {
        let identity = artifact.identity();
        let retained = self
            .prepared_terrain_artifacts
            .lock()
            .map_err(|_| anyhow::anyhow!("prepared terrain artifact store mutex poisoned"))?;
        let terrain_artifact = retained
            .get(&identity)
            .context("completed preparation did not retain its terrain authority artifact")?;
        let retained_fingerprint = PreparationFingerprint::new(terrain_artifact.fingerprint())
            .context("retained terrain fingerprint cannot be represented as preparation fingerprint")?;
        if retained_fingerprint != artifact.artifact_fingerprint() {
            bail!("retained terrain artifact fingerprint does not match prepared metadata");
        }
        if terrain_artifact.byte_len() != artifact.artifact_bytes() {
            bail!("retained terrain artifact byte length does not match prepared metadata");
        }
        Ok(())
    }

    fn remove_retained_terrain_artifact(&self, identity: QuadrantPreparationIdentity) -> anyhow::Result<()> {
        let mut retained = self
            .prepared_terrain_artifacts
            .lock()
            .map_err(|_| anyhow::anyhow!("prepared terrain artifact store mutex poisoned"))?;
        retained.remove(&identity);
        Ok(())
    }

    fn schedule_candidates(
        &mut self,
        frontier: &mut FrontierRuntime,
        now: Instant,
        report: &mut PreparationSchedulerReport,
    ) -> anyhow::Result<()> {
        let occupied = frontier
            .prepared_len()
            .saturating_add(frontier.preparation_in_flight_len());
        if occupied >= self.settings.max_prepared_quadrants {
            return Ok(());
        }

        let available_budget = self.settings.max_prepared_quadrants - occupied;
        let admission_budget = available_budget.min(self.settings.max_submissions_per_tick);
        if admission_budget == 0 {
            return Ok(());
        }

        let mut admitted = 0usize;
        for coord in frontier.candidate_quadrants() {
            if admitted >= admission_budget {
                break;
            }
            if !self.retry_allows(coord, now) {
                continue;
            }

            let seed = self.universe_seed.quadrant_seed(coord, self.generator_version);
            let fingerprint = PreparationFingerprint::new(*seed.as_bytes())
                .context("deterministic quadrant seed cannot be used as preparation fingerprint")?;
            let identity = frontier.preparation_identity(coord, fingerprint)?;

            match frontier.request_preparation(identity)? {
                QuadrantPreparationRequestOutcome::Started => {}
                QuadrantPreparationRequestOutcome::AlreadyInFlight
                | QuadrantPreparationRequestOutcome::AlreadyPrepared => continue,
            }

            match self.ingress.try_submit(identity) {
                PreparationSubmissionOutcome::Submitted => {
                    admitted += 1;
                    report.submitted += 1;
                    info!(
                        quadrant_x = coord.x(),
                        quadrant_y = coord.y(),
                        generator_version = identity.generator_version(),
                        generation_inputs_fingerprint = ?identity.generation_inputs_fingerprint(),
                        outcome = "submitted",
                        remaining_queue_capacity = self.ingress.remaining_queue_capacity(),
                        authoritative_in_flight = frontier.preparation_in_flight_len(),
                        "authoritative private quadrant preparation admission observed"
                    );
                }
                PreparationSubmissionOutcome::Duplicate => {
                    admitted += 1;
                    info!(
                        quadrant_x = coord.x(),
                        quadrant_y = coord.y(),
                        generator_version = identity.generator_version(),
                        generation_inputs_fingerprint = ?identity.generation_inputs_fingerprint(),
                        outcome = "duplicate",
                        remaining_queue_capacity = self.ingress.remaining_queue_capacity(),
                        authoritative_in_flight = frontier.preparation_in_flight_len(),
                        "authoritative private quadrant preparation admission observed"
                    );
                }
                PreparationSubmissionOutcome::Full => {
                    let abandoned = frontier.cancel_preparation(identity)?;
                    if abandoned != QuadrantPreparationAbandonOutcome::Removed {
                        bail!(
                            "preparation queue backpressure could not roll back reservation at ({}, {})",
                            coord.x(),
                            coord.y()
                        );
                    }
                    report.backpressure += 1;
                    warn!(
                        quadrant_x = coord.x(),
                        quadrant_y = coord.y(),
                        generator_version = identity.generator_version(),
                        generation_inputs_fingerprint = ?identity.generation_inputs_fingerprint(),
                        outcome = "full",
                        remaining_queue_capacity = self.ingress.remaining_queue_capacity(),
                        authoritative_in_flight = frontier.preparation_in_flight_len(),
                        "authoritative private quadrant preparation admission backpressure observed"
                    );
                    break;
                }
                PreparationSubmissionOutcome::Closed => {
                    let abandoned = frontier.cancel_preparation(identity)?;
                    warn!(
                        quadrant_x = coord.x(),
                        quadrant_y = coord.y(),
                        generator_version = identity.generator_version(),
                        generation_inputs_fingerprint = ?identity.generation_inputs_fingerprint(),
                        outcome = "closed",
                        rollback_outcome = ?abandoned,
                        remaining_queue_capacity = self.ingress.remaining_queue_capacity(),
                        authoritative_in_flight = frontier.preparation_in_flight_len(),
                        "authoritative private quadrant preparation admission closed"
                    );
                    bail!("preparation worker request queue is closed");
                }
            }
        }
        Ok(())
    }

    fn retry_allows(&self, coord: QuadrantCoord, now: Instant) -> bool {
        self.retries
            .get(&coord)
            .is_none_or(|retry| !retry.exhausted && now >= retry.retry_after)
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io, time::Duration};

    use aurenfall_contracts::{
        TERRAIN_AUTHORITY_CONTRACT_VERSION, TERRAIN_CELL_BLOCKED, TERRAIN_CELL_BUILDABLE,
        TERRAIN_CELL_WALKABLE, TERRAIN_CELL_WATER,
    };
    use aurenfall_core::QuadrantCoord;
    use aurenfall_domain::{FrontierQuadrantState, InitialFrontier};
    use aurenfall_preparation::PreparationWorkerSettings;
    use aurenfall_terrain::{HydrologyKindV1, sample_hydrology_v1};

    use super::*;

    fn frontier_fixture() -> anyhow::Result<FrontierRuntime> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        FrontierRuntime::new(&initial, 1, 7, 64_000)
    }

    fn scheduler_fixture(max_prepared: usize) -> anyhow::Result<PreparationScheduler> {
        let worker = PreparationWorkerSettings::new(4, 4, 1)?;
        let scheduler = PreparationSchedulerSettings::new(
            Duration::from_millis(10),
            1,
            max_prepared,
            Duration::from_millis(20),
            2,
        )?;
        Ok(PreparationScheduler::spawn(
            worker,
            scheduler,
            UniverseSeed::from_phrase("p1-c-test"),
            64_000,
            7,
            vec![1, 2],
        ))
    }

    fn first_water_bearing_revealed_coord(
        scheduler: &PreparationScheduler,
    ) -> anyhow::Result<FrontierQuadrantCoord> {
        for quadrant_y in -16..=16 {
            for quadrant_x in -16..=16 {
                let coord = FrontierQuadrantCoord::new(quadrant_x, quadrant_y);
                let terrain = scheduler.terrain_authority_for_revealed(coord)?;
                if terrain
                    .cell_flags
                    .iter()
                    .any(|flags| flags & TERRAIN_CELL_WATER != 0)
                {
                    return Ok(coord);
                }
            }
        }

        Err(anyhow::anyhow!(
            "deterministic production hydrology scan did not find a WATER-bearing Quadrant"
        ))
    }

    #[test]
    fn production_helper_carries_known_lake_and_river_into_water_cells() -> anyhow::Result<()> {
        let world_seed = 0xA11C_EFA1_1A11_CE01;
        let generator_version = 1;
        let samples_per_quadrant = i64::from(aurenfall_terrain::TERRAIN_RECIPE_V1_CONTROL_GRID_SIDE - 1);

        for (expected_kind, global_x, global_y) in [
            (HydrologyKindV1::Lake, 33_i64, -128_i64),
            (HydrologyKindV1::River, -46_i64, -128_i64),
        ] {
            let sample = sample_hydrology_v1(
                HydrologyFieldV1Settings {
                    world_seed,
                    generator_version,
                },
                global_x,
                global_y,
            )?;

            assert_eq!(sample.kind, expected_kind);

            let coord = FrontierQuadrantCoord::new(
                global_x.div_euclid(samples_per_quadrant),
                global_y.div_euclid(samples_per_quadrant),
            );

            let terrain = generate_hydrology_integrated_terrain_authority_v1(
                64_000,
                generator_version,
                world_seed,
                coord,
                &[1, 2],
            )?;

            assert!(
                terrain
                    .cell_flags
                    .iter()
                    .any(|flags| flags & TERRAIN_CELL_WATER != 0),
                "known {expected_kind:?} fixture did not reach WATER semantics through production helper"
            );
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revealed_production_hydrology_is_v1_and_water_policy_is_zero() -> Result<(), Box<dyn Error>> {
        let scheduler = scheduler_fixture(1)?;
        let coord = first_water_bearing_revealed_coord(&scheduler)?;
        let (terrain, policy) = scheduler.presentation_contracts_for_revealed(coord)?;

        assert_eq!(terrain.version, TERRAIN_AUTHORITY_CONTRACT_VERSION);
        assert_eq!(terrain.cell_flags.len(), policy.cell_family_masks.len());

        let mut water_cells = 0usize;

        for (flags, family_mask) in terrain.cell_flags.iter().zip(policy.cell_family_masks.iter()) {
            if flags & TERRAIN_CELL_WATER == 0 {
                continue;
            }

            water_cells += 1;
            assert_ne!(flags & TERRAIN_CELL_BLOCKED, 0);
            assert_eq!(flags & TERRAIN_CELL_WALKABLE, 0);
            assert_eq!(flags & TERRAIN_CELL_BUILDABLE, 0);
            assert_eq!(*family_mask, 0);
        }

        assert!(
            water_cells > 0,
            "production Revealed fixture must contain authoritative WATER cells"
        );

        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revealed_presentation_contracts_share_same_terrain_identity() -> Result<(), Box<dyn Error>> {
        let scheduler = scheduler_fixture(1)?;
        let coord = FrontierQuadrantCoord::new(2, -1);
        let (terrain, policy) = scheduler.presentation_contracts_for_revealed(coord)?;

        assert_eq!(terrain.quadrant_coord, policy.quadrant_coord);
        assert_eq!(terrain.generator_version, policy.terrain_generator_version);
        assert_eq!(terrain.control_grid_side, policy.control_grid_side);
        assert_eq!(terrain.cell_flags.len(), policy.cell_family_masks.len());
        assert_eq!(terrain.quadrant_coord, coord);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revealed_water_surface_contracts_share_same_terrain_identity() -> Result<(), Box<dyn Error>> {
        let scheduler = scheduler_fixture(1)?;
        let coord = FrontierQuadrantCoord::new(2, -1);

        let (terrain, policy, water_surface) =
            scheduler.presentation_contracts_with_water_for_revealed(coord)?;

        assert_eq!(terrain.quadrant_coord, coord);
        assert_eq!(terrain.quadrant_coord, policy.quadrant_coord);
        assert_eq!(terrain.quadrant_coord, water_surface.quadrant_coord);

        assert_eq!(terrain.generator_version, policy.terrain_generator_version);
        assert_eq!(terrain.generator_version, water_surface.terrain_generator_version);
        assert_eq!(
            terrain.generator_version,
            water_surface.hydrology_generator_version
        );

        assert_eq!(terrain.control_grid_side, policy.control_grid_side);
        assert_eq!(terrain.control_grid_side, water_surface.control_grid_side);

        assert_eq!(terrain.cell_flags.len(), policy.cell_family_masks.len());
        assert_eq!(terrain.elevation_samples_mm.len(), water_surface.samples.len());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revealed_water_bearing_triple_carries_authoritative_surface_samples()
    -> Result<(), Box<dyn Error>> {
        let scheduler = scheduler_fixture(1)?;
        let coord = first_water_bearing_revealed_coord(&scheduler)?;

        let (terrain, policy, water_surface) =
            scheduler.presentation_contracts_with_water_for_revealed(coord)?;

        let water_cells = terrain
            .cell_flags
            .iter()
            .filter(|flags| **flags & TERRAIN_CELL_WATER != 0)
            .count();

        let wet_samples = water_surface
            .samples
            .iter()
            .copied()
            .filter(|sample| sample.is_wet())
            .count();

        assert!(
            water_cells > 0,
            "production Revealed fixture must contain WATER cells"
        );
        assert!(
            wet_samples > 0,
            "WATER-bearing Revealed fixture must contain authoritative wet surface samples"
        );

        assert_eq!(terrain.quadrant_coord, water_surface.quadrant_coord);
        assert_eq!(terrain.generator_version, water_surface.terrain_generator_version);
        assert_eq!(
            terrain.generator_version,
            water_surface.hydrology_generator_version
        );
        assert_eq!(terrain.elevation_samples_mm.len(), water_surface.samples.len());
        assert_eq!(terrain.cell_flags.len(), policy.cell_family_masks.len());

        for (index, sample) in water_surface.samples.iter().copied().enumerate() {
            if sample.is_wet() {
                assert!(
                    sample.water_surface_mm >= terrain.elevation_samples_mm[index],
                    "wet surface sample {index} is below authoritative terrain"
                );
            }
        }

        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn candidate_is_prepared_privately_without_public_revision_change() -> Result<(), Box<dyn Error>> {
        let mut frontier = frontier_fixture()?;
        let candidates = frontier.candidate_quadrants();
        let expected_coord = candidates
            .first()
            .copied()
            .ok_or_else(|| io::Error::other("candidate fixture must not be empty"))?;
        let mut scheduler = scheduler_fixture(1)?;
        let revision_before = frontier.revision();

        let first = scheduler.evaluate(&mut frontier, Instant::now())?;
        assert_eq!(first.submitted, 1);
        assert_eq!(frontier.preparation_in_flight_len(), 1);
        assert_eq!(frontier.prepared_len(), 0);
        assert_eq!(frontier.revision(), revision_before);

        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            let report = scheduler.evaluate(&mut frontier, Instant::now())?;
            if report.prepared_private == 1 {
                break;
            }
        }

        assert_eq!(frontier.prepared_len(), 1);
        assert_eq!(frontier.preparation_in_flight_len(), 0);
        assert_eq!(frontier.revision(), revision_before);
        assert_eq!(frontier.manifest()?.revision, revision_before);
        assert_eq!(frontier.manifest()?.quadrants.len(), 16);

        let seed = scheduler
            .universe_seed
            .quadrant_seed(expected_coord, scheduler.generator_version);
        let generation_inputs_fingerprint = PreparationFingerprint::new(*seed.as_bytes())?;
        let identity = frontier.preparation_identity(expected_coord, generation_inputs_fingerprint)?;
        let prepared = frontier
            .prepared_artifact(identity)
            .ok_or_else(|| io::Error::other("prepared metadata must exist"))?;
        let terrain = scheduler
            .prepared_terrain_artifact(identity)?
            .ok_or_else(|| io::Error::other("prepared terrain authority artifact must be retained"))?;
        assert_eq!(terrain.byte_len(), prepared.artifact_bytes());
        assert_eq!(terrain.fingerprint(), prepared.artifact_fingerprint().bytes());
        assert_eq!(terrain.contract().quadrant_coord.x, expected_coord.x());
        assert_eq!(terrain.contract().quadrant_coord.y, expected_coord.y());
        assert_eq!(terrain.contract().quadrant_size_mm, 64_000);
        assert_eq!(terrain.contract().generator_version, 7);

        let direct = generate_hydrology_integrated_terrain_authority_v1(
            scheduler.quadrant_size_mm,
            scheduler.generator_version,
            terrain_world_seed_v1(&scheduler.universe_seed),
            FrontierQuadrantCoord::new(expected_coord.x(), expected_coord.y()),
            &[1, 2],
        )?;

        assert_eq!(terrain.contract(), &direct);

        let direct_artifact = TerrainAuthorityArtifactV1::from_contract(direct);
        assert_eq!(terrain.fingerprint(), direct_artifact.fingerprint());
        assert_eq!(terrain.canonical_bytes(), direct_artifact.canonical_bytes());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prepared_budget_stops_speculation_after_one_private_quadrant() -> Result<(), Box<dyn Error>> {
        let mut frontier = frontier_fixture()?;
        let candidates = frontier.candidate_quadrants();
        let expected = candidates
            .first()
            .copied()
            .ok_or_else(|| io::Error::other("candidate fixture must not be empty"))?;
        let mut scheduler = scheduler_fixture(1)?;

        let _ = scheduler.evaluate(&mut frontier, Instant::now())?;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            let _ = scheduler.evaluate(&mut frontier, Instant::now())?;
            if frontier.prepared_len() == 1 {
                break;
            }
        }

        for _ in 0..10 {
            let report = scheduler.evaluate(&mut frontier, Instant::now())?;
            assert_eq!(report.submitted, 0);
        }

        assert_eq!(frontier.prepared_len(), 1);
        assert_eq!(frontier.quadrant_state(expected), FrontierQuadrantState::Prepared);
        assert_eq!(frontier.revision(), 1);
        assert_eq!(frontier.candidate_quadrants().len(), candidates.len() - 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prepared_artifact_matches_direct_hydrology_integrated_generation() -> Result<(), Box<dyn Error>>
    {
        let mut frontier = frontier_fixture()?;
        let expected_coord = frontier
            .candidate_quadrants()
            .first()
            .copied()
            .ok_or_else(|| io::Error::other("candidate fixture must not be empty"))?;

        let mut scheduler = scheduler_fixture(1)?;
        let revision_before = frontier.revision();

        let first = scheduler.evaluate(&mut frontier, Instant::now())?;
        assert_eq!(first.submitted, 1);

        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(2)).await;
            let _ = scheduler.evaluate(&mut frontier, Instant::now())?;
            if frontier.prepared_len() == 1 {
                break;
            }
        }

        assert_eq!(frontier.prepared_len(), 1);
        assert_eq!(frontier.preparation_in_flight_len(), 0);

        // Private preparation must not mutate public discovery truth.
        assert_eq!(frontier.revision(), revision_before);
        assert_eq!(frontier.manifest()?.revision, revision_before);
        assert!(
            frontier
                .manifest()?
                .quadrants
                .iter()
                .all(|coord| { coord.x != expected_coord.x() || coord.y != expected_coord.y() })
        );

        let seed = scheduler
            .universe_seed
            .quadrant_seed(expected_coord, scheduler.generator_version);

        let generation_inputs_fingerprint = PreparationFingerprint::new(*seed.as_bytes())?;

        let identity = frontier.preparation_identity(expected_coord, generation_inputs_fingerprint)?;

        let prepared_metadata = frontier
            .prepared_artifact(identity)
            .ok_or_else(|| io::Error::other("prepared metadata must exist"))?;

        let retained = scheduler
            .prepared_terrain_artifact(identity)?
            .ok_or_else(|| io::Error::other("prepared terrain authority artifact must be retained"))?;

        let direct = generate_hydrology_integrated_terrain_authority_v1(
            scheduler.quadrant_size_mm,
            scheduler.generator_version,
            terrain_world_seed_v1(&scheduler.universe_seed),
            FrontierQuadrantCoord::new(expected_coord.x(), expected_coord.y()),
            &[1, 2],
        )?;

        // The private worker and direct Revealed-generation path must
        // resolve exactly the same authoritative TerrainAuthority.
        assert_eq!(retained.contract(), &direct);

        let direct_artifact = TerrainAuthorityArtifactV1::from_contract(direct);

        assert_eq!(retained.fingerprint(), direct_artifact.fingerprint());

        assert_eq!(retained.canonical_bytes(), direct_artifact.canonical_bytes());

        // Prepared metadata must also identify those exact bytes.
        assert_eq!(
            prepared_metadata.artifact_fingerprint().bytes(),
            retained.fingerprint()
        );

        assert_eq!(prepared_metadata.artifact_bytes(), retained.byte_len());

        Ok(())
    }
    #[test]
    fn scheduler_rejects_zero_budgets() {
        assert!(
            PreparationSchedulerSettings::new(Duration::ZERO, 1, 1, Duration::from_millis(1), 1).is_err()
        );
        assert!(
            PreparationSchedulerSettings::new(Duration::from_millis(1), 0, 1, Duration::from_millis(1), 1)
                .is_err()
        );
        assert!(
            PreparationSchedulerSettings::new(Duration::from_millis(1), 1, 0, Duration::from_millis(1), 1)
                .is_err()
        );
    }
}
