mod development_frontier;
mod frontier_runtime;
mod preparation_scheduler;
mod runtime;
mod session_runtime;
mod snapshot_bridge;
mod world_router;

use std::{path::{Path, PathBuf}, time::Duration};
use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use aurenfall_config::ServerConfig;
use aurenfall_contracts::{
    FrontierQuadrantCoord, MessageKind, MovementPredictionProfile, PresentationCorrectionProfile,
    TERRAIN_CELL_WALKABLE, TERRAIN_CELL_WATER, TerrainAuthorityContractV1,
};
use aurenfall_core::{QuadrantCoord, QuadrantSizeMm, UniverseId, UniverseSeed, WorldPositionMm, ZoneId};
use aurenfall_domain::InitialFrontier;
use aurenfall_gamedata::GameDataRegistry;
use aurenfall_persistence::{connect, run_migrations, PostgresSettings};
use aurenfall_persistence::position_buffer::PositionPersistenceBuffer;
use aurenfall_preparation::PreparationWorkerSettings;
use aurenfall_simulation::{CharacterMovementSettings, StaticTraversalBlocker, TraversalWorld, ZoneRuntime};
use aurenfall_transport::{InitialReliableFrame, QuicServer, QuicServerSettings};
use development_frontier::DevelopmentFrontierRevealConfig;
use frontier_runtime::FrontierRuntime;
use preparation_scheduler::{
    PreparationScheduler, PreparationSchedulerSettings, generate_hydrology_integrated_terrain_authority_v1,
    terrain_world_seed_v1,
};
use session_runtime::{SessionRuntime, SessionRuntimeSettings};
use tokio::{
    sync::{mpsc, watch},
    task::{JoinError, JoinHandle},
};
use tracing::{error, info};
use world_router::WorldRouter;

const INITIAL_FRONTIER_MANIFEST_REVISION: u64 = 1;
const DEVELOPMENT_FRONTIER_REVEAL_DELAY_ENV: &str = "AURENFALL_DEV_FRONTIER_REVEAL_AFTER_MS";
const DEVELOPMENT_HYDROLOGY_PROOF_FRONTIER_ENV: &str = "AURENFALL_DEV_HYDROLOGY_PROOF_FRONTIER";
const DEVELOPMENT_HYDROLOGY_PROOF_SEARCH_RADIUS_QUADRANTS: i64 = 16;

// Bloco de desenvolvimento: paredes estáticas de teste
const DEVELOPMENT_WALL_MIN_X_MM: i64 = 650;
const DEVELOPMENT_WALL_MAX_X_MM: i64 = 750;
const DEVELOPMENT_WALL_MIN_Y_MM: i64 = -2_000;
const DEVELOPMENT_WALL_MAX_Y_MM: i64 = 2_000;

#[derive(Debug)]
enum RuntimeStop {
    Signal(anyhow::Result<()>),
    Transport(Result<anyhow::Result<()>, JoinError>),
    Session(Result<anyhow::Result<()>, JoinError>),
    Zone(Result<anyhow::Result<()>, JoinError>),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    aurenfall_observability::init("info");

    info!("=== AURENFALL WORLD SERVER BOOTING ===");

    // 1. Carrega e valida o GameData primeiro (modelo Server Files de Mu Online)
    let gamedata_dir = Path::new("gamedata");
    let gamedata = match GameDataRegistry::load_from_directory(gamedata_dir) {
        Ok(registry) => {
            info!(
                content_hash = %registry.content_hash,
                "GameData definitions loaded and validated via BLAKE3 hash"
            );
            Arc::new(registry)
        }
        Err(err) => {
            error!(error = %err, "CRITICAL: GameData validation failed. Server boot aborted.");
            bail!("GameData startup failure: {}", err);
        }
    };

    // 2. Carrega as configurações de infraestrutura do servidor (rede, portas, banco)
    let config_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/server.toml"));

    let config = ServerConfig::load(&config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;
    config.validate()?;

    let universe_id = UniverseId(config.universe.id);
    let universe_seed = UniverseSeed::from_phrase(&config.universe.seed);
    let quadrant_size = QuadrantSizeMm::new(config.universe.quadrant_size_mm)?;
    let zone_id = ZoneId(1);
    let development_auto_admit = config.server.environment.eq_ignore_ascii_case("development");

    info!(
        environment = %config.server.environment,
        instance = %config.server.instance_name,
        universe_id = universe_id.0,
        generator_version = config.universe.generator_version,
        quadrant_size_mm = quadrant_size.value(),
        tick_rate_hz = config.simulation.tick_rate_hz,
        network_bind = %config.network.bind_address,
        persistence_enabled = config.persistence.enabled,
        "Universe configuration ready"
    );

    let genesis_coord = QuadrantCoord::new(0, 0);
    let genesis = universe_seed.quadrant_seed(genesis_coord, config.universe.generator_version);
    info!(
        quadrant_x = genesis_coord.x(),
        quadrant_y = genesis_coord.y(),
        quadrant_size_mm = quadrant_size.value(),
        genesis_quadrant_seed = %genesis,
        "universe deterministic quadrant geometry ready"
    );

    let frontier_config = &config.universe.initial_frontier;
    let configured_initial_frontier = InitialFrontier::rectangular(
        QuadrantCoord::new(frontier_config.min_x, frontier_config.min_y),
        frontier_config.width,
        frontier_config.height,
    )?;

    let hydrology_proof_frontier = development_hydrology_proof_frontier(
        development_auto_admit,
        std::env::var_os(DEVELOPMENT_HYDROLOGY_PROOF_FRONTIER_ENV).is_some(),
        &configured_initial_frontier,
        &universe_seed,
        quadrant_size.value(),
        config.universe.generator_version,
    )?;

    let development_spawn_position = if let Some(proof) = &hydrology_proof_frontier {
        proof.spawn_position
    } else {
        let world_seed = terrain_world_seed_v1(&universe_seed);
        let genesis_terrain = generate_hydrology_integrated_terrain_authority_v1(
            quadrant_size.value(),
            config.universe.generator_version,
            world_seed,
            FrontierQuadrantCoord::new(0, 0),
            &[],
        );

        let ground_z = match genesis_terrain {
            Ok(terrain) => {
                let center_idx = terrain.elevation_samples_mm.len() / 2;
                i64::from(terrain.elevation_samples_mm[center_idx]).saturating_add(300)
            }
            Err(_) => 500,
        };

        WorldPositionMm::new(0, 0, ground_z)
    };

    let initial_frontier = if let Some(proof) = hydrology_proof_frontier {
        let spawn_quadrant = proof.spawn_position.quadrant_coord(quadrant_size);

        info!(
            target_quadrant_x = proof.target.x(),
            target_quadrant_y = proof.target.y(),
            water_cells = proof.water_cells,
            dry_walkable_cells = proof.dry_walkable_cells,
            visual_balance_cells = proof.visual_balance_cells,
            proof_spawn_x_mm = proof.spawn_position.x(),
            proof_spawn_y_mm = proof.spawn_position.y(),
            proof_spawn_quadrant_x = spawn_quadrant.x(),
            proof_spawn_quadrant_y = spawn_quadrant.y(),
            proof_spawn_surface_mm = proof.spawn_surface_mm,
            proof_spawn_cell_flags = proof.spawn_cell_flags,
            width = proof.frontier.width(),
            height = proof.frontier.height(),
            "development-only WATER-bearing proof frontier selected"
        );
        proof.frontier
    } else {
        configured_initial_frontier
    };
    let frontier_min = initial_frontier.min_coord();
    let frontier_max = initial_frontier.max_coord();
    info!(
        revealed_quadrants = initial_frontier.len(),
        min_x = frontier_min.x(),
        min_y = frontier_min.y(),
        max_x = frontier_max.x(),
        max_y = frontier_max.y(),
        width = initial_frontier.width(),
        height = initial_frontier.height(),
        "server-owned initial discovery frontier manifested"
    );

    let frontier_runtime = FrontierRuntime::new(
        &initial_frontier,
        INITIAL_FRONTIER_MANIFEST_REVISION,
        config.universe.generator_version,
        quadrant_size.value(),
    )?;
    let frontier_manifest = frontier_runtime.manifest()?;
    let frontier_manifest_payload = frontier_manifest.encode()?;
    info!(
        manifest_version = frontier_manifest.manifest_version,
        manifest_revision = frontier_runtime.revision(),
        manifest_quadrants = frontier_runtime.revealed_len(),
        manifest_payload_bytes = frontier_manifest_payload.len(),
        "authoritative frontier runtime encoded public manifest for reliable bootstrap"
    );

    let development_frontier_reveal =
        development_frontier_reveal_config(development_auto_admit, &initial_frontier)?;
    if let Some(proof) = development_frontier_reveal {
        info!(
            quadrant_x = proof.coord().x(),
            quadrant_y = proof.coord().y(),
            delay = ?proof.delay(),
            "development-only server-owned frontier reveal proof configured"
        );
    }

    // 3. MOVIMENTO VINDO DIRETAMENTE DO GAMEDATA!
    let movement_settings = CharacterMovementSettings::from_gamedata(&gamedata.movement)?;

    let correction_profile = PresentationCorrectionProfile::new(
        config.simulation.correction_absorb_max_mm,
        config.simulation.correction_smooth_max_mm,
        config.simulation.correction_hard_snap_threshold_mm,
        config.simulation.correction_smooth_duration_ms,
        config.simulation.correction_rapid_duration_ms,
    )?;
    let prediction_profile = MovementPredictionProfile::new(
        movement_settings.speed_mm_per_second(),
        config.simulation.tick_rate_hz,
        movement_settings.character_radius_mm(),
        movement_settings.input_timeout_ticks(),
        correction_profile,
    )?;
    let mut traversal_world = TraversalWorld::default();
    if development_auto_admit {
        let wall = StaticTraversalBlocker::new(
            DEVELOPMENT_WALL_MIN_X_MM,
            DEVELOPMENT_WALL_MAX_X_MM,
            DEVELOPMENT_WALL_MIN_Y_MM,
            DEVELOPMENT_WALL_MAX_Y_MM,
        )?;
        traversal_world.add_static_blocker(wall);
        info!(
            min_x_mm = wall.min_x_mm(),
            max_x_mm = wall.max_x_mm(),
            min_y_mm = wall.min_y_mm(),
            max_y_mm = wall.max_y_mm(),
            character_radius_mm = movement_settings.character_radius_mm(),
            "development traversal wall installed"
        );
    }

    let (snapshot_tx, snapshot_rx) = mpsc::channel(config.simulation.movement_snapshot_capacity);
    let (zone_tx, zone_rx) = mpsc::channel(config.simulation.zone_command_capacity);
    let (gameplay_tx, mut gameplay_rx) = mpsc::channel(64);

    let zone = ZoneRuntime::new(
        zone_id,
        config.simulation.tick_rate_hz,
        movement_settings,
        traversal_world,
        zone_rx,
    )?
    .with_movement_snapshots(snapshot_tx)
    .with_gameplay_events(gameplay_tx);

    let (postgres_client, position_buffer) = if config.persistence.enabled {
        info!(
            connection_string = %config.persistence.connection_string,
            "connecting to PostgreSQL persistence layer"
        );
        let settings = PostgresSettings {
            connection_string: config.persistence.connection_string.clone(),
        };
        let client = connect(&settings).await.context("failed to connect to PostgreSQL")?;
        run_migrations(&client).await.context("failed to run persistence migrations")?;
        info!("PostgreSQL migrations applied successfully");

        // Envolvemos o cliente numa Arc para partilha segura entre threads
        let client_arc = Arc::new(client);

        // Cria o buffer de posições utilizando o Arc<Client>
        let buffer = Arc::new(PositionPersistenceBuffer::new(client_arc.clone()));
        
        // Dispara a task assíncrona que descarrega o buffer no Postgres a cada 3 segundos
        buffer.clone().start_background_flusher(Duration::from_secs(3));

        (Some(client_arc), Some(buffer))
    } else {
        info!("PostgreSQL persistence disabled by server configuration");
        (None, None)
    };

    let mut world_router = WorldRouter::default();
    world_router.register_zone(zone_id, zone_tx.clone())?;

   // Busca a definição da rocha inicial diretamente no GameData carregado
    let rock_def = gamedata
        .gathering
        .resources
        .iter()
        .find(|r| r.id == "rock_granite_basic")
        .context("Configuração da rocha 'rock_granite_basic' não encontrada no GameData")?;

    let initial_rock_pos = WorldPositionMm::new(
        development_spawn_position.x().saturating_add(1_500),
        development_spawn_position.y(),
        development_spawn_position.z(),
    );
    info!(
        resource_id = rock_def.resource_type_id,
        health = rock_def.max_health,
        x_mm = initial_rock_pos.x(),
        y_mm = initial_rock_pos.y(),
        "Spawning authoritative playable rock entity from GameData"
    );
    let _ = zone_tx
        .send(aurenfall_simulation::ZoneCommand::SpawnRock {
            position: initial_rock_pos,
            health: rock_def.max_health,
        })
        .await;




// 1. Coleta os IDs numéricos dos biomas carregados a partir de world/biomes.yaml
    let mut available_biome_ids: Vec<u8> = gamedata
        .biomes
        .biomes
        .values()
        .map(|config| config.numeric_id)
        .collect();
    available_biome_ids.sort_unstable();

    info!(
        available_biomes = ?available_biome_ids,
        "Authoritative biome pool registered for world generation"
    );

    // 2. Configurações dos workers de preparação em segundo plano
    let preparation_worker_settings = PreparationWorkerSettings::new(
        config.preparation.request_queue_capacity,
        config.preparation.result_queue_capacity,
        config.preparation.max_in_flight,
    )?;
    let preparation_scheduler_settings = PreparationSchedulerSettings::new(
        Duration::from_millis(config.preparation.scheduler_interval_ms),
        config.preparation.max_submissions_per_tick,
        config.preparation.max_prepared_quadrants,
        Duration::from_millis(config.preparation.retry_backoff_ms),
        config.preparation.max_retry_attempts,
    )?;

    // 3. Inicializa o agendador de geração procedural com o pool de biomas
    let preparation_scheduler = PreparationScheduler::spawn(
        preparation_worker_settings,
        preparation_scheduler_settings,
        universe_seed.clone(),
        quadrant_size.value(),
        config.universe.generator_version,
        available_biome_ids,
    );




    let (session_event_tx, session_event_rx) = mpsc::channel(config.network.session_event_capacity);
    let session_settings = SessionRuntimeSettings::new(
        config.simulation.zone_command_capacity,
        zone_id,
        development_auto_admit,
        development_frontier_reveal,
    )
    .with_development_spawn_position(development_spawn_position)
    .with_frontier_delivery(
        Duration::from_millis(config.network.frontier_ack_retry_interval_ms),
        Duration::from_millis(config.network.frontier_ack_timeout_ms),
        config.network.frontier_ack_max_retries,
    )?;
    let mut session_runtime = SessionRuntime::new(
        session_event_rx,
        snapshot_rx,
        session_settings,
        world_router,
        frontier_runtime,
    )?
    .with_preparation_scheduler(preparation_scheduler);

    // Conecta o cliente PostgreSQL se ele estiver ativo
    if let Some(pg_client) = postgres_client {
        session_runtime = session_runtime.with_postgres_client(pg_client);
    }

    // Conecta o buffer de persistência se ele estiver ativo
    if let Some(buffer) = position_buffer {
        session_runtime = session_runtime.with_position_buffer(buffer);
    }
    let transport = QuicServer::bind(QuicServerSettings {
        bind_address: config
            .network
            .bind_address
            .parse()
            .context("invalid network.bind_address")?,
        certificate_der_path: PathBuf::from(&config.network.certificate_der_path),
        private_key_der_path: PathBuf::from(&config.network.private_key_der_path),
        universe_id,
        max_connections: config.network.max_connections,
        max_datagram_bytes: config.network.max_datagram_bytes,
        max_control_frame_bytes: config.network.max_control_frame_bytes,
        max_bidi_streams: config.network.max_bidi_streams,
        max_uni_streams: config.network.max_uni_streams,
        handshake_timeout: Duration::from_millis(config.network.handshake_timeout_ms),
        rejection_close_grace: Duration::from_millis(config.network.rejection_close_grace_ms),
        idle_timeout: Duration::from_millis(config.network.idle_timeout_ms),
        datagram_receive_buffer_bytes: config.network.datagram_receive_buffer_bytes,
        datagram_send_buffer_bytes: config.network.datagram_send_buffer_bytes,
        minimum_client_build: config.network.minimum_client_build,
    })?
    .with_initial_reliable_frame(InitialReliableFrame::new(
        MessageKind::MovementPredictionProfile,
        prediction_profile.encode().to_vec(),
    ))?
    .with_initial_reliable_frame(InitialReliableFrame::new(
        MessageKind::FrontierManifest,
        frontier_manifest_payload,
    ))?
    .with_session_events(session_event_tx);

    info!(
        profile_version = prediction_profile.profile_version,
        speed_mm_per_second = prediction_profile.speed_mm_per_second,
        simulation_tick_hz = prediction_profile.simulation_tick_hz,
        movement_input_timeout_ticks = prediction_profile.movement_input_timeout_ticks,
        character_radius_mm = prediction_profile.character_radius_mm,
        correction_absorb_max_mm = correction_profile.absorb_max_mm,
        correction_smooth_max_mm = correction_profile.smooth_max_mm,
        correction_hard_snap_threshold_mm = correction_profile.hard_snap_threshold_mm,
        correction_smooth_duration_ms = correction_profile.smooth_duration_ms,
        correction_rapid_duration_ms = correction_profile.rapid_duration_ms,
        "server-owned movement prediction profile configured from GameData"
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut zone_task = tokio::spawn(zone.run());
    let mut session_task = tokio::spawn(session_runtime.run());
    let mut transport_task = tokio::spawn(transport.run(shutdown_rx));

    info!("Aurenfall Server 2 ready; press Ctrl+C to stop");
    let stop = tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            RuntimeStop::Signal(signal.context("failed to listen for Ctrl+C"))
        }
        Some(destroyed_rock) = gameplay_rx.recv() => {
            info!(
                x_mm = destroyed_rock.position.x(),
                y_mm = destroyed_rock.position.y(),
                "MINING LOOP COMPLETO: Pedra quebrada no mundo autoritativo! Desencadeando recompensa e expansao de fronteira..."
            );
            tokio::signal::ctrl_c().await.map_err(anyhow::Error::from)?;
            RuntimeStop::Signal(Ok(()))
        }
        result = &mut transport_task => RuntimeStop::Transport(result),
        result = &mut session_task => RuntimeStop::Session(result),
        result = &mut zone_task => RuntimeStop::Zone(result),
    };

    info!("shutdown requested");
    let _ = shutdown_tx.send(true);
    drop(zone_tx);

    match stop {
        RuntimeStop::Signal(signal_result) => {
            let transport_result = await_runtime_task("transport", transport_task).await;
            let session_result = await_runtime_task("session", session_task).await;
            let zone_result = await_runtime_task("zone", zone_task).await;
            signal_result?;
            transport_result?;
            session_result?;
            zone_result?;
            info!("Aurenfall Server 2 stopped cleanly");
            Ok(())
        }
        RuntimeStop::Transport(result) => {
            let primary = unexpected_runtime_exit("transport", result);
            if let Err(cleanup_error) = await_runtime_task("session", session_task).await {
                error!(%cleanup_error, "session cleanup failed after transport exit");
            }
            if let Err(cleanup_error) = await_runtime_task("zone", zone_task).await {
                error!(%cleanup_error, "zone cleanup failed after transport exit");
            }
            Err(primary)
        }
        RuntimeStop::Session(result) => {
            let primary = unexpected_runtime_exit("session", result);
            if let Err(cleanup_error) = await_runtime_task("transport", transport_task).await {
                error!(%cleanup_error, "transport cleanup failed after session exit");
            }
            if let Err(cleanup_error) = await_runtime_task("zone", zone_task).await {
                error!(%cleanup_error, "zone cleanup failed after session exit");
            }
            Err(primary)
        }
        RuntimeStop::Zone(result) => {
            let primary = unexpected_runtime_exit("zone", result);
            if let Err(cleanup_error) = await_runtime_task("transport", transport_task).await {
                error!(%cleanup_error, "transport cleanup failed after zone exit");
            }
            if let Err(cleanup_error) = await_runtime_task("session", session_task).await {
                error!(%cleanup_error, "session cleanup failed after zone exit");
            }
            Err(primary)
        }
    }
}

// Funções auxiliares mantidas intactas
struct DevelopmentHydrologyProofFrontier {
    frontier: InitialFrontier,
    target: QuadrantCoord,
    water_cells: usize,
    dry_walkable_cells: usize,
    visual_balance_cells: usize,
    spawn_position: WorldPositionMm,
    spawn_cell_flags: u8,
    spawn_surface_mm: i32,
}

struct DevelopmentHydrologyProofCandidate {
    target: QuadrantCoord,
    water_cells: usize,
    dry_walkable_cells: usize,
    visual_balance_cells: usize,
    spawn_position: WorldPositionMm,
    spawn_cell_flags: u8,
    spawn_surface_mm: i32,
}

fn development_hydrology_proof_spawn_position(
    terrain: &TerrainAuthorityContractV1,
) -> anyhow::Result<(WorldPositionMm, u8, i32)> {
    let cell_side = terrain
        .control_grid_side
        .checked_sub(1)
        .context("hydrology proof terrain must contain semantic cells")?;

    if cell_side == 0 {
        bail!("hydrology proof terrain has zero semantic cell side");
    }

    let grid_side = usize::from(terrain.control_grid_side);
    let mut selected: Option<(i64, u16, u16, u8, i32)> = None;

    for local_y in 0..cell_side {
        for local_x in 0..cell_side {
            let index = usize::from(local_y) * usize::from(cell_side) + usize::from(local_x);

            let flags = terrain.cell_flags[index];

            if flags & TERRAIN_CELL_WATER != 0 || flags & TERRAIN_CELL_WALKABLE == 0 {
                continue;
            }

            let i00 = usize::from(local_y) * grid_side + usize::from(local_x);
            let i10 = i00 + 1;
            let i01 = i00 + grid_side;
            let i11 = i01 + 1;

            let surface_sum = i64::from(terrain.elevation_samples_mm[i00])
                + i64::from(terrain.elevation_samples_mm[i10])
                + i64::from(terrain.elevation_samples_mm[i01])
                + i64::from(terrain.elevation_samples_mm[i11]);

            let surface_mm =
                i32::try_from(surface_sum / 4).context("hydrology proof spawn surface overflow")?;

            let score = i64::from(surface_mm).abs();

            let replace = selected
                .as_ref()
                .is_none_or(|(best_score, best_y, best_x, _, _)| {
                    (score, local_y, local_x) < (*best_score, *best_y, *best_x)
                });

            if replace {
                selected = Some((score, local_y, local_x, flags, surface_mm));
            }
        }
    }

    let (_, local_y, local_x, flags, surface_mm) =
        selected.context("WATER-bearing proof Quadrant has no non-WATER WALKABLE spawn cell")?;

    let quadrant_size = i128::from(terrain.quadrant_size_mm);
    let denominator = i128::from(cell_side) * 2;

    let origin_x = i128::from(terrain.quadrant_coord.x)
        .checked_mul(quadrant_size)
        .context("hydrology proof spawn quadrant x overflow")?;

    let origin_y = i128::from(terrain.quadrant_coord.y)
        .checked_mul(quadrant_size)
        .context("hydrology proof spawn quadrant y overflow")?;

    let offset_x = ((i128::from(local_x) * 2 + 1)
        .checked_mul(quadrant_size)
        .context("hydrology proof spawn local x overflow")?)
        / denominator;

    let offset_y = ((i128::from(local_y) * 2 + 1)
        .checked_mul(quadrant_size)
        .context("hydrology proof spawn local y overflow")?)
        / denominator;

    let x_mm = i64::try_from(origin_x + offset_x).context("hydrology proof spawn global x overflow")?;

    let y_mm = i64::try_from(origin_y + offset_y).context("hydrology proof spawn global y overflow")?;

    let z_mm = i64::from(surface_mm).saturating_add(300);
    Ok((WorldPositionMm::new(x_mm, y_mm, z_mm), flags, surface_mm))
}

fn development_hydrology_proof_frontier(
    development_environment: bool,
    requested: bool,
    configured: &InitialFrontier,
    universe_seed: &UniverseSeed,
    quadrant_size_mm: i64,
    generator_version: u32,
) -> anyhow::Result<Option<DevelopmentHydrologyProofFrontier>> {
    if !requested {
        return Ok(None);
    }

    if !development_environment {
        bail!(
            "{DEVELOPMENT_HYDROLOGY_PROOF_FRONTIER_ENV} is development-only and cannot be enabled in this environment"
        );
    }

    let half_width = i64::from(configured.width() / 2);
    let half_height = i64::from(configured.height() / 2);

    let center_x = configured
        .min_coord()
        .x()
        .checked_add(half_width)
        .context("development hydrology proof center x overflow")?;
    let center_y = configured
        .min_coord()
        .y()
        .checked_add(half_height)
        .context("development hydrology proof center y overflow")?;

    let world_seed = terrain_world_seed_v1(universe_seed);
    let mut selected: Option<DevelopmentHydrologyProofCandidate> = None;

    for offset_y in -DEVELOPMENT_HYDROLOGY_PROOF_SEARCH_RADIUS_QUADRANTS
        ..=DEVELOPMENT_HYDROLOGY_PROOF_SEARCH_RADIUS_QUADRANTS
    {
        for offset_x in -DEVELOPMENT_HYDROLOGY_PROOF_SEARCH_RADIUS_QUADRANTS
            ..=DEVELOPMENT_HYDROLOGY_PROOF_SEARCH_RADIUS_QUADRANTS
        {
            let quadrant_x = center_x
                .checked_add(offset_x)
                .context("development hydrology proof quadrant x overflow")?;
            let quadrant_y = center_y
                .checked_add(offset_y)
                .context("development hydrology proof quadrant y overflow")?;

            let terrain = generate_hydrology_integrated_terrain_authority_v1(
                quadrant_size_mm,
                generator_version,
                world_seed,
                FrontierQuadrantCoord::new(quadrant_x, quadrant_y),
                &[],
            )?;

            let water_cells = terrain
                .cell_flags
                .iter()
                .filter(|flags| **flags & TERRAIN_CELL_WATER != 0)
                .count();
            let dry_walkable_cells = terrain
                .cell_flags
                .iter()
                .filter(|flags| **flags & TERRAIN_CELL_WATER == 0 && **flags & TERRAIN_CELL_WALKABLE != 0)
                .count();

            if water_cells == 0 || dry_walkable_cells == 0 {
                continue;
            }

            let Ok((spawn_position, spawn_cell_flags, spawn_surface_mm)) =
                development_hydrology_proof_spawn_position(&terrain)
            else {
                continue;
            };

            let candidate = DevelopmentHydrologyProofCandidate {
                target: QuadrantCoord::new(quadrant_x, quadrant_y),
                water_cells,
                dry_walkable_cells,
                visual_balance_cells: water_cells.min(dry_walkable_cells),
                spawn_position,
                spawn_cell_flags,
                spawn_surface_mm,
            };

            let replace = selected.as_ref().is_none_or(|best| {
                (candidate.visual_balance_cells, candidate.water_cells)
                    > (best.visual_balance_cells, best.water_cells)
                    || ((candidate.visual_balance_cells, candidate.water_cells)
                        == (best.visual_balance_cells, best.water_cells)
                        && (candidate.target.y(), candidate.target.x()) < (best.target.y(), best.target.x()))
            });

            if replace {
                selected = Some(candidate);
            }
        }
    }

    let selected = selected.context(
        "development hydrology proof could not find a WATER-bearing Quadrant with a safe dry WALKABLE spawn",
    )?;

    let min_x = selected
        .target
        .x()
        .checked_sub(half_width)
        .context("development hydrology proof min x overflow")?;
    let min_y = selected
        .target
        .y()
        .checked_sub(half_height)
        .context("development hydrology proof min y overflow")?;

    let frontier = InitialFrontier::rectangular(
        QuadrantCoord::new(min_x, min_y),
        configured.width(),
        configured.height(),
    )?;

    Ok(Some(DevelopmentHydrologyProofFrontier {
        frontier,
        target: selected.target,
        water_cells: selected.water_cells,
        dry_walkable_cells: selected.dry_walkable_cells,
        visual_balance_cells: selected.visual_balance_cells,
        spawn_position: selected.spawn_position,
        spawn_cell_flags: selected.spawn_cell_flags,
        spawn_surface_mm: selected.spawn_surface_mm,
    }))
}

fn development_frontier_reveal_config(
    development_environment: bool,
    initial_frontier: &InitialFrontier,
) -> anyhow::Result<Option<DevelopmentFrontierRevealConfig>> {
    let Some(raw_delay) = std::env::var_os(DEVELOPMENT_FRONTIER_REVEAL_DELAY_ENV) else {
        return Ok(None);
    };
    if !development_environment {
        bail!(
            "{DEVELOPMENT_FRONTIER_REVEAL_DELAY_ENV} is development-only and cannot be enabled in this environment"
        );
    }

    let raw_delay = raw_delay
        .into_string()
        .map_err(|_| anyhow!("{DEVELOPMENT_FRONTIER_REVEAL_DELAY_ENV} must be valid UTF-8"))?;
    let delay_ms = raw_delay.parse::<u64>().with_context(|| {
        format!("{DEVELOPMENT_FRONTIER_REVEAL_DELAY_ENV} must be an integer number of milliseconds")
    })?;
    let target_x = initial_frontier
        .max_coord()
        .x()
        .checked_add(1)
        .context("development frontier proof target x overflow")?;
    let target_y = initial_frontier
        .min_coord()
        .y()
        .checked_add(i64::from(initial_frontier.height() / 2))
        .context("development frontier proof target y overflow")?;
    let config = DevelopmentFrontierRevealConfig::new(
        QuadrantCoord::new(target_x, target_y),
        Duration::from_millis(delay_ms),
    )?;
    Ok(Some(config))
}

async fn await_runtime_task(name: &'static str, task: JoinHandle<anyhow::Result<()>>) -> anyhow::Result<()> {
    task.await
        .with_context(|| format!("{name} task join failed"))?
        .with_context(|| format!("{name} runtime failed"))
}

fn unexpected_runtime_exit(
    name: &'static str,
    result: Result<anyhow::Result<()>, JoinError>,
) -> anyhow::Error {
    match result {
        Ok(Ok(())) => anyhow!("{name} runtime stopped unexpectedly"),
        Ok(Err(error)) => error.context(format!("{name} runtime failed")),
        Err(error) => anyhow!("{name} task join failed: {error}"),
    }
}