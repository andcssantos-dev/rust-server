mod held_prediction;
mod prediction;
mod presentation;
mod reconciliation;

use std::{fs, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, anyhow, bail};
use aurenfall_contracts::{
    AURENFALL_ALPN, ClientHello, MessageKind, MoveIntent, MovementPredictionProfile, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, SelfMovementSnapshot, SelfMovementSnapshotV2, ServerHello,
};
use aurenfall_transport::{
    decode_datagram_frame, encode_datagram_frame, read_single_frame, write_single_frame,
};
use held_prediction::HeldInputExtrapolator;
use prediction::{ClientPredictionClock, PositionalPredictor, PredictedPositionMm};
use presentation::{CorrectionMode, PresentationReconciler};
use quinn::{Endpoint, crypto::rustls::QuicClientConfig};
use reconciliation::{ClientReconciler, ReplayPlan, SnapshotApplyOutcome};
use rustls::{RootCertStore, pki_types::CertificateDer};
use tokio::time::{Instant, timeout, timeout_at};
use tracing::{debug, info};

const CLIENT_BUILD: u32 = 3;
const MAX_CONTROL_FRAME_BYTES: usize = 4096;
const MAX_DATAGRAM_BYTES: usize = 1200;
const BOOTSTRAP_RESPONSE_RECEIVED_CODE: u32 = 0x1002;
const PROBE_OBSERVATION_GRACE: Duration = Duration::from_millis(750);
const PREDICTION_PROFILE_WAIT: Duration = Duration::from_secs(5);
const PENDING_INPUT_HISTORY_CAPACITY: usize = 256;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    aurenfall_observability::init("info");

    let server_address: SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7777".to_string())
        .parse()
        .context("invalid server socket address")?;
    let certificate_path = std::env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/tls/dev-cert.der"));
    let server_name = std::env::args().nth(3).unwrap_or_else(|| "localhost".to_string());

    let certificate = fs::read(&certificate_path)
        .with_context(|| format!("failed to read {}", certificate_path.display()))?;
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(certificate))
        .context("invalid trusted development certificate")?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("failed to configure TLS 1.3")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![AURENFALL_ALPN.to_vec()];
    let quic_crypto = QuicClientConfig::try_from(tls).context("TLS config is not QUIC compatible")?;
    let client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));

    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().context("invalid local client bind")?)?;
    endpoint.set_default_client_config(client_config);
    let connection = endpoint
        .connect(server_address, &server_name)
        .context("failed to start QUIC connection")?
        .await
        .context("QUIC connection failed")?;

    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .context("failed to open control stream")?;
    let mut client_nonce = [0_u8; 16];
    getrandom::fill(&mut client_nonce)
        .map_err(|error| anyhow!("failed to create client handshake nonce: {error}"))?;
    let hello = ClientHello {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        client_build: CLIENT_BUILD,
        client_nonce,
    };
    write_single_frame(
        &mut send,
        MessageKind::ClientHello,
        &hello.encode(),
        MAX_CONTROL_FRAME_BYTES,
    )
    .await
    .context("failed to send ClientHello")?;

    let (kind, payload) = read_single_frame(&mut recv, MAX_CONTROL_FRAME_BYTES)
        .await
        .context("failed to receive ServerHello")?;
    if kind != MessageKind::ServerHello {
        bail!("server returned unexpected bootstrap message {kind:?}");
    }
    let server_hello = ServerHello::decode(&payload).context("invalid ServerHello")?;
    if server_hello.client_nonce_echo != client_nonce {
        bail!("ServerHello nonce echo does not match ClientHello");
    }
    if !server_hello.accepted {
        connection.close(
            BOOTSTRAP_RESPONSE_RECEIVED_CODE.into(),
            b"bootstrap rejection received",
        );
        endpoint.wait_idle().await;
        bail!("Aurenfall handshake rejected: {:?}", server_hello.reject_code);
    }

    info!(
        server = %server_address,
        connection_id = server_hello.connection_id,
        session_id = server_hello.session_id,
        universe_id = server_hello.universe_id,
        protocol_major = server_hello.protocol_major,
        protocol_minor = server_hello.protocol_minor,
        client_build = CLIENT_BUILD,
        "Aurenfall protocol probe connected successfully"
    );

    let mut prediction_stream = timeout(PREDICTION_PROFILE_WAIT, connection.accept_uni())
        .await
        .context("timed out waiting for server-owned movement prediction profile")??;
    let (profile_kind, profile_payload) = read_single_frame(&mut prediction_stream, MAX_CONTROL_FRAME_BYTES)
        .await
        .context("failed to receive movement prediction profile")?;
    if profile_kind != MessageKind::MovementPredictionProfile {
        bail!("server returned unexpected initial reliable message {profile_kind:?}");
    }
    let prediction_profile = MovementPredictionProfile::decode(&profile_payload)
        .context("invalid server-owned MovementPredictionProfile")?;
    let predictor = PositionalPredictor::new(prediction_profile)
        .context("failed to construct client positional predictor")?;
    let correction_profile = prediction_profile.presentation_correction;
    let mut presentation_reconciler = PresentationReconciler::new(correction_profile)
        .context("failed to construct client presentation reconciler")?;
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
        "protocol probe received server-owned movement prediction and presentation profile"
    );

    let prediction_clock = ClientPredictionClock::start();
    let mut reconciler = ClientReconciler::new(PENDING_INPUT_HISTORY_CAPACITY)
        .context("failed to create bounded client reconciliation history")?;
    let mut held_extrapolator = HeldInputExtrapolator::new(PENDING_INPUT_HISTORY_CAPACITY)
        .context("failed to create bounded acknowledged held-input history")?;
    let move_intent = MoveIntent::new(1, 16_384, 0).context("failed to create protocol-probe MoveIntent")?;
    send_and_record_move(
        &connection,
        &prediction_clock,
        &mut reconciler,
        &mut held_extrapolator,
        move_intent,
    )?;
    info!(
        sequence = move_intent.sequence,
        axis_x = move_intent.axis_x,
        axis_y = move_intent.axis_y,
        observation_ms = PROBE_OBSERVATION_GRACE.as_millis(),
        pending_history_capacity = PENDING_INPUT_HISTORY_CAPACITY,
        "protocol probe sent first MoveIntent under server-owned prediction profile"
    );

    let observation_deadline = Instant::now() + PROBE_OBSERVATION_GRACE;
    let mut snapshot_count = 0_u32;
    let mut applied_snapshot_count = 0_u32;
    let mut stale_snapshot_count = 0_u32;
    let mut duplicate_snapshot_count = 0_u32;
    let mut max_pre_replay_duration_us = 0_u64;
    let mut max_abs_correction_x_mm = 0_u64;
    let mut held_extrapolation_snapshot_count = 0_u32;
    let mut absorb_correction_count = 0_u32;
    let mut smooth_correction_count = 0_u32;
    let mut rapid_correction_count = 0_u32;
    let mut hard_snap_correction_count = 0_u32;
    let mut max_presentation_correction_distance_mm = 0_u64;
    let last_authoritative_snapshot: Option<SelfMovementSnapshot> = None;
    let mut second_intent_sent = false;

    loop {
        let datagram = match timeout_at(observation_deadline, connection.read_datagram()).await {
            Ok(Ok(datagram)) => datagram,
            Ok(Err(reason)) => {
                return Err(reason).context("QUIC connection closed during snapshot observation");
            }
            Err(_) => break,
        };
        let (kind, payload) = decode_datagram_frame(&datagram, MAX_DATAGRAM_BYTES)
            .context("invalid authoritative snapshot datagram frame")?;
        if kind != MessageKind::SelfMovementSnapshotV2 {
            bail!("protocol probe received unexpected realtime message {kind:?}");
        }
        let snapshot_v2 = SelfMovementSnapshotV2::decode_wire(payload)
            .context("failed to decode authoritative SelfMovementSnapshotV2")?;
        let snapshot = SelfMovementSnapshot::new(
            snapshot_v2.server_tick,
            snapshot_v2.x_mm,
            snapshot_v2.y_mm,
            snapshot_v2.z_mm,
            snapshot_v2.last_processed_input_sequence,
        );
        snapshot_count = snapshot_count
            .checked_add(1)
            .context("protocol probe snapshot counter overflow")?;

        let received_at = prediction_clock
            .now()
            .context("failed to timestamp authoritative snapshot on client prediction clock")?;
        let pending_pre_replay = reconciler
            .replay_plan(received_at)
            .context("failed to build pending temporal replay plan before reconciliation")?;
        let pre_replay = held_extrapolator
            .augment_replay_plan(&pending_pre_replay, received_at)
            .context("failed to augment replay with acknowledged held input")?;
        max_pre_replay_duration_us = max_pre_replay_duration_us.max(pre_replay.total_duration_us);
        if pre_replay.segments.len() > pending_pre_replay.segments.len() {
            held_extrapolation_snapshot_count = held_extrapolation_snapshot_count
                .checked_add(1)
                .context("held extrapolation snapshot counter overflow")?;
        }
        let old_gameplay_prediction = last_authoritative_snapshot
            .map(|baseline| gameplay_position(predictor, baseline, &pre_replay))
            .transpose()
            .context("client pre-reconciliation gameplay prediction failed")?;

        match reconciler
            .apply_snapshot(snapshot)
            .context("authoritative movement snapshot violated reconciliation invariants")?
        {
            SnapshotApplyOutcome::Applied(applied) => {
                held_extrapolator
                    .apply_authoritative_snapshot(snapshot_v2, received_at)
                    .context("authoritative active movement state violated held-input invariants")?;
                applied_snapshot_count = applied_snapshot_count
                    .checked_add(1)
                    .context("protocol probe applied snapshot counter overflow")?;
                let pending_post_replay = reconciler
                    .replay_plan(received_at)
                    .context("failed to rebuild pending temporal replay after reconciliation")?;
                let post_replay = held_extrapolator
                    .augment_replay_plan(&pending_post_replay, received_at)
                    .context("failed to rebuild held-input replay after reconciliation")?;
                let new_gameplay_prediction = gameplay_position(predictor, applied.snapshot, &post_replay)
                    .context("client post-reconciliation gameplay prediction failed")?;
                let correction_x_mm = correction_x(old_gameplay_prediction, applied.snapshot)?;
                max_abs_correction_x_mm = max_abs_correction_x_mm.max(correction_x_mm.unsigned_abs());

                let correction_decision = old_gameplay_prediction
                    .map(|old_gameplay| {
                        presentation_reconciler.reconcile(old_gameplay, new_gameplay_prediction, received_at)
                    })
                    .transpose()
                    .context("client presentation correction planning failed")?;
                if let Some(decision) = correction_decision {
                    max_presentation_correction_distance_mm =
                        max_presentation_correction_distance_mm.max(decision.distance_mm);
                    match decision.mode {
                        CorrectionMode::Absorb => {
                            absorb_correction_count = absorb_correction_count
                                .checked_add(1)
                                .context("absorb correction counter overflow")?;
                        }
                        CorrectionMode::Smooth => {
                            smooth_correction_count = smooth_correction_count
                                .checked_add(1)
                                .context("smooth correction counter overflow")?;
                        }
                        CorrectionMode::Rapid => {
                            rapid_correction_count = rapid_correction_count
                                .checked_add(1)
                                .context("rapid correction counter overflow")?;
                        }
                        CorrectionMode::HardSnap => {
                            hard_snap_correction_count = hard_snap_correction_count
                                .checked_add(1)
                                .context("hard snap correction counter overflow")?;
                        }
                    }
                }
                let _rendered_position = presentation_reconciler
                    .rendered_position(new_gameplay_prediction, received_at)
                    .context("failed to sample client rendered position after reconciliation")?;

                info!(
                    server_tick = applied.snapshot.server_tick,
                    x_mm = applied.snapshot.x_mm,
                    y_mm = applied.snapshot.y_mm,
                    z_mm = applied.snapshot.z_mm,
                    last_processed_input_sequence = ?applied.snapshot.sequence,
                    acknowledged_inputs = applied.acknowledged_inputs,
                    pending_inputs = applied.pending_inputs,
                    final_x_mm = new_gameplay_prediction.x,
                    final_y_mm = new_gameplay_prediction.y,
                    "applied authoritative self movement snapshot"
                );

                if !second_intent_sent && applied.snapshot.sequence == Some(1) {
                    if let Ok(second_intent) = MoveIntent::new(2, 0, 127) {
                        let datagram = encode_datagram_frame(
                            MessageKind::MoveIntent,
                            &second_intent.encode(),
                            MAX_DATAGRAM_BYTES,
                        )
                        .context("failed to encode second MoveIntent datagram")?;
                        connection
                            .send_datagram(datagram.into())
                            .context("failed to send second MoveIntent datagram")?;
                        let sent_at = prediction_clock
                            .now()
                            .context("failed to timestamp second MoveIntent")?;
                        reconciler
                            .record_sent_input(second_intent, sent_at)
                            .context("failed to record second MoveIntent")?;
                        held_extrapolator
                            .record_sent_input(second_intent)
                            .context("failed to record second MoveIntent in held extrapolator")?;
                        second_intent_sent = true;
                        info!(
                            sequence = 2,
                            axis_x = 0,
                            axis_y = 127,
                            "sent second MoveIntent datagram following sequence 1 confirmation"
                        );
                    }
                }
            }
            SnapshotApplyOutcome::IgnoredStale {
                incoming_tick,
                current_tick,
            } => {
                stale_snapshot_count = stale_snapshot_count
                    .checked_add(1)
                    .context("protocol probe stale snapshot counter overflow")?;
                debug!(
                    incoming_tick,
                    current_tick, "protocol probe ignored stale authoritative movement snapshot"
                );
            }
            SnapshotApplyOutcome::IgnoredDuplicate { server_tick } => {
                duplicate_snapshot_count = duplicate_snapshot_count
                    .checked_add(1)
                    .context("protocol probe duplicate snapshot counter overflow")?;
                debug!(
                    server_tick,
                    "protocol probe ignored duplicate authoritative movement snapshot"
                );
            }
        }
    }

    info!(
        snapshot_count,
        applied_snapshot_count,
        stale_snapshot_count,
        duplicate_snapshot_count,
        max_pre_replay_duration_us,
        max_abs_correction_x_mm,
        held_extrapolation_snapshot_count,
        absorb_correction_count,
        smooth_correction_count,
        rapid_correction_count,
        hard_snap_correction_count,
        max_presentation_correction_distance_mm,
        second_intent_sent,
        "protocol probe completed presentation reconciliation observation"
    );
    connection.close(0_u32.into(), b"protocol probe complete");
    endpoint.wait_idle().await;
    Ok(())
}

fn gameplay_position(
    predictor: PositionalPredictor,
    baseline: SelfMovementSnapshot,
    replay: &ReplayPlan,
) -> anyhow::Result<PredictedPositionMm> {
    if replay.segments.is_empty() {
        return Ok(PredictedPositionMm {
            x: baseline.x_mm,
            y: baseline.y_mm,
            z: baseline.z_mm,
        });
    }
    predictor
        .predict(baseline, replay)
        .context("failed to integrate replay into gameplay prediction")
}

fn send_and_record_move(
    connection: &quinn::Connection,
    prediction_clock: &ClientPredictionClock,
    reconciler: &mut ClientReconciler,
    held_extrapolator: &mut HeldInputExtrapolator,
    intent: MoveIntent,
) -> anyhow::Result<u64> {
    let datagram = encode_datagram_frame(MessageKind::MoveIntent, &intent.encode(), MAX_DATAGRAM_BYTES)
        .context("failed to encode protocol-probe MoveIntent datagram")?;
    connection
        .send_datagram(datagram.into())
        .context("failed to send protocol-probe MoveIntent datagram")?;
    let sent_at = prediction_clock
        .now()
        .context("failed to timestamp sent MoveIntent on client prediction clock")?;
    reconciler
        .record_sent_input(intent, sent_at)
        .context("failed to record sent MoveIntent for reconciliation")?;
    held_extrapolator
        .record_sent_input(intent)
        .context("failed to record sent MoveIntent for held-input extrapolation")?;
    Ok(sent_at.value())
}

fn correction_x(
    predicted: Option<PredictedPositionMm>,
    authoritative: SelfMovementSnapshot,
) -> anyhow::Result<i64> {
    let Some(predicted) = predicted else {
        return Ok(0);
    };
    authoritative
        .x_mm
        .checked_sub(predicted.x)
        .context("prediction correction X overflow")
}
