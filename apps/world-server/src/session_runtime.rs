#[path = "frontier_delivery.rs"]
mod frontier_delivery;
#[path = "frontier_sync.rs"]
mod frontier_sync;

use std::{
    collections::{HashMap, hash_map::Entry},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use aurenfall_contracts::{
    ENVIRONMENT_PRESENTATION_POLICY_MIN_PROTOCOL_MINOR, MessageKind, TERRAIN_CELL_WATER,
    WATER_SURFACE_PRESENTATION_MIN_PROTOCOL_MINOR,
};
use aurenfall_core::{
    AccountId, CharacterId, ConnectionId, IntentSequence, SessionId, WorldPositionMm, ZoneId,
};
use aurenfall_domain::AuthoritativeRevealOutcome;
use aurenfall_session::{AuthorizedIntent, IntentIngressError, IntentPayload, SessionIntentIngress};
use aurenfall_simulation::{MovementSnapshot, InventoryDespawnResponder};

use aurenfall_transport::{ReliableCapabilitySendOutcome, ReliableConnectionSender, TransportSessionEvent};
use tokio::{
    sync::mpsc,
    task::JoinSet,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, info, warn};



use std::sync::Arc;
use aurenfall_domain::items::CharacterInventoryState;
use aurenfall_persistence::InventoryRepository;
use aurenfall_persistence::position_buffer::{PositionPersistenceBuffer, CharacterPositionUpdate};
use tokio_postgres::Client as PostgresClient;

use self::{
    frontier_delivery::{FrontierDeliveryAction, FrontierDeliveryPolicy, FrontierDeliveryTracker},
    frontier_sync::{FrontierAckOutcome, FrontierSyncError, FrontierSyncRegistry},
};
use crate::{
    development_frontier::{DevelopmentFrontierRevealConfig, DevelopmentFrontierRevealGate},
    frontier_runtime::FrontierRuntime,
    preparation_scheduler::PreparationScheduler,
    snapshot_bridge::{SelfSnapshotBridge, SnapshotRouteOutcome},
    world_router::{WorldRouteOutcome, WorldRouter},
};

const DEVELOPMENT_FRONTIER_EVALUATION_INTERVAL: Duration = Duration::from_millis(100);
const FRONTIER_DELIVERY_EVALUATION_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy)]
pub struct SessionRuntimeSettings {
    authorized_capacity: usize,
    zone_id: ZoneId,
    development_auto_admit: bool,
    development_frontier_reveal: Option<DevelopmentFrontierRevealConfig>,
    development_spawn_position: WorldPositionMm,
    frontier_delivery_policy: FrontierDeliveryPolicy,
}

impl SessionRuntimeSettings {
    #[must_use]
    pub const fn new(
        authorized_capacity: usize,
        zone_id: ZoneId,
        development_auto_admit: bool,
        development_frontier_reveal: Option<DevelopmentFrontierRevealConfig>,
    ) -> Self {
        Self {
            authorized_capacity,
            zone_id,
            development_auto_admit,
            development_frontier_reveal,
            development_spawn_position: WorldPositionMm::ORIGIN,
            frontier_delivery_policy: FrontierDeliveryPolicy::DEFAULT,
        }
    }

    #[must_use]
    pub const fn with_development_spawn_position(mut self, position: WorldPositionMm) -> Self {
        self.development_spawn_position = position;
        self
    }
    pub fn with_frontier_delivery(
        mut self,
        retry_interval: Duration,
        ack_timeout: Duration,
        max_retries: u32,
    ) -> anyhow::Result<Self> {
        self.frontier_delivery_policy = FrontierDeliveryPolicy::new(retry_interval, ack_timeout, max_retries)
            .context("invalid frontier delivery policy")?;
        Ok(self)
    }
}

#[derive(Debug)]
pub struct SessionRuntime {
    ingress: SessionIntentIngress,
    authorized_rx: mpsc::Receiver<AuthorizedIntent>,
    transport_events: mpsc::Receiver<TransportSessionEvent>,
    movement_snapshots: mpsc::Receiver<MovementSnapshot>,
    snapshot_bridge: SelfSnapshotBridge,
    reliable_by_connection: HashMap<ConnectionId, ReliableConnectionSender>,
    connection_by_session: HashMap<SessionId, ConnectionId>,
    frontier_sync: FrontierSyncRegistry,
    frontier_delivery: FrontierDeliveryTracker,
    preparation_scheduler: Option<PreparationScheduler>,
    world_router: WorldRouter,
    frontier_runtime: FrontierRuntime,
    bootstrap_frontier_revision: u64,
    development_frontier_reveal: Option<DevelopmentFrontierRevealGate>,
    development_auto_admit: bool,
    development_spawn_position: WorldPositionMm,
    zone_id: ZoneId,
    postgres_client: Option<Arc<PostgresClient>>,
    position_buffer: Option<Arc<PositionPersistenceBuffer>>,
}

impl SessionRuntime {
    pub fn new(
        transport_events: mpsc::Receiver<TransportSessionEvent>,
        movement_snapshots: mpsc::Receiver<MovementSnapshot>,
        settings: SessionRuntimeSettings,
        world_router: WorldRouter,
        mut frontier_runtime: FrontierRuntime,
    ) -> anyhow::Result<Self> {
        if settings.development_frontier_reveal.is_some() && !settings.development_auto_admit {
            bail!("development frontier reveal proof requires development environment");
        }
        let bootstrap_frontier_revision = frontier_runtime.revision();
        let development_frontier_reveal = settings
            .development_frontier_reveal
            .map(|config| DevelopmentFrontierRevealGate::new(&mut frontier_runtime, config))
            .transpose()?;
        let (ingress, authorized_rx) = SessionIntentIngress::bounded(settings.authorized_capacity)
            .context("failed to create bounded authorized session ingress")?;
        Ok(Self {
            ingress,
            authorized_rx,
            transport_events,
            movement_snapshots,
            snapshot_bridge: SelfSnapshotBridge::default(),
            reliable_by_connection: HashMap::new(),
            connection_by_session: HashMap::new(),
            frontier_sync: FrontierSyncRegistry::default(),
            frontier_delivery: FrontierDeliveryTracker::new(settings.frontier_delivery_policy),
            preparation_scheduler: None,
            world_router,
            frontier_runtime,
            bootstrap_frontier_revision,
            development_frontier_reveal,
            development_auto_admit: settings.development_auto_admit,
            development_spawn_position: settings.development_spawn_position,
            zone_id: settings.zone_id,
            postgres_client: None,
            position_buffer: None,
        })
    }

    #[must_use]
    pub fn with_preparation_scheduler(mut self, scheduler: PreparationScheduler) -> Self {
        self.preparation_scheduler = Some(scheduler);
        self
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn with_postgres_client(mut self, client: Arc<PostgresClient>) -> Self {
        self.postgres_client = Some(client);
        self
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        info!(
            development_auto_admit = self.development_auto_admit,
            frontier_revision = self.frontier_runtime.revision(),
            revealed_quadrants = self.frontier_runtime.revealed_len(),
            "authoritative session runtime started"
        );

        let development_frontier_enabled = self.development_frontier_reveal.is_some();
        let mut development_frontier_tick = interval(DEVELOPMENT_FRONTIER_EVALUATION_INTERVAL);
        development_frontier_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut frontier_delivery_tick = interval(FRONTIER_DELIVERY_EVALUATION_INTERVAL);
        frontier_delivery_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let preparation_enabled = self.preparation_scheduler.is_some();
        let preparation_interval = self
            .preparation_scheduler
            .as_ref()
            .map_or(Duration::from_secs(1), PreparationScheduler::evaluation_interval);
        let mut preparation_tick = interval(preparation_interval);
        preparation_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut auto_save_ticker = tokio::time::interval(Duration::from_secs(30));
        auto_save_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        

        loop {
            tokio::select! {
                event = self.transport_events.recv() => {
                    match event {
                        Some(event) => self.handle_transport_event(event).await?,
                        None => break,
                    }
                }
                authorized = self.authorized_rx.recv() => {
                    match authorized {
                        Some(intent) => self.route_authorized_intent(intent)?,
                        None => bail!("authorized intent queue closed while session runtime was active"),
                    }
                }
                snapshot = self.movement_snapshots.recv() => {
                    match snapshot {
                        Some(snapshot) => self.route_movement_snapshot(snapshot)?,
                        None => break,
                    }
                }
                _ = frontier_delivery_tick.tick() => {
                    self.evaluate_frontier_delivery(Instant::now()).await?;
                }
                _ = preparation_tick.tick(), if preparation_enabled => {
                    self.evaluate_preparation_pipeline(Instant::now())?;
                }
                _ = development_frontier_tick.tick(), if development_frontier_enabled => {
                    self.evaluate_development_frontier_reveal(Instant::now()).await?;
                }
                _ = auto_save_ticker.tick() => {
                    self.auto_save_active_inventories().await;
                }
            }
        }

        info!(
            live_sessions = self.ingress.live_session_count(),
            frontier_revision = self.frontier_runtime.revision(),
            revealed_quadrants = self.frontier_runtime.revealed_len(),
            "authoritative session runtime stopped"
        );
        Ok(())
    }

    pub fn with_position_buffer(mut self, buffer: Arc<PositionPersistenceBuffer>) -> Self {
        self.position_buffer = Some(buffer);
        self
    }

    async fn handle_transport_event(&mut self, event: TransportSessionEvent) -> anyhow::Result<()> {
        match event {
            TransportSessionEvent::Admitted {
                connection_id,
                session_id,
                realtime,
                reliable,
            } => {
                if realtime.connection_id() != connection_id || reliable.connection_id() != connection_id {
                    bail!(
                        "transport-admitted session capabilities do not match connection {}",
                        connection_id.0
                    );
                }

                match self.reliable_by_connection.entry(connection_id) {
                    Entry::Vacant(entry) => {
                        entry.insert(reliable);
                    }
                    Entry::Occupied(_) => bail!(
                        "connection {} already has reliable control capability",
                        connection_id.0
                    ),
                }

                if let Err(error) = self.snapshot_bridge.register_connection(realtime) {
                    self.reliable_by_connection.remove(&connection_id);
                    return Err(error).context("failed to register realtime snapshot capability");
                }
                if let Err(error) = self.ingress.register(session_id, connection_id) {
                    self.snapshot_bridge.remove_connection(connection_id);
                    self.reliable_by_connection.remove(&connection_id);
                    return Err(error).context("failed to register transport-admitted session");
                }
                match self.connection_by_session.entry(session_id) {
                    Entry::Vacant(entry) => {
                        entry.insert(connection_id);
                    }
                    Entry::Occupied(_) => {
                        self.snapshot_bridge.remove_connection(connection_id);
                        self.reliable_by_connection.remove(&connection_id);
                        let _ = self.ingress.remove_by_connection(connection_id);
                        bail!("session {} already has a live connection mapping", session_id.0);
                    }
                }
                if let Err(error) = self
                    .frontier_sync
                    .register_pending(connection_id, self.frontier_runtime.revision())
                {
                    self.connection_by_session.remove(&session_id);
                    self.snapshot_bridge.remove_connection(connection_id);
                    self.reliable_by_connection.remove(&connection_id);
                    let _ = self.ingress.remove_by_connection(connection_id);
                    return Err(error).context("failed to register frontier sync barrier");
                }
                self.frontier_delivery
                    .track(connection_id, self.frontier_runtime.revision(), Instant::now());
                info!(
                    connection_id = connection_id.0,
                    session_id = session_id.0,
                    frontier_revision = self.frontier_runtime.revision(),
                    "frontier synchronization pending for admitted session"
                );

                if self.development_auto_admit {
                    let account_id = AccountId(1);
                    let character_id = CharacterId(1);
                    self.ingress
                        .authenticate(connection_id, account_id)
                        .context("development session authentication binding failed")?;
                    self.ingress
                        .bind_character(connection_id, character_id, self.zone_id)
                        .context("development character binding failed")?;
                    self.snapshot_bridge
                        .bind_character(character_id, connection_id)
                        .context("development snapshot character binding failed")?;

                    let binding = self
                        .ingress
                        .world_binding_for_connection(connection_id)?
                        .context("bound development session has no world binding")?;

                    // Carrega o inventário persistido do banco ou inicializa novo
                    let (inventory_state, inventory_revision) = if let Some(pg) = &self.postgres_client {
                        match InventoryRepository::load_inventory(pg, character_id).await {
                            Ok(Some((inv, rev))) => {
                                info!(
                                    character_id = character_id.0,
                                    revision = rev,
                                    items_count = inv.items.len(),
                                    "loaded persistent character inventory from PostgreSQL"
                                );
                                (inv, rev)
                            }
                            Ok(None) => {
                                info!(
                                    character_id = character_id.0,
                                    "no persistent inventory found; initializing empty grid"
                                );
                                (CharacterInventoryState::new(10, 6), 0)
                            }
                            Err(error) => {
                                warn!(
                                    character_id = character_id.0,
                                    %error,
                                    "failed to query character inventory; using empty fallback"
                                );
                                (CharacterInventoryState::new(10, 6), 0)
                            }
                        }
                    } else {
                        (CharacterInventoryState::new(10, 6), 0)
                    };

                    // Carrega a última posição persistida do personagem ou usa o ponto padrão
                    let spawn_position = if let Some(pg) = &self.postgres_client {
                        match aurenfall_persistence::CharacterRepository::load_position(pg, character_id).await {
                            Ok(Some(saved_position)) => {
                                info!(
                                    character_id = character_id.0,
                                    x_mm = saved_position.x(),
                                    y_mm = saved_position.y(),
                                    z_mm = saved_position.z(),
                                    "Loaded persistent character spawn position from PostgreSQL"
                                );
                                saved_position
                            }
                            Ok(None) => {
                                info!(
                                    character_id = character_id.0,
                                    "No persistent position found; spawning at default position"
                                );
                                self.development_spawn_position
                            }
                            Err(error) => {
                                warn!(
                                    character_id = character_id.0,
                                    %error,
                                    "Failed to load persistent position; falling back to default"
                                );
                                self.development_spawn_position
                            }
                        }
                    } else {
                        self.development_spawn_position
                    };

                   self.world_router
                        .spawn_character(
                            binding.zone_id(),
                            binding.character_id(),
                            spawn_position,
                            inventory_state,
                            inventory_revision,
                        )
                        .context("failed to spawn development character into authoritative zone")?;

                    if let Err(error) = self.ingress.activate_world(connection_id) {
                        self.world_router
                            .despawn_character(binding.zone_id(), binding.character_id(), None)
                            .context("failed to roll back development character spawn")?;
                        self.frontier_sync.remove(connection_id);
                        self.connection_by_session.remove(&session_id);
                        self.snapshot_bridge.remove_connection(connection_id);
                        self.reliable_by_connection.remove(&connection_id);
                        return Err(error).context("development world activation failed");
                    }

                    info!(
                        connection_id = connection_id.0,
                        session_id = session_id.0,
                        account_id = account_id.0,
                        character_id = character_id.0,
                        zone_id = self.zone_id.0,
                        "development session auto-admitted with server-owned identity and character spawn"
                    );

                    if let Some(gate) = self.development_frontier_reveal.as_mut()
                        && gate.arm(Instant::now())
                    {
                        info!(
                            quadrant_x = gate.coord().x(),
                            quadrant_y = gate.coord().y(),
                            delay = ?gate.delay(),
                            frontier_revision = self.frontier_runtime.revision(),
                            "development server-owned frontier reveal proof armed"
                        );
                    }
                } else {
                    info!(
                        connection_id = connection_id.0,
                        session_id = session_id.0,
                        "protocol session registered; awaiting authentication"
                    );
                }

                if self.frontier_runtime.revision() > self.bootstrap_frontier_revision {
                    self.sync_current_frontier_to_connection(connection_id).await?;
                }
            }
            TransportSessionEvent::MoveIntent {
                connection_id,
                intent,
            } => {
                if !self.frontier_sync.is_synced(connection_id) {
                    debug!(
                        connection_id = connection_id.0,
                        sequence = intent.sequence,
                        frontier_sync = ?self.frontier_sync.state(connection_id),
                        "dropping MoveIntent while frontier synchronization is pending"
                    );
                    return Ok(());
                }
                match self.ingress.try_ingest_move(
                    connection_id,
                    IntentSequence(intent.sequence),
                    intent.axis_x,
                    intent.axis_y,
                ) {
                    Ok(()) => {}
                    Err(IntentIngressError::QueueFull) => {
                        debug!(
                            connection_id = connection_id.0,
                            sequence = intent.sequence,
                            "replaceable movement intent dropped by authorized ingress backpressure"
                        );
                    }
                    Err(IntentIngressError::QueueClosed) => {
                        bail!("authorized intent queue closed during live movement ingress");
                    }
                    Err(IntentIngressError::Sequence(error)) => {
                        warn!(
                            connection_id = connection_id.0,
                            sequence = intent.sequence,
                            %error,
                            "movement intent rejected by authoritative session ingress"
                        );
                    }
                }
            }
            TransportSessionEvent::FrontierManifestAck { connection_id, ack } => {
                let Some(sender) = self.reliable_by_connection.get(&connection_id).cloned() else {
                    debug!(
                        connection_id = connection_id.0,
                        frontier_revision = ack.revision,
                        "dropping FrontierManifestAck after connection route was removed"
                    );
                    return Ok(());
                };

                match self.frontier_sync.apply_ack(connection_id, ack.revision) {
                    Ok(FrontierAckOutcome::Synchronized { revision }) => {
                        self.frontier_delivery.acknowledge(connection_id, revision);
                        info!(
                            connection_id = connection_id.0,
                            frontier_revision = revision,
                            "frontier synchronization acknowledged; gameplay delivery resumed"
                        );
                        self.send_revealed_presentation_contracts_to_connection(connection_id, revision)
                            .await?;
                    }
                    Ok(FrontierAckOutcome::Duplicate { revision }) => debug!(
                        connection_id = connection_id.0,
                        frontier_revision = revision,
                        "duplicate current FrontierManifestAck is idempotent"
                    ),
                    Ok(FrontierAckOutcome::Stale { expected, received }) => debug!(
                        connection_id = connection_id.0,
                        expected_frontier_revision = expected,
                        received_frontier_revision = received,
                        "stale FrontierManifestAck cannot satisfy pending frontier revision"
                    ),
                    Err(FrontierSyncError::FutureAck {
                        expected, received, ..
                    }) => {
                        warn!(
                            connection_id = connection_id.0,
                            expected_frontier_revision = expected,
                            received_frontier_revision = received,
                            "future FrontierManifestAck is a protocol violation; fail-closing connection"
                        );
                        sender.fail_close();
                    }
                    Err(error) => {
                        warn!(
                            connection_id = connection_id.0,
                            frontier_revision = ack.revision,
                            %error,
                            "invalid frontier synchronization state; fail-closing connection"
                        );
                        sender.fail_close();
                    }
                }
            }
            TransportSessionEvent::Disconnected {
                connection_id,
                session_id,
            } => {
                let binding = self
                    .ingress
                    .world_binding_for_connection(connection_id)
                    .context("failed to resolve disconnect world binding")?;
                self.ingress
                    .begin_closing(connection_id)
                    .context("failed to mark disconnected session as closing")?;

                if let Some(binding) = binding {
                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                    let responder = InventoryDespawnResponder::new(resp_tx);

                    self.world_router
                        .despawn_character(
                            binding.zone_id(),
                            binding.character_id(),
                            Some(responder),
                        )
                        .context("failed to despawn disconnected authoritative character")?;

                    if let Some(pg) = &self.postgres_client {
                        if let Ok((inventory_state, revision)) = resp_rx.await {
                            let _ = aurenfall_persistence::InventoryRepository::save_inventory(
                                pg,
                                binding.character_id(),
                                &inventory_state,
                                revision,
                            )
                            .await;
                        }
                    }
                }
                self.frontier_sync.remove(connection_id);
                self.frontier_delivery.remove(connection_id);
                self.connection_by_session.remove(&session_id);
                self.snapshot_bridge.remove_connection(connection_id);
                self.reliable_by_connection.remove(&connection_id);

                let removed = self
                    .ingress
                    .remove_by_connection(connection_id)
                    .context("failed to remove disconnected live session")?;
                if removed != session_id {
                    bail!(
                        "disconnect cleanup removed session {:?}, expected {:?}",
                        removed,
                        session_id
                    );
                }
                debug!(
                    connection_id = connection_id.0,
                    session_id = session_id.0,
                    "live session removed after transport disconnect"
                );
            }
            TransportSessionEvent::MineIntent {
                connection_id,
                intent,
            } => {
                if !self.frontier_sync.is_synced(connection_id) {
                    tracing::warn!(
                        connection_id = connection_id.0,
                        "MineIntent descartado: frontier_sync pendente"
                    );
                    return Ok(());
                }

                if let Ok(Some(binding)) = self.ingress.world_binding_for_connection(connection_id) {
                    tracing::info!(
                        character_id = binding.character_id().0,
                        zone_id = binding.zone_id().0,
                        resource_id = intent.resource_id,
                        sequence = intent.sequence,
                        "ROTEANDO MineIntent da UE5 para a simulacao da Zona!"
                    );
                    let _ = self.world_router.send_zone_command(
                        binding.zone_id(),
                        aurenfall_simulation::ZoneCommand::MineIntent {
                            character_id: binding.character_id(),
                            power: 1, // Aplica 1 ponto de dano na rocha por golpe
                        },
                    );
                } else {
                    tracing::warn!(
                        connection_id = connection_id.0,
                        "MineIntent descartado: sem world_binding"
                    );
                }
            }
        }
        Ok(())
    }

    fn evaluate_preparation_pipeline(&mut self, now: Instant) -> anyhow::Result<()> {
        let Some(scheduler) = self.preparation_scheduler.as_mut() else {
            return Ok(());
        };
        let report = scheduler.evaluate(&mut self.frontier_runtime, now)?;
        if report.has_activity() {
            info!(
                submitted = report.submitted,
                completed = report.completed,
                failed = report.failed,
                backpressure = report.backpressure,
                exhausted = report.exhausted,
                private_prepared_quadrants = report.prepared_private,
                preparation_in_flight = report.in_flight,
                public_frontier_revision = self.frontier_runtime.revision(),
                public_revealed_quadrants = self.frontier_runtime.revealed_len(),
                "authoritative private quadrant preparation scheduler evaluated"
            );
        }
        Ok(())
    }

    async fn evaluate_frontier_delivery(&mut self, now: Instant) -> anyhow::Result<()> {
        let actions = self.frontier_delivery.evaluate(now);
        if actions.is_empty() {
            return Ok(());
        }

        let manifest = self.frontier_runtime.manifest()?;
        let payload = manifest.encode()?;

        for action in actions {
            match action {
                FrontierDeliveryAction::Retry {
                    connection_id,
                    revision,
                    attempt,
                } => {
                    let Some(sender) = self.reliable_by_connection.get(&connection_id).cloned() else {
                        continue;
                    };
                    if manifest.revision != revision {
                        warn!(
                            connection_id = connection_id.0,
                            pending_frontier_revision = revision,
                            authoritative_frontier_revision = manifest.revision,
                            "frontier retry revision diverged from authoritative revision; fail-closing connection"
                        );
                        self.frontier_delivery.remove(connection_id);
                        sender.fail_close();
                        continue;
                    }

                    match sender.send(MessageKind::FrontierManifest, &payload).await {
                        Ok(()) => info!(
                            connection_id = connection_id.0,
                            frontier_revision = revision,
                            retry_attempt = attempt,
                            "resent canonical frontier manifest while awaiting ACK"
                        ),
                        Err(error) => {
                            self.frontier_delivery.remove(connection_id);
                            warn!(
                                connection_id = connection_id.0,
                                frontier_revision = revision,
                                retry_attempt = attempt,
                                %error,
                                "frontier manifest retry failed; reliable capability fail-closed"
                            );
                        }
                    }
                }
                FrontierDeliveryAction::Timeout {
                    connection_id,
                    revision,
                    retries_sent,
                } => {
                    if let Some(sender) = self.reliable_by_connection.get(&connection_id) {
                        warn!(
                            connection_id = connection_id.0,
                            frontier_revision = revision,
                            retries_sent,
                            "frontier ACK deadline expired; fail-closing connection"
                        );
                        sender.fail_close();
                    }
                }
            }
        }
        Ok(())
    }

    async fn evaluate_development_frontier_reveal(&mut self, now: Instant) -> anyhow::Result<()> {
        let Some(gate) = self.development_frontier_reveal.as_ref() else {
            return Ok(());
        };
        if gate.is_complete() || !gate.is_ready(now) {
            return Ok(());
        }
        self.commit_development_frontier_reveal(true).await
    }

    async fn commit_development_frontier_reveal(&mut self, satisfied: bool) -> anyhow::Result<()> {
        let Some(gate) = self.development_frontier_reveal.as_ref() else {
            return Ok(());
        };
        if gate.is_complete() {
            return Ok(());
        }

        let coord = gate.coord();
        let requirements = gate.requirements().clone();
        let resolver = gate.resolver(satisfied);
        match self.frontier_runtime.commit_reveal(&requirements, &resolver)? {
            AuthoritativeRevealOutcome::Blocked { evaluation } => {
                debug!(
                    quadrant_x = coord.x(),
                    quadrant_y = coord.y(),
                    unsatisfied_requirements = evaluation.unsatisfied_requirements().len(),
                    frontier_revision = self.frontier_runtime.revision(),
                    "development frontier requirement remains unsatisfied"
                );
            }
            AuthoritativeRevealOutcome::AlreadyRevealed { revision, .. } => {
                if let Some(gate) = self.development_frontier_reveal.as_mut() {
                    gate.mark_complete();
                }
                debug!(
                    quadrant_x = coord.x(),
                    quadrant_y = coord.y(),
                    frontier_revision = revision,
                    "development frontier reveal proof already committed"
                );
            }
            AuthoritativeRevealOutcome::Revealed { transition, .. } => {
                let pending_connections = self
                    .frontier_sync
                    .mark_all_pending(transition.revision())
                    .context("failed to advance live sessions to pending frontier revision")?;
                let pending_since = Instant::now();
                for connection_id in self.reliable_by_connection.keys().copied() {
                    self.frontier_delivery
                        .track(connection_id, transition.revision(), pending_since);
                }
                info!(
                    frontier_revision = transition.revision(),
                    pending_connections, "live sessions marked frontier-pending before manifest delivery"
                );

                let manifest = self.frontier_runtime.manifest()?;
                let payload = manifest.encode()?;
                let (sent, failed) = self.broadcast_frontier_manifest(&payload).await;
                if let Some(gate) = self.development_frontier_reveal.as_mut() {
                    gate.mark_complete();
                }
                info!(
                    quadrant_x = coord.x(),
                    quadrant_y = coord.y(),
                    previous_revision = transition.previous_revision(),
                    frontier_revision = transition.revision(),
                    revealed_quadrants = manifest.quadrants.len(),
                    reliable_recipients = sent,
                    reliable_failures = failed,
                    "development server-owned frontier reveal committed and broadcast"
                );
            }
        }
        Ok(())
    }

    async fn sync_current_frontier_to_connection(&self, connection_id: ConnectionId) -> anyhow::Result<()> {
        let Some(sender) = self.reliable_by_connection.get(&connection_id).cloned() else {
            return Ok(());
        };
        let manifest = self.frontier_runtime.manifest()?;
        let payload = manifest.encode()?;
        match sender.send(MessageKind::FrontierManifest, &payload).await {
            Ok(()) => info!(
                connection_id = connection_id.0,
                frontier_revision = manifest.revision,
                revealed_quadrants = manifest.quadrants.len(),
                "current authoritative frontier synchronized to admitted connection"
            ),
            Err(error) => warn!(
                connection_id = connection_id.0,
                frontier_revision = manifest.revision,
                %error,
                "failed to synchronize current authoritative frontier to admitted connection"
            ),
        }
        Ok(())
    }

    async fn broadcast_frontier_manifest(&self, payload: &[u8]) -> (usize, usize) {
        let mut sends = JoinSet::new();
        for sender in self.reliable_by_connection.values() {
            let sender = sender.clone();
            let payload = payload.to_vec();
            sends.spawn(async move {
                let connection_id = sender.connection_id();
                let result = sender.send(MessageKind::FrontierManifest, &payload).await;
                (connection_id, result)
            });
        }

        let mut sent = 0;
        let mut failed = 0;
        while let Some(joined) = sends.join_next().await {
            match joined {
                Ok((connection_id, Ok(()))) => {
                    sent += 1;
                    debug!(
                        connection_id = connection_id.0,
                        "reliable frontier manifest sent to live connection"
                    );
                }
                Ok((connection_id, Err(error))) => {
                    failed += 1;
                    warn!(
                        connection_id = connection_id.0,
                        %error,
                        "reliable frontier manifest send failed for live connection"
                    );
                }
                Err(error) => {
                    failed += 1;
                    warn!(%error, "reliable frontier manifest send task failed");
                }
            }
        }
        (sent, failed)
    }

    fn isolate_connection_after_reliable_delivery_failure(
        &mut self,
        connection_id: ConnectionId,
    ) -> anyhow::Result<()> {
        let binding = self
            .ingress
            .world_binding_for_connection(connection_id)
            .context("failed to resolve world binding while isolating failed reliable connection")?;

        self.ingress
            .begin_closing(connection_id)
            .context("failed to revoke gameplay after reliable presentation delivery failure")?;

        self.frontier_sync.remove(connection_id);
        self.frontier_delivery.remove(connection_id);
        self.snapshot_bridge.remove_connection(connection_id);
        self.reliable_by_connection.remove(&connection_id);

        if let Some(binding) = binding {
            self.world_router
                .despawn_character(binding.zone_id(), binding.character_id(), None)
                .context("failed to despawn character after reliable presentation delivery failure")?;
        }

        info!(
            connection_id = connection_id.0,
            "isolated connection after reliable presentation delivery failure"
        );

        Ok(())
    }
    async fn send_revealed_presentation_contracts_to_connection(
        &mut self,
        connection_id: ConnectionId,
        frontier_revision: u64,
    ) -> anyhow::Result<()> {
        if self.preparation_scheduler.is_none() {
            return Ok(());
        }
        if !self.frontier_sync.is_synced(connection_id) {
            bail!(
                "presentation contract delivery attempted before frontier synchronization for connection {}",
                connection_id.0
            );
        }
        let Some(sender) = self.reliable_by_connection.get(&connection_id).cloned() else {
            return Ok(());
        };
        let manifest = self.frontier_runtime.manifest()?;
        if manifest.revision != frontier_revision {
            bail!(
                "presentation contract delivery revision mismatch for connection {}: acknowledged={} authoritative={}",
                connection_id.0,
                frontier_revision,
                manifest.revision
            );
        }

        let mut terrain_sent = 0usize;
        let mut policies_sent = 0usize;
        let mut policies_suppressed = 0usize;
        let mut water_surfaces_sent = 0usize;
        let mut water_surfaces_suppressed = 0usize;
        for coord in manifest.quadrants.iter().copied() {
            let scheduler = self
                .preparation_scheduler
                .as_ref()
                .context("presentation scheduler disappeared during revealed delivery")?;

            let (terrain, policy, water_surface) =
                if sender.supports_protocol_minor(WATER_SURFACE_PRESENTATION_MIN_PROTOCOL_MINOR) {
                    let (terrain, policy, water_surface) =
                        scheduler.presentation_contracts_with_water_for_revealed(coord)?;
                    (terrain, policy, Some(water_surface))
                } else {
                    let (terrain, policy) = scheduler.presentation_contracts_for_revealed(coord)?;
                    (terrain, policy, None)
                };

            let water_cells = terrain
                .cell_flags
                .iter()
                .filter(|flags| **flags & TERRAIN_CELL_WATER != 0)
                .count();

            let terrain_payload = terrain
                .encode_wire()
                .context("failed to encode revealed TerrainAuthority contract")?;
            if let Err(error) = sender.send(MessageKind::TerrainAuthority, &terrain_payload).await {
                warn!(
                    connection_id = connection_id.0,
                    frontier_revision,
                    quadrant_x = coord.x,
                    quadrant_y = coord.y,
                    %error,
                    "revealed TerrainAuthority delivery failed; isolating affected connection"
                );
                sender.fail_close();
                self.isolate_connection_after_reliable_delivery_failure(connection_id)?;
                return Ok(());
            }
            terrain_sent += 1;
            info!(
                connection_id = connection_id.0,
                frontier_revision,
                quadrant_x = coord.x,
                quadrant_y = coord.y,
                generator_version = terrain.generator_version,
                grid_side = terrain.control_grid_side,
                elevation_samples = terrain.elevation_samples_mm.len(),
                semantic_cells = terrain.cell_flags.len(),
                water_cells,
                payload_bytes = terrain_payload.len(),
                "sent revealed TerrainAuthority after frontier ACK"
            );

            let policy_payload = policy
                .encode_wire()
                .context("failed to encode revealed EnvironmentPresentationPolicy contract")?;
            match sender
                .send_if_supported(
                    ENVIRONMENT_PRESENTATION_POLICY_MIN_PROTOCOL_MINOR,
                    MessageKind::EnvironmentPresentationPolicy,
                    &policy_payload,
                )
                .await
            {
                Ok(ReliableCapabilitySendOutcome::Sent) => {
                    policies_sent += 1;
                    info!(
                        connection_id = connection_id.0,
                        frontier_revision,
                        quadrant_x = coord.x,
                        quadrant_y = coord.y,
                        terrain_generator_version = policy.terrain_generator_version,
                        policy_generator_version = policy.policy_generator_version,
                        policy_seed = policy.policy_seed,
                        grid_side = policy.control_grid_side,
                        semantic_cells = policy.cell_family_masks.len(),
                        payload_bytes = policy_payload.len(),
                        "sent revealed EnvironmentPresentationPolicy after TerrainAuthority"
                    );
                }
                Ok(ReliableCapabilitySendOutcome::UnsupportedProtocolMinor { negotiated, required }) => {
                    policies_suppressed += 1;
                    debug!(
                        connection_id = connection_id.0,
                        frontier_revision,
                        quadrant_x = coord.x,
                        quadrant_y = coord.y,
                        negotiated_protocol_minor = negotiated,
                        required_protocol_minor = required,
                        "suppressed EnvironmentPresentationPolicy for protocol-minor-ineligible connection"
                    );
                }
                Err(error) => {
                    warn!(
                        connection_id = connection_id.0,
                        frontier_revision,
                        quadrant_x = coord.x,
                        quadrant_y = coord.y,
                        %error,
                        "revealed EnvironmentPresentationPolicy delivery failed; isolating affected connection"
                    );
                    sender.fail_close();
                    self.isolate_connection_after_reliable_delivery_failure(connection_id)?;
                    return Ok(());
                }
            }
            match water_surface {
                Some(water_surface) => {
                    let wet_samples = water_surface
                        .samples
                        .iter()
                        .copied()
                        .filter(|sample| sample.is_wet())
                        .count();

                    let water_payload = water_surface
                        .encode_wire()
                        .context("failed to encode revealed WaterSurfacePresentation contract")?;

                    match sender
                        .send_if_supported(
                            WATER_SURFACE_PRESENTATION_MIN_PROTOCOL_MINOR,
                            MessageKind::WaterSurfacePresentation,
                            &water_payload,
                        )
                        .await
                    {
                        Ok(ReliableCapabilitySendOutcome::Sent) => {
                            water_surfaces_sent += 1;

                            info!(
                                connection_id = connection_id.0,
                                frontier_revision,
                                quadrant_x = coord.x,
                                quadrant_y = coord.y,
                                terrain_generator_version = water_surface.terrain_generator_version,
                                hydrology_generator_version = water_surface.hydrology_generator_version,
                                grid_side = water_surface.control_grid_side,
                                samples = water_surface.samples.len(),
                                wet_samples,
                                payload_bytes = water_payload.len(),
                                "sent revealed WaterSurfacePresentation after EnvironmentPresentationPolicy"
                            );
                        }
                        Ok(ReliableCapabilitySendOutcome::UnsupportedProtocolMinor {
                            negotiated,
                            required,
                        }) => {
                            warn!(
                                connection_id = connection_id.0,
                                frontier_revision,
                                quadrant_x = coord.x,
                                quadrant_y = coord.y,
                                negotiated_protocol_minor = negotiated,
                                required_protocol_minor = required,
                                "WaterSurfacePresentation capability changed during eligible delivery; isolating affected connection"
                            );

                            sender.fail_close();

                            self.isolate_connection_after_reliable_delivery_failure(connection_id)?;

                            return Ok(());
                        }
                        Err(error) => {
                            warn!(
                                connection_id = connection_id.0,
                                frontier_revision,
                                quadrant_x = coord.x,
                                quadrant_y = coord.y,
                                %error,
                                "revealed WaterSurfacePresentation delivery failed; isolating affected connection"
                            );

                            sender.fail_close();

                            self.isolate_connection_after_reliable_delivery_failure(connection_id)?;

                            return Ok(());
                        }
                    }
                }
                None => {
                    water_surfaces_suppressed += 1;

                    debug!(
                        connection_id = connection_id.0,
                        frontier_revision,
                        quadrant_x = coord.x,
                        quadrant_y = coord.y,
                        negotiated_protocol_minor = sender.negotiated_protocol_minor(),
                        required_protocol_minor = WATER_SURFACE_PRESENTATION_MIN_PROTOCOL_MINOR,
                        "suppressed WaterSurfacePresentation for protocol-minor-ineligible connection"
                    );
                }
            }
        }
        info!(
            connection_id = connection_id.0,
            frontier_revision,
            terrain_contracts_sent = terrain_sent,
            environment_policies_sent = policies_sent,
            environment_policies_suppressed = policies_suppressed,
            water_surfaces_sent,
            water_surfaces_suppressed,
            negotiated_protocol_minor = sender.negotiated_protocol_minor(),
            "revealed presentation contract synchronization completed"
        );
        Ok(())
    }

    fn route_authorized_intent(&self, intent: AuthorizedIntent) -> anyhow::Result<()> {
        if !self.ingress.is_world_active_session(intent.session_id()) {
            debug!(
                session_id = intent.session_id().0,
                character_id = intent.character_id().0,
                zone_id = intent.zone_id().0,
                sequence = intent.sequence().0,
                "dropping queued authorized intent from inactive session"
            );
            return Ok(());
        }
        let Some(connection_id) = self.connection_by_session.get(&intent.session_id()).copied() else {
            debug!(
                session_id = intent.session_id().0,
                sequence = intent.sequence().0,
                "dropping queued authorized intent without live connection mapping"
            );
            return Ok(());
        };
        if !self.frontier_sync.is_synced(connection_id) {
            debug!(
                connection_id = connection_id.0,
                session_id = intent.session_id().0,
                sequence = intent.sequence().0,
                frontier_sync = ?self.frontier_sync.state(connection_id),
                "dropping queued authorized intent while frontier synchronization is pending"
            );
            return Ok(());
        }

        let IntentPayload::Move { axis_x, axis_y } = intent.payload();
        match self.world_router.route_authorized(intent)? {
            WorldRouteOutcome::Routed => {
                if self.development_auto_admit {
                    info!(
                        session_id = intent.session_id().0,
                        account_id = intent.account_id().0,
                        character_id = intent.character_id().0,
                        zone_id = intent.zone_id().0,
                        sequence = intent.sequence().0,
                        axis_x,
                        axis_y,
                        "server-authorized movement intent routed to zone single-writer"
                    );
                } else {
                    debug!(
                        session_id = intent.session_id().0,
                        character_id = intent.character_id().0,
                        zone_id = intent.zone_id().0,
                        sequence = intent.sequence().0,
                        "server-authorized movement intent routed to zone"
                    );
                }
            }
            WorldRouteOutcome::DroppedBackpressure => {
                debug!(
                    session_id = intent.session_id().0,
                    character_id = intent.character_id().0,
                    zone_id = intent.zone_id().0,
                    sequence = intent.sequence().0,
                    "replaceable movement intent dropped by zone command backpressure"
                );
            }
        }
        Ok(())
    }

    fn route_movement_snapshot(&self, snapshot: MovementSnapshot) -> anyhow::Result<()> {
        if let Some(ref buffer) = self.position_buffer {
            let character_id = snapshot.character_id();
            let pos = snapshot.position();
            let x = pos.x();
            let y = pos.y();
            let z = pos.z();
            
            // Tamanho padrão de quadrante em milímetros (512_000 mm conforme a fixture ou config)
            let q_size = 512_000_i64; 

            buffer.track_position(CharacterPositionUpdate {
                character_id: character_id.0 as i64,
                x_mm: x,
                y_mm: y,
                z_mm: z,
                quadrant_x: x.checked_div(q_size).unwrap_or(0),
                quadrant_y: y.checked_div(q_size).unwrap_or(0),
            });
        }
        if let Some(connection_id) = self
            .snapshot_bridge
            .connection_id_for_character(snapshot.character_id())
            && !self.frontier_sync.is_synced(connection_id)
        {
            debug!(
                connection_id = connection_id.0,
                character_id = snapshot.character_id().0,
                server_tick = snapshot.server_tick().0,
                frontier_sync = ?self.frontier_sync.state(connection_id),
                "dropping movement snapshot while frontier synchronization is pending"
            );
            return Ok(());
        }

        match self.snapshot_bridge.route(snapshot)? {
            SnapshotRouteOutcome::Sent => {
                debug!(
                    character_id = snapshot.character_id().0,
                    server_tick = snapshot.server_tick().0,
                    x_mm = snapshot.position().x(),
                    y_mm = snapshot.position().y(),
                    z_mm = snapshot.position().z(),
                    last_processed_input_sequence = ?snapshot.last_processed_input_sequence().map(|sequence| sequence.0),
                    active_movement_sequence = ?snapshot.active_movement_sequence().map(|sequence| sequence.0),
                    "authoritative self movement snapshot sent"
                );
            }
            SnapshotRouteOutcome::DroppedNoRoute => {
                debug!(
                    character_id = snapshot.character_id().0,
                    server_tick = snapshot.server_tick().0,
                    "dropping movement snapshot without live realtime route"
                );
            }
            SnapshotRouteOutcome::DroppedRealtime => {
                debug!(
                    character_id = snapshot.character_id().0,
                    server_tick = snapshot.server_tick().0,
                    "dropping replaceable movement snapshot due realtime backpressure or disconnect"
                );
            }
        }
        Ok(())
    }

    async fn auto_save_active_inventories(&self) {
        let Some(pg) = &self.postgres_client else {
            return;
        };

        for &connection_id in self.connection_by_session.values() {
            let Ok(Some(binding)) = self.ingress.world_binding_for_connection(connection_id) else {
                continue;
            };

            let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
            let responder = InventoryDespawnResponder::new(resp_tx);

            if self
                .world_router
                .snapshot_inventory(binding.zone_id(), binding.character_id(), responder)
                .is_ok()
            {
                if let Ok((inventory_state, revision)) = resp_rx.await {
                    let _ = aurenfall_persistence::InventoryRepository::save_inventory(
                        pg,
                        binding.character_id(),
                        &inventory_state,
                        revision,
                    )
                    .await;
                }
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_contracts::{
        EnvironmentPresentationPolicyV1, FrontierManifest, FrontierManifestAck, FrontierQuadrantCoord,
        MoveIntent, SelfMovementSnapshotV2, TerrainAuthorityContractV1, WaterSurfacePresentationV1,
    };
    use aurenfall_core::{ConnectionId, QuadrantCoord, ServerTick, SessionId, UniverseSeed};
    use aurenfall_domain::InitialFrontier;
    use aurenfall_preparation::PreparationWorkerSettings;
    use aurenfall_session::SessionPhase;
    use aurenfall_simulation::{MovementInput, ZoneCommand};
    use aurenfall_transport::{
        RealtimeConnectionSender, RealtimeDatagram, ReliableConnectionSender, ReliableControlFrame,
        ReliableSendError,
    };

    type AdmittedEventResult = anyhow::Result<(
        TransportSessionEvent,
        mpsc::Receiver<RealtimeDatagram>,
        mpsc::Receiver<ReliableControlFrame>,
    )>;

    fn dummy_responder() -> InventoryDespawnResponder {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        InventoryDespawnResponder::new(tx)
    }

    fn frontier_fixture() -> anyhow::Result<FrontierRuntime> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        FrontierRuntime::new(&initial, 1, 7, 512_000)
    }

    fn runtime_with_zone(
        development_auto_admit: bool,
    ) -> anyhow::Result<(SessionRuntime, mpsc::Receiver<ZoneCommand>)> {
        runtime_with_zone_and_frontier(development_auto_admit, None)
    }

    fn runtime_with_zone_and_frontier(
        development_auto_admit: bool,
        development_frontier_reveal: Option<DevelopmentFrontierRevealConfig>,
    ) -> anyhow::Result<(SessionRuntime, mpsc::Receiver<ZoneCommand>)> {
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (_snapshot_tx, snapshot_rx) = mpsc::channel(8);
        let (zone_tx, zone_rx) = mpsc::channel(8);
        let mut router = WorldRouter::default();
        router.register_zone(ZoneId(3), zone_tx)?;
        let settings =
            SessionRuntimeSettings::new(8, ZoneId(3), development_auto_admit, development_frontier_reveal);
        let runtime = SessionRuntime::new(transport_rx, snapshot_rx, settings, router, frontier_fixture()?)?;
        Ok((runtime, zone_rx))
    }

    fn preparation_scheduler_fixture() -> anyhow::Result<PreparationScheduler> {
        let worker = PreparationWorkerSettings::new(4, 4, 1)?;
        let settings = crate::preparation_scheduler::PreparationSchedulerSettings::new(
            Duration::from_millis(10),
            1,
            1,
            Duration::from_millis(20),
            2,
        )?;
        Ok(PreparationScheduler::spawn(
            worker,
            settings,
            UniverseSeed::from_phrase("session-runtime-policy-test"),
            512_000,
            7,
            vec![1, 2],
        ))
    }

    fn runtime_with_zone_and_scheduler() -> anyhow::Result<(SessionRuntime, mpsc::Receiver<ZoneCommand>)> {
        let (runtime, zone_rx) = runtime_with_zone(true)?;
        Ok((
            runtime.with_preparation_scheduler(preparation_scheduler_fixture()?),
            zone_rx,
        ))
    }

    fn admitted_event(connection_id: ConnectionId, session_id: SessionId) -> AdmittedEventResult {
        admitted_event_with_minor(connection_id, session_id, 0, 8)
    }

    fn admitted_event_with_minor(
        connection_id: ConnectionId,
        session_id: SessionId,
        negotiated_protocol_minor: u16,
        reliable_capacity: usize,
    ) -> AdmittedEventResult {
        let (realtime, datagrams) = RealtimeConnectionSender::bounded_channel(connection_id, 8)?;
        let (reliable, control_frames) = ReliableConnectionSender::bounded_channel(
            connection_id,
            reliable_capacity,
            4_096,
            Duration::from_secs(2),
        )?;
        let reliable = reliable.with_negotiated_protocol_minor(negotiated_protocol_minor);
        Ok((
            TransportSessionEvent::Admitted {
                connection_id,
                session_id,
                realtime,
                reliable,
            },
            datagrams,
            control_frames,
        ))
    }

    async fn acknowledge_frontier(
        runtime: &mut SessionRuntime,
        connection_id: ConnectionId,
        revision: u64,
    ) -> anyhow::Result<()> {
        runtime
            .handle_transport_event(TransportSessionEvent::FrontierManifestAck {
                connection_id,
                ack: FrontierManifestAck::new(revision)?,
            })
            .await
    }

    fn development_reveal_config() -> anyhow::Result<DevelopmentFrontierRevealConfig> {
        DevelopmentFrontierRevealConfig::new(QuadrantCoord::new(3, 0), Duration::from_secs(60))
    }

    #[tokio::test]
    async fn protocol_minor_zero_gets_terrain_without_environment_policy() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone_and_scheduler()?;
        let connection_id = ConnectionId(70);
        let session_id = SessionId(110);
        let (event, _datagrams, mut control) = admitted_event_with_minor(connection_id, session_id, 0, 64)?;
        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        let sender = runtime
            .reliable_by_connection
            .get(&connection_id)
            .cloned()
            .context("test reliable sender must exist")?;
        let manifest = runtime.frontier_runtime.manifest()?;

        acknowledge_frontier(&mut runtime, connection_id, manifest.revision).await?;

        let mut delivered = Vec::with_capacity(manifest.quadrants.len());
        for _ in 0..manifest.quadrants.len() {
            let frame = control.try_recv()?;
            assert_eq!(frame.kind, MessageKind::TerrainAuthority);
            let terrain = TerrainAuthorityContractV1::decode_wire(&frame.payload)?;
            delivered.push(terrain.quadrant_coord);
        }
        assert_eq!(delivered.len(), manifest.quadrants.len());
        for coord in &manifest.quadrants {
            assert!(delivered.contains(coord));
        }
        assert!(control.try_recv().is_err());

        sender.send(MessageKind::FrontierManifest, &[1]).await?;
        let frame = control.recv().await.context("connection should remain usable")?;
        assert_eq!(frame.kind, MessageKind::FrontierManifest);
        Ok(())
    }

    #[tokio::test]
    async fn protocol_minor_one_gets_matching_terrain_and_environment_policy() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone_and_scheduler()?;
        let connection_id = ConnectionId(71);
        let session_id = SessionId(111);
        let (event, _datagrams, mut control) = admitted_event_with_minor(connection_id, session_id, 1, 64)?;
        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        let manifest = runtime.frontier_runtime.manifest()?;

        acknowledge_frontier(&mut runtime, connection_id, manifest.revision).await?;

        for coord in &manifest.quadrants {
            let terrain_frame = control.try_recv()?;
            assert_eq!(terrain_frame.kind, MessageKind::TerrainAuthority);
            let terrain = TerrainAuthorityContractV1::decode_wire(&terrain_frame.payload)?;

            let policy_frame = control.try_recv()?;
            assert_eq!(policy_frame.kind, MessageKind::EnvironmentPresentationPolicy);
            let policy = EnvironmentPresentationPolicyV1::decode_wire(&policy_frame.payload)?;

            assert_eq!(terrain.quadrant_coord, *coord);
            assert_eq!(policy.quadrant_coord, terrain.quadrant_coord);
            assert_eq!(policy.terrain_generator_version, terrain.generator_version);
            assert_eq!(policy.control_grid_side, terrain.control_grid_side);
            assert_eq!(policy.cell_family_masks.len(), terrain.cell_flags.len());
        }
        assert!(control.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn protocol_minor_two_gets_matching_terrain_environment_policy_and_water_surface()
    -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone_and_scheduler()?;

        let connection_id = ConnectionId(76);
        let session_id = SessionId(116);

        let (event, _datagrams, mut control) = admitted_event_with_minor(connection_id, session_id, 2, 64)?;

        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;

        let manifest = runtime.frontier_runtime.manifest()?;

        acknowledge_frontier(&mut runtime, connection_id, manifest.revision).await?;

        for coord in &manifest.quadrants {
            let terrain_frame = control.try_recv()?;
            assert_eq!(terrain_frame.kind, MessageKind::TerrainAuthority);

            let terrain = TerrainAuthorityContractV1::decode_wire(&terrain_frame.payload)?;

            let policy_frame = control.try_recv()?;
            assert_eq!(policy_frame.kind, MessageKind::EnvironmentPresentationPolicy);

            let policy = EnvironmentPresentationPolicyV1::decode_wire(&policy_frame.payload)?;

            let water_frame = control.try_recv()?;
            assert_eq!(water_frame.kind, MessageKind::WaterSurfacePresentation);

            let water = WaterSurfacePresentationV1::decode_wire(&water_frame.payload)?;

            assert_eq!(terrain.quadrant_coord, *coord);
            assert_eq!(policy.quadrant_coord, terrain.quadrant_coord);
            assert_eq!(water.quadrant_coord, terrain.quadrant_coord);

            assert_eq!(policy.terrain_generator_version, terrain.generator_version);
            assert_eq!(water.terrain_generator_version, terrain.generator_version);
            assert_eq!(water.hydrology_generator_version, terrain.generator_version);

            assert_eq!(policy.control_grid_side, terrain.control_grid_side);
            assert_eq!(water.control_grid_side, terrain.control_grid_side);

            assert_eq!(policy.cell_family_masks.len(), terrain.cell_flags.len());
            assert_eq!(water.samples.len(), terrain.elevation_samples_mm.len());
        }

        assert!(control.try_recv().is_err());

        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delivery_uses_only_revealed_manifest_not_candidate_or_prepared() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone_and_scheduler()?;
        let private_coord = runtime
            .frontier_runtime
            .candidate_quadrants()
            .first()
            .copied()
            .context("frontier fixture must have a private candidate")?;

        for _ in 0..100 {
            runtime.evaluate_preparation_pipeline(Instant::now())?;
            if runtime.frontier_runtime.prepared_len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(runtime.frontier_runtime.prepared_len(), 1);
        let manifest = runtime.frontier_runtime.manifest()?;
        assert_eq!(manifest.quadrants.len(), 16);
        assert!(
            manifest
                .quadrants
                .iter()
                .all(|coord| coord.x != private_coord.x() || coord.y != private_coord.y())
        );

        let connection_id = ConnectionId(72);
        let session_id = SessionId(112);
        let (event, _datagrams, mut control) = admitted_event_with_minor(connection_id, session_id, 2, 64)?;
        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        acknowledge_frontier(&mut runtime, connection_id, manifest.revision).await?;

        let expected_frames = manifest.quadrants.len() * 3;
        for index in 0..expected_frames {
            let frame = control.try_recv()?;
            match frame.kind {
                MessageKind::TerrainAuthority => {
                    let terrain = TerrainAuthorityContractV1::decode_wire(&frame.payload)?;
                    assert!(
                        terrain.quadrant_coord.x != private_coord.x()
                            || terrain.quadrant_coord.y != private_coord.y()
                    );
                }
                MessageKind::EnvironmentPresentationPolicy => {
                    let policy = EnvironmentPresentationPolicyV1::decode_wire(&frame.payload)?;
                    assert!(
                        policy.quadrant_coord.x != private_coord.x()
                            || policy.quadrant_coord.y != private_coord.y()
                    );
                }
                MessageKind::WaterSurfacePresentation => {
                    let water = WaterSurfacePresentationV1::decode_wire(&frame.payload)?;
                    assert!(
                        water.quadrant_coord.x != private_coord.x()
                            || water.quadrant_coord.y != private_coord.y()
                    );
                }
                other => anyhow::bail!("unexpected public presentation frame at index {index}: {other:?}"),
            }
        }
        assert!(control.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn presentation_delivery_failure_isolates_connection_and_preserves_other_session()
    -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone_and_scheduler()?;
        let manifest = runtime.frontier_runtime.manifest()?;

        let failed_connection = ConnectionId(73);
        let failed_session = SessionId(113);
        let (failed_event, _failed_datagrams, failed_control) =
            admitted_event_with_minor(failed_connection, failed_session, 1, 64)?;
        runtime.handle_transport_event(failed_event).await?;
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::SpawnCharacter {
                character_id: CharacterId(failed_session.0),
                position: WorldPositionMm::ORIGIN,
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 0,
            }
        );

        let failed_sender = runtime
            .reliable_by_connection
            .get(&failed_connection)
            .cloned()
            .context("failed-connection reliable sender must exist")?;

        let healthy_connection = ConnectionId(74);
        let healthy_session = SessionId(114);
        let (healthy_event, _healthy_datagrams, mut healthy_control) =
            admitted_event_with_minor(healthy_connection, healthy_session, 1, 64)?;
        runtime.handle_transport_event(healthy_event).await?;
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::SpawnCharacter {
                character_id: CharacterId(healthy_session.0),
                position: WorldPositionMm::ORIGIN,
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 0,
            }
        );

        // Force connection A reliable delivery to fail.
        drop(failed_control);

        // A transport failure must not escape the ACK handler.
        acknowledge_frontier(&mut runtime, failed_connection, manifest.revision).await?;

        // The reliable capability itself must be fail-closed.
        assert!(matches!(
            failed_sender.send(MessageKind::FrontierManifest, &[1]).await,
            Err(ReliableSendError::FailClosed {
                connection_id: ConnectionId(73),
            })
        ));

        // More importantly, failed presentation sync must revoke gameplay
        // immediately rather than waiting for the later Disconnected event.
        assert!(!runtime.frontier_sync.is_synced(failed_connection));
        assert_eq!(
            runtime.ingress.phase_for_connection(failed_connection)?,
            SessionPhase::Closing
        );
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::DespawnCharacter {
                character_id: CharacterId(failed_session.0),
                responder: None,
            }
        );

        runtime
            .handle_transport_event(TransportSessionEvent::MoveIntent {
                connection_id: failed_connection,
                intent: MoveIntent::new(1, 16_384, 0)?,
            })
            .await?;
        assert!(runtime.authorized_rx.try_recv().is_err());

        // Connection B must remain completely healthy.
        acknowledge_frontier(&mut runtime, healthy_connection, manifest.revision).await?;
        assert!(runtime.frontier_sync.is_synced(healthy_connection));
        assert_eq!(
            runtime.ingress.phase_for_connection(healthy_connection)?,
            SessionPhase::WorldActive
        );

        for coord in &manifest.quadrants {
            let terrain_frame = healthy_control.try_recv()?;
            assert_eq!(terrain_frame.kind, MessageKind::TerrainAuthority);
            let terrain = TerrainAuthorityContractV1::decode_wire(&terrain_frame.payload)?;

            let policy_frame = healthy_control.try_recv()?;
            assert_eq!(policy_frame.kind, MessageKind::EnvironmentPresentationPolicy);
            let policy = EnvironmentPresentationPolicyV1::decode_wire(&policy_frame.payload)?;

            assert_eq!(terrain.quadrant_coord, *coord);
            assert_eq!(policy.quadrant_coord, terrain.quadrant_coord);
            assert_eq!(policy.terrain_generator_version, terrain.generator_version);
            assert_eq!(policy.control_grid_side, terrain.control_grid_side);
            assert_eq!(policy.cell_family_masks.len(), terrain.cell_flags.len());
        }

        assert!(healthy_control.try_recv().is_err());
        Ok(())
    }
    #[tokio::test]
    async fn water_surface_delivery_failure_isolates_only_affected_minor_two_connection() -> anyhow::Result<()>
    {
        let (mut runtime, mut zone_rx) = runtime_with_zone_and_scheduler()?;

        let manifest = runtime.frontier_runtime.manifest()?;

        // Connection A is minor 2 but has room for exactly 205 + 206.
        // The first 207 therefore fails deterministically by backpressure timeout.
        let failed_connection = ConnectionId(77);
        let failed_session = SessionId(117);

        let (failed_event, _failed_datagrams, mut failed_control) =
            admitted_event_with_minor(failed_connection, failed_session, 2, 2)?;

        runtime.handle_transport_event(failed_event).await?;

        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::SpawnCharacter {
                character_id: CharacterId(failed_session.0),
                position: WorldPositionMm::ORIGIN,
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 0,
            }
        );

        let failed_sender = runtime
            .reliable_by_connection
            .get(&failed_connection)
            .cloned()
            .context("failed minor-2 reliable sender must exist")?;

        // Connection B has enough capacity to complete the entire 205/206/207 sync.
        let healthy_connection = ConnectionId(78);
        let healthy_session = SessionId(118);

        let (healthy_event, _healthy_datagrams, mut healthy_control) =
            admitted_event_with_minor(healthy_connection, healthy_session, 2, 64)?;

        runtime.handle_transport_event(healthy_event).await?;

        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::SpawnCharacter {
                character_id: CharacterId(healthy_session.0),
                position: WorldPositionMm::ORIGIN,
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 0,
            }
        );

        // This ACK must not escape as a SessionRuntime error.
        // 205 and 206 enqueue; 207 hits reliable backpressure timeout.
        acknowledge_frontier(&mut runtime, failed_connection, manifest.revision).await?;

        // Prove the failure happened after 205 + 206 and before 207.
        let failed_terrain_frame = failed_control.try_recv()?;
        assert_eq!(failed_terrain_frame.kind, MessageKind::TerrainAuthority);
        let failed_terrain = TerrainAuthorityContractV1::decode_wire(&failed_terrain_frame.payload)?;

        let failed_policy_frame = failed_control.try_recv()?;
        assert_eq!(
            failed_policy_frame.kind,
            MessageKind::EnvironmentPresentationPolicy
        );
        let failed_policy = EnvironmentPresentationPolicyV1::decode_wire(&failed_policy_frame.payload)?;

        assert_eq!(failed_terrain.quadrant_coord, manifest.quadrants[0]);
        assert_eq!(failed_policy.quadrant_coord, failed_terrain.quadrant_coord);

        // No 207 was successfully enqueued.
        assert!(failed_control.try_recv().is_err());

        // Reliable capability is now fail-closed.
        assert!(matches!(
            failed_sender.send(MessageKind::FrontierManifest, &[1]).await,
            Err(ReliableSendError::FailClosed {
                connection_id: ConnectionId(77),
            })
        ));

        // Gameplay and presentation routes for A are revoked immediately.
        assert!(!runtime.frontier_sync.is_synced(failed_connection));

        assert_eq!(
            runtime.ingress.phase_for_connection(failed_connection)?,
            SessionPhase::Closing
        );

        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::DespawnCharacter {
                character_id: CharacterId(failed_session.0),
                responder: None,
            }
        );

        assert!(!runtime.reliable_by_connection.contains_key(&failed_connection));

        // A cannot submit gameplay after presentation failure.
        runtime
            .handle_transport_event(TransportSessionEvent::MoveIntent {
                connection_id: failed_connection,
                intent: MoveIntent::new(1, 16_384, 0)?,
            })
            .await?;

        assert!(runtime.authorized_rx.try_recv().is_err());

        // B must remain fully healthy.
        acknowledge_frontier(&mut runtime, healthy_connection, manifest.revision).await?;

        assert!(runtime.frontier_sync.is_synced(healthy_connection));

        assert_eq!(
            runtime.ingress.phase_for_connection(healthy_connection)?,
            SessionPhase::WorldActive
        );

        for coord in &manifest.quadrants {
            let terrain_frame = healthy_control.try_recv()?;
            assert_eq!(terrain_frame.kind, MessageKind::TerrainAuthority);
            let terrain = TerrainAuthorityContractV1::decode_wire(&terrain_frame.payload)?;

            let policy_frame = healthy_control.try_recv()?;
            assert_eq!(policy_frame.kind, MessageKind::EnvironmentPresentationPolicy);
            let policy = EnvironmentPresentationPolicyV1::decode_wire(&policy_frame.payload)?;

            let water_frame = healthy_control.try_recv()?;
            assert_eq!(water_frame.kind, MessageKind::WaterSurfacePresentation);
            let water = WaterSurfacePresentationV1::decode_wire(&water_frame.payload)?;

            assert_eq!(terrain.quadrant_coord, *coord);
            assert_eq!(policy.quadrant_coord, terrain.quadrant_coord);
            assert_eq!(water.quadrant_coord, terrain.quadrant_coord);

            assert_eq!(water.terrain_generator_version, terrain.generator_version);

            assert_eq!(water.control_grid_side, terrain.control_grid_side);

            assert_eq!(water.samples.len(), terrain.elevation_samples_mm.len());
        }

        assert!(healthy_control.try_recv().is_err());

        // The later transport disconnect for A is final cleanup only.
        runtime
            .handle_transport_event(TransportSessionEvent::Disconnected {
                connection_id: failed_connection,
                session_id: failed_session,
            })
            .await?;

        assert!(runtime.ingress.phase_for_connection(failed_connection).is_err());

        assert!(!runtime.connection_by_session.contains_key(&failed_session));

        // Isolation already despawned A. No second despawn is allowed.
        assert!(zone_rx.try_recv().is_err());

        // B is still live after all cleanup for A.
        assert_eq!(
            runtime.ingress.phase_for_connection(healthy_connection)?,
            SessionPhase::WorldActive
        );

        Ok(())
    }
    #[tokio::test]
    async fn disconnected_after_delivery_isolation_completes_cleanup_without_second_despawn()
    -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone_and_scheduler()?;
        let manifest = runtime.frontier_runtime.manifest()?;

        let connection_id = ConnectionId(75);
        let session_id = SessionId(115);
        let (event, _datagrams, control) = admitted_event_with_minor(connection_id, session_id, 1, 64)?;

        runtime.handle_transport_event(event).await?;

        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::SpawnCharacter {
                character_id: CharacterId(session_id.0),
                position: WorldPositionMm::ORIGIN,
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 0,
            }
        );

        // Force reliable presentation delivery failure.
        drop(control);

        acknowledge_frontier(&mut runtime, connection_id, manifest.revision).await?;

        // Isolation must already have revoked gameplay and despawned once.
        assert_eq!(
            runtime.ingress.phase_for_connection(connection_id)?,
            SessionPhase::Closing
        );
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::DespawnCharacter {
                character_id: CharacterId(session_id.0),
                responder: None,
            }
        );
        assert!(zone_rx.try_recv().is_err());

        // The transport may report Disconnected later. This is final cleanup,
        // not a second gameplay teardown.
        runtime
            .handle_transport_event(TransportSessionEvent::Disconnected {
                connection_id,
                session_id,
            })
            .await?;

        assert!(runtime.ingress.phase_for_connection(connection_id).is_err());
        assert!(!runtime.reliable_by_connection.contains_key(&connection_id));
        assert!(!runtime.connection_by_session.contains_key(&session_id));
        assert!(!runtime.frontier_sync.is_synced(connection_id));

        // No second despawn was emitted.
        assert!(zone_rx.try_recv().is_err());

        Ok(())
    }
    #[tokio::test]
    async fn development_session_lifecycle_gates_gameplay_until_frontier_ack() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone(true)?;
        let connection_id = ConnectionId(7);
        let session_id = SessionId(11);
        let (event, mut datagrams, _control_frames) = admitted_event(connection_id, session_id)?;

        runtime.handle_transport_event(event).await?;
        assert!(runtime.reliable_by_connection.contains_key(&connection_id));
        assert!(!runtime.frontier_sync.is_synced(connection_id));
        assert_eq!(
            runtime.ingress.phase_for_connection(connection_id)?,
            SessionPhase::WorldActive
        );
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::SpawnCharacter {
                character_id: CharacterId(session_id.0),
                position: WorldPositionMm::ORIGIN,
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 0,
            }
        );

        runtime
            .handle_transport_event(TransportSessionEvent::MoveIntent {
                connection_id,
                intent: MoveIntent::new(1, 16_384, 0)?,
            })
            .await?;
        assert!(runtime.authorized_rx.try_recv().is_err());

        runtime.route_movement_snapshot(MovementSnapshot::new(
            CharacterId(session_id.0),
            ServerTick(54),
            WorldPositionMm::new(300, 0, 0),
            None,
            None,
        ))?;
        assert!(datagrams.try_recv().is_err());

        acknowledge_frontier(&mut runtime, connection_id, 1).await?;
        assert!(runtime.frontier_sync.is_synced(connection_id));

        let move_intent = MoveIntent::new(1, 16_384, 0)?;
        runtime
            .handle_transport_event(TransportSessionEvent::MoveIntent {
                connection_id,
                intent: move_intent,
            })
            .await?;

        let authorized = runtime.authorized_rx.try_recv()?;
        runtime.route_authorized_intent(authorized)?;
        let input = MovementInput::from_axes(16_384, 0).context("test movement input must be valid")?;
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::MoveIntent {
                character_id: CharacterId(session_id.0),
                sequence: 1,
                input,
            }
        );

        runtime.route_movement_snapshot(MovementSnapshot::new(
            CharacterId(session_id.0),
            ServerTick(55),
            WorldPositionMm::new(400, 0, 0),
            Some(IntentSequence(1)),
            Some(IntentSequence(1)),
        ))?;
        let datagram = datagrams.try_recv()?;
        assert_eq!(datagram.kind, MessageKind::SelfMovementSnapshotV2);
        let snapshot = SelfMovementSnapshotV2::decode_wire(&datagram.payload)?;
        assert_eq!(snapshot.server_tick, 55);
        assert_eq!(snapshot.x_mm, 400);
        assert_eq!(snapshot.last_processed_input_sequence, Some(1));
        assert_eq!(snapshot.active_movement_sequence, Some(1));

        runtime
            .handle_transport_event(TransportSessionEvent::Disconnected {
                connection_id,
                session_id,
            })
            .await?;
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::DespawnCharacter {
                character_id: CharacterId(session_id.0),
                responder: Some(dummy_responder()),
            }
        );
        assert!(!runtime.reliable_by_connection.contains_key(&connection_id));
        assert_eq!(runtime.frontier_sync.state(connection_id), None);
        assert_eq!(runtime.ingress.live_session_count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn disconnect_invalidates_routes() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone(true)?;
        let connection_id = ConnectionId(7);
        let session_id = SessionId(11);
        let (event, _datagrams, _control_frames) = admitted_event(connection_id, session_id)?;

        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        acknowledge_frontier(&mut runtime, connection_id, 1).await?;
        runtime
            .handle_transport_event(TransportSessionEvent::MoveIntent {
                connection_id,
                intent: MoveIntent::new(1, 1, 0)?,
            })
            .await?;
        let authorized = runtime.authorized_rx.try_recv()?;

        runtime
            .handle_transport_event(TransportSessionEvent::Disconnected {
                connection_id,
                session_id,
            })
            .await?;
        runtime.route_authorized_intent(authorized)?;
        runtime.route_movement_snapshot(MovementSnapshot::new(
            CharacterId(session_id.0),
            ServerTick(1),
            WorldPositionMm::ORIGIN,
            None,
            None,
        ))?;

        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::DespawnCharacter {
                character_id: CharacterId(session_id.0),
                responder: Some(dummy_responder()),
            }
        );
        assert!(zone_rx.try_recv().is_err());
        assert!(!runtime.reliable_by_connection.contains_key(&connection_id));
        Ok(())
    }

    #[tokio::test]
    async fn non_development_session_stays_protocol_only_and_pending() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone(false)?;
        let connection_id = ConnectionId(7);
        let (event, _datagrams, _control_frames) = admitted_event(connection_id, SessionId(11))?;

        runtime.handle_transport_event(event).await?;

        assert!(runtime.reliable_by_connection.contains_key(&connection_id));
        assert!(!runtime.frontier_sync.is_synced(connection_id));
        assert_eq!(
            runtime.ingress.phase_for_connection(connection_id)?,
            SessionPhase::ProtocolAdmitted
        );
        assert!(zone_rx.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn movement_does_not_reveal_frontier() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) =
            runtime_with_zone_and_frontier(true, Some(development_reveal_config()?))?;
        let connection_id = ConnectionId(7);
        let (event, _datagrams, mut control_frames) = admitted_event(connection_id, SessionId(11))?;

        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        acknowledge_frontier(&mut runtime, connection_id, 1).await?;
        runtime
            .handle_transport_event(TransportSessionEvent::MoveIntent {
                connection_id,
                intent: MoveIntent::new(1, 32_000, 0)?,
            })
            .await?;
        let authorized = runtime.authorized_rx.try_recv()?;
        runtime.route_authorized_intent(authorized)?;

        assert_eq!(runtime.frontier_runtime.revision(), 1);
        assert_eq!(runtime.frontier_runtime.revealed_len(), 16);
        assert!(control_frames.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn reveal_returns_sessions_to_pending_and_gates_queued_gameplay() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) =
            runtime_with_zone_and_frontier(true, Some(development_reveal_config()?))?;
        let connection_id = ConnectionId(7);
        let session_id = SessionId(11);
        let (event, mut datagrams, mut control) = admitted_event(connection_id, session_id)?;
        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        acknowledge_frontier(&mut runtime, connection_id, 1).await?;

        runtime
            .handle_transport_event(TransportSessionEvent::MoveIntent {
                connection_id,
                intent: MoveIntent::new(1, 10_000, 0)?,
            })
            .await?;
        let queued = runtime.authorized_rx.try_recv()?;

        runtime.commit_development_frontier_reveal(true).await?;
        assert_eq!(runtime.frontier_runtime.revision(), 2);
        assert!(!runtime.frontier_sync.is_synced(connection_id));

        runtime.route_authorized_intent(queued)?;
        assert!(zone_rx.try_recv().is_err());
        runtime.route_movement_snapshot(MovementSnapshot::new(
            CharacterId(session_id.0),
            ServerTick(10),
            WorldPositionMm::new(100, 0, 0),
            None,
            None,
        ))?;
        assert!(datagrams.try_recv().is_err());

        let frame = control.try_recv()?;
        assert_eq!(frame.kind, MessageKind::FrontierManifest);
        let manifest = FrontierManifest::decode(&frame.payload)?;
        assert_eq!(manifest.revision, 2);

        acknowledge_frontier(&mut runtime, connection_id, 1).await?;
        assert!(!runtime.frontier_sync.is_synced(connection_id));
        acknowledge_frontier(&mut runtime, connection_id, 2).await?;
        assert!(runtime.frontier_sync.is_synced(connection_id));
        Ok(())
    }

    #[tokio::test]
    async fn future_ack_fail_closes_reliable_connection() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone(true)?;
        let connection_id = ConnectionId(7);
        let (event, _datagrams, _control) = admitted_event(connection_id, SessionId(11))?;
        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        let sender = runtime
            .reliable_by_connection
            .get(&connection_id)
            .cloned()
            .context("test reliable sender must exist")?;

        acknowledge_frontier(&mut runtime, connection_id, 2).await?;
        assert!(matches!(
            sender.send(MessageKind::FrontierManifest, &[1]).await,
            Err(ReliableSendError::FailClosed { connection_id: id }) if id == connection_id
        ));
        assert!(!runtime.frontier_sync.is_synced(connection_id));
        Ok(())
    }

    #[tokio::test]
    async fn server_requirement_reveals_once_and_broadcasts() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) =
            runtime_with_zone_and_frontier(true, Some(development_reveal_config()?))?;
        let first_id = ConnectionId(7);
        let second_id = ConnectionId(8);
        let (first_event, _first_datagrams, mut first_control) = admitted_event(first_id, SessionId(11))?;
        let (second_event, _second_datagrams, mut second_control) = admitted_event(second_id, SessionId(12))?;
        runtime.handle_transport_event(first_event).await?;
        runtime.handle_transport_event(second_event).await?;
        let _first_spawn = zone_rx.try_recv()?;
        let _second_spawn = zone_rx.try_recv()?;
        acknowledge_frontier(&mut runtime, first_id, 1).await?;
        acknowledge_frontier(&mut runtime, second_id, 1).await?;

        runtime.commit_development_frontier_reveal(false).await?;
        assert_eq!(runtime.frontier_runtime.revision(), 1);
        assert!(first_control.try_recv().is_err());
        assert!(second_control.try_recv().is_err());

        runtime.commit_development_frontier_reveal(true).await?;
        assert_eq!(runtime.frontier_runtime.revision(), 2);
        assert_eq!(runtime.frontier_runtime.revealed_len(), 17);
        assert!(!runtime.frontier_sync.is_synced(first_id));
        assert!(!runtime.frontier_sync.is_synced(second_id));

        for receiver in [&mut first_control, &mut second_control] {
            let frame = receiver.try_recv()?;
            assert_eq!(frame.kind, MessageKind::FrontierManifest);
            let manifest = FrontierManifest::decode(&frame.payload)?;
            assert_eq!(manifest.revision, 2);
            assert_eq!(manifest.quadrants.len(), 17);
            assert!(manifest.quadrants.contains(&FrontierQuadrantCoord::new(3, 0)));
        }

        acknowledge_frontier(&mut runtime, first_id, 2).await?;
        acknowledge_frontier(&mut runtime, second_id, 2).await?;
        runtime.commit_development_frontier_reveal(true).await?;
        assert_eq!(runtime.frontier_runtime.revision(), 2);
        assert!(first_control.try_recv().is_err());
        assert!(second_control.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn late_admission_requires_current_revision_ack() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) =
            runtime_with_zone_and_frontier(true, Some(development_reveal_config()?))?;
        let first_id = ConnectionId(7);
        let (first_event, _first_datagrams, mut first_control) = admitted_event(first_id, SessionId(11))?;
        runtime.handle_transport_event(first_event).await?;
        let _first_spawn = zone_rx.try_recv()?;
        acknowledge_frontier(&mut runtime, first_id, 1).await?;
        runtime.commit_development_frontier_reveal(true).await?;
        let _first_update = first_control.try_recv()?;
        acknowledge_frontier(&mut runtime, first_id, 2).await?;

        let late_id = ConnectionId(8);
        let (late_event, _late_datagrams, mut late_control) = admitted_event(late_id, SessionId(12))?;
        runtime.handle_transport_event(late_event).await?;
        let _late_spawn = zone_rx.try_recv()?;
        assert!(!runtime.frontier_sync.is_synced(late_id));

        acknowledge_frontier(&mut runtime, late_id, 1).await?;
        assert!(!runtime.frontier_sync.is_synced(late_id));

        let frame = late_control.try_recv()?;
        assert_eq!(frame.kind, MessageKind::FrontierManifest);
        let manifest = FrontierManifest::decode(&frame.payload)?;
        assert_eq!(manifest.revision, 2);
        assert_eq!(manifest.quadrants.len(), 17);

        acknowledge_frontier(&mut runtime, late_id, 2).await?;
        assert!(runtime.frontier_sync.is_synced(late_id));
        Ok(())
    }

    #[tokio::test]
    async fn pending_frontier_retries_canonical_full_manifest() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone(true)?;
        let connection_id = ConnectionId(7);
        let (event, _datagrams, mut control) = admitted_event(connection_id, SessionId(11))?;
        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;

        runtime
            .evaluate_frontier_delivery(Instant::now() + Duration::from_secs(1))
            .await?;

        let frame = control.try_recv()?;
        assert_eq!(frame.kind, MessageKind::FrontierManifest);
        let manifest = FrontierManifest::decode(&frame.payload)?;
        assert_eq!(manifest.revision, 1);
        assert_eq!(manifest.quadrants.len(), 16);
        assert!(!runtime.frontier_sync.is_synced(connection_id));
        Ok(())
    }

    #[tokio::test]
    async fn frontier_ack_timeout_fail_closes_pending_connection() -> anyhow::Result<()> {
        let (mut runtime, mut zone_rx) = runtime_with_zone(true)?;
        let connection_id = ConnectionId(7);
        let (event, _datagrams, _control) = admitted_event(connection_id, SessionId(11))?;
        runtime.handle_transport_event(event).await?;
        let _spawn = zone_rx.try_recv()?;
        let sender = runtime
            .reliable_by_connection
            .get(&connection_id)
            .cloned()
            .context("test reliable sender must exist")?;

        runtime
            .evaluate_frontier_delivery(Instant::now() + Duration::from_secs(10))
            .await?;

        assert!(matches!(
            sender.send(MessageKind::FrontierManifest, &[1]).await,
            Err(ReliableSendError::FailClosed { connection_id: id }) if id == connection_id
        ));
        assert!(!runtime.frontier_sync.is_synced(connection_id));
        Ok(())
    }
}
