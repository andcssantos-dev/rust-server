use std::{fs, path::Path};

use anyhow::{Context, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub server: ServerSection,
    pub universe: UniverseSection,
    pub simulation: SimulationSection,
    pub preparation: PreparationSection,
    pub network: NetworkSection,
    pub persistence: PersistenceSection,
    pub cache: CacheSection,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSection {
    pub environment: String,
    pub instance_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UniverseSection {
    pub id: u64,
    pub seed: String,
    pub generator_version: u32,
    pub quadrant_size_mm: i64,
    pub initial_frontier: InitialFrontierSection,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InitialFrontierSection {
    pub min_x: i64,
    pub min_y: i64,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SimulationSection {
    pub tick_rate_hz: u32,
    pub zone_command_capacity: usize,
    pub movement_snapshot_capacity: usize,
    pub sector_size_meters: u32,
    pub correction_absorb_max_mm: u32,
    pub correction_smooth_max_mm: u32,
    pub correction_hard_snap_threshold_mm: u32,
    pub correction_smooth_duration_ms: u32,
    pub correction_rapid_duration_ms: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PreparationSection {
    pub request_queue_capacity: usize,
    pub result_queue_capacity: usize,
    pub max_in_flight: usize,
    pub scheduler_interval_ms: u64,
    pub max_submissions_per_tick: usize,
    pub max_prepared_quadrants: usize,
    pub retry_backoff_ms: u64,
    pub max_retry_attempts: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkSection {
    pub bind_address: String,
    pub certificate_der_path: String,
    pub private_key_der_path: String,
    pub max_connections: usize,
    pub session_event_capacity: usize,
    pub frontier_ack_retry_interval_ms: u64,
    pub frontier_ack_timeout_ms: u64,
    pub frontier_ack_max_retries: u32,
    pub max_datagram_bytes: usize,
    pub max_control_frame_bytes: usize,
    pub max_bidi_streams: u32,
    pub max_uni_streams: u32,
    pub handshake_timeout_ms: u64,
    pub rejection_close_grace_ms: u64,
    pub idle_timeout_ms: u64,
    pub datagram_receive_buffer_bytes: usize,
    pub datagram_send_buffer_bytes: usize,
    pub minimum_client_build: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PersistenceSection {
    pub enabled: bool,
    pub connection_string: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheSection {
    pub character_capacity: usize,
    pub gamedata_versions: usize,
}

impl ServerConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let source =
            fs::read_to_string(path).with_context(|| format!("unable to read {}", path.display()))?;
        toml::from_str(&source).context("invalid server TOML configuration")
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.universe.seed.trim().is_empty() {
            bail!("universe.seed cannot be empty");
        }
        if self.universe.quadrant_size_mm <= 0 {
            bail!("universe.quadrant_size_mm must be > 0");
        }
        if self.universe.initial_frontier.width == 0 {
            bail!("universe.initial_frontier.width must be > 0");
        }
        if self.universe.initial_frontier.height == 0 {
            bail!("universe.initial_frontier.height must be > 0");
        }
        if !(1..=240).contains(&self.simulation.tick_rate_hz) {
            bail!("simulation.tick_rate_hz must be in 1..=240");
        }
        if self.simulation.zone_command_capacity == 0 {
            bail!("simulation.zone_command_capacity must be > 0");
        }
        if self.simulation.movement_snapshot_capacity == 0 {
            bail!("simulation.movement_snapshot_capacity must be > 0");
        }
        if self.simulation.correction_absorb_max_mm >= self.simulation.correction_smooth_max_mm
            || self.simulation.correction_smooth_max_mm >= self.simulation.correction_hard_snap_threshold_mm
        {
            bail!("simulation correction thresholds must satisfy absorb < smooth < hard snap");
        }
        if self.simulation.correction_smooth_duration_ms == 0 {
            bail!("simulation.correction_smooth_duration_ms must be > 0");
        }
        if self.simulation.correction_rapid_duration_ms == 0 {
            bail!("simulation.correction_rapid_duration_ms must be > 0");
        }
        if self.preparation.request_queue_capacity == 0 {
            bail!("preparation.request_queue_capacity must be > 0");
        }
        if self.preparation.result_queue_capacity == 0 {
            bail!("preparation.result_queue_capacity must be > 0");
        }
        if self.preparation.max_in_flight == 0 {
            bail!("preparation.max_in_flight must be > 0");
        }
        if !(10..=60_000).contains(&self.preparation.scheduler_interval_ms) {
            bail!("preparation.scheduler_interval_ms must be in 10..=60000");
        }
        if self.preparation.max_submissions_per_tick == 0 {
            bail!("preparation.max_submissions_per_tick must be > 0");
        }
        if self.preparation.max_prepared_quadrants == 0 {
            bail!("preparation.max_prepared_quadrants must be > 0");
        }
        if !(10..=300_000).contains(&self.preparation.retry_backoff_ms) {
            bail!("preparation.retry_backoff_ms must be in 10..=300000");
        }
        if !(1..=32).contains(&self.preparation.max_retry_attempts) {
            bail!("preparation.max_retry_attempts must be in 1..=32");
        }
        if self.network.bind_address.trim().is_empty() {
            bail!("network.bind_address cannot be empty");
        }
        if self.network.certificate_der_path.trim().is_empty()
            || self.network.private_key_der_path.trim().is_empty()
        {
            bail!("network TLS certificate/key paths cannot be empty");
        }
        if self.network.max_connections == 0 {
            bail!("network.max_connections must be > 0");
        }
        if self.network.session_event_capacity == 0 {
            bail!("network.session_event_capacity must be > 0");
        }
        if !(100..=60_000).contains(&self.network.frontier_ack_retry_interval_ms) {
            bail!("network.frontier_ack_retry_interval_ms must be in 100..=60000");
        }
        if !(500..=300_000).contains(&self.network.frontier_ack_timeout_ms) {
            bail!("network.frontier_ack_timeout_ms must be in 500..=300000");
        }
        if self.network.frontier_ack_timeout_ms <= self.network.frontier_ack_retry_interval_ms {
            bail!("network.frontier_ack_timeout_ms must be greater than frontier_ack_retry_interval_ms");
        }
        if !(1..=32).contains(&self.network.frontier_ack_max_retries) {
            bail!("network.frontier_ack_max_retries must be in 1..=32");
        }
        if self.network.max_datagram_bytes < 512 || self.network.max_datagram_bytes > 65_507 {
            bail!("network.max_datagram_bytes is outside the supported UDP payload range");
        }
        if !(64..=65_535).contains(&self.network.max_control_frame_bytes) {
            bail!("network.max_control_frame_bytes must be in 64..=65535");
        }
        if self.network.max_bidi_streams == 0 {
            bail!("network.max_bidi_streams must be > 0 for the bootstrap stream");
        }
        if !(100..=300_000).contains(&self.network.handshake_timeout_ms) {
            bail!("network.handshake_timeout_ms must be in 100..=300000");
        }
        if !(100..=10_000).contains(&self.network.rejection_close_grace_ms) {
            bail!("network.rejection_close_grace_ms must be in 100..=10000");
        }
        if !(1_000..=3_600_000).contains(&self.network.idle_timeout_ms) {
            bail!("network.idle_timeout_ms must be in 1000..=3600000");
        }
        if self.network.datagram_receive_buffer_bytes < self.network.max_datagram_bytes {
            bail!("network.datagram_receive_buffer_bytes must fit at least one datagram");
        }
        if self.network.datagram_send_buffer_bytes < self.network.max_datagram_bytes {
            bail!("network.datagram_send_buffer_bytes must fit at least one datagram");
        }
        Ok(())
    }
}
