use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, anyhow, ensure};
use aurenfall_contracts::{
    AURENFALL_ALPN, ClientHello, HandshakeRejectCode, MessageKind, MoveIntent, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, SelfMovementSnapshot, ServerHello,
};
use aurenfall_core::UniverseId;
use aurenfall_transport::{
    QuicServer, QuicServerSettings, TransportSessionEvent, decode_datagram_frame, encode_datagram_frame,
    read_single_frame, write_single_frame,
};
use quinn::{Endpoint, crypto::rustls::QuicClientConfig};
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::{RootCertStore, pki_types::CertificateDer};
use tokio::{
    sync::{mpsc, watch},
    time::timeout,
};

const MAX_CONTROL_FRAME_BYTES: usize = 4096;
const MAX_DATAGRAM_BYTES: usize = 1200;
const BOOTSTRAP_RESPONSE_RECEIVED_CODE: u32 = 0x1002;

#[derive(Debug)]
struct TestIdentity {
    directory: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
}

impl Drop for TestIdentity {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn create_test_identity() -> anyhow::Result<TestIdentity> {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random)
        .map_err(|error| anyhow!("failed to create test identity suffix: {error}"))?;
    let suffix = u64::from_le_bytes(random);
    let directory = std::env::temp_dir().join(format!("aurenfall-quic-test-{}-{suffix}", std::process::id()));
    fs::create_dir_all(&directory).with_context(|| format!("failed to create {}", directory.display()))?;

    let certificate = directory.join("cert.der");
    let private_key = directory.join("key.der");
    let CertifiedKey { cert, signing_key } = generate_simple_self_signed(vec!["localhost".to_string()])
        .context("failed to generate test TLS identity")?;
    fs::write(&certificate, cert.der().as_ref())
        .with_context(|| format!("failed to write {}", certificate.display()))?;
    fs::write(&private_key, signing_key.serialize_der())
        .with_context(|| format!("failed to write {}", private_key.display()))?;

    Ok(TestIdentity {
        directory,
        certificate,
        private_key,
    })
}

fn server_settings(identity: &TestIdentity, minimum_client_build: u32) -> QuicServerSettings {
    QuicServerSettings {
        bind_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        certificate_der_path: identity.certificate.clone(),
        private_key_der_path: identity.private_key.clone(),
        universe_id: UniverseId(42),
        max_connections: 8,
        max_datagram_bytes: MAX_DATAGRAM_BYTES,
        max_control_frame_bytes: MAX_CONTROL_FRAME_BYTES,
        max_bidi_streams: 4,
        max_uni_streams: 4,
        handshake_timeout: Duration::from_secs(5),
        rejection_close_grace: Duration::from_secs(2),
        idle_timeout: Duration::from_secs(10),
        datagram_receive_buffer_bytes: 64 * 1024,
        datagram_send_buffer_bytes: 64 * 1024,
        minimum_client_build,
    }
}

async fn connect_client(
    server_address: SocketAddr,
    certificate_path: &Path,
) -> anyhow::Result<(Endpoint, quinn::Connection)> {
    let certificate = fs::read(certificate_path)
        .with_context(|| format!("failed to read {}", certificate_path.display()))?;
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(certificate))
        .context("invalid test certificate")?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("failed to configure test TLS 1.3")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![AURENFALL_ALPN.to_vec()];
    let quic_crypto = QuicClientConfig::try_from(tls).context("test TLS config is not QUIC compatible")?;
    let client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));

    let mut endpoint = Endpoint::client(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    endpoint.set_default_client_config(client_config);
    let connection = endpoint
        .connect(server_address, "localhost")
        .context("failed to start test QUIC connection")?
        .await
        .context("test QUIC connection failed")?;
    Ok((endpoint, connection))
}

async fn exchange_hello(
    connection: &quinn::Connection,
    client_build: u32,
    protocol_minor: u16,
) -> anyhow::Result<ServerHello> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .context("failed to open test bootstrap stream")?;
    let mut client_nonce = [0_u8; 16];
    getrandom::fill(&mut client_nonce)
        .map_err(|error| anyhow!("failed to create test client nonce: {error}"))?;
    let client_hello = ClientHello {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor,
        client_build,
        client_nonce,
    };
    write_single_frame(
        &mut send,
        MessageKind::ClientHello,
        &client_hello.encode(),
        MAX_CONTROL_FRAME_BYTES,
    )
    .await
    .context("failed to send test ClientHello")?;

    let (kind, payload) = read_single_frame(&mut recv, MAX_CONTROL_FRAME_BYTES)
        .await
        .context("failed to receive test ServerHello")?;
    ensure!(
        kind == MessageKind::ServerHello,
        "unexpected bootstrap response {kind:?}"
    );
    let server_hello = ServerHello::decode(&payload).context("invalid test ServerHello")?;
    ensure!(
        server_hello.client_nonce_echo == client_nonce,
        "ServerHello nonce echo mismatch"
    );
    Ok(server_hello)
}

async fn stop_server(
    shutdown_tx: watch::Sender<bool>,
    server_task: tokio::task::JoinHandle<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    shutdown_tx
        .send(true)
        .map_err(|_| anyhow!("test server shutdown receiver disappeared"))?;
    server_task.await.context("test server task join failed")??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_bootstrap_assigns_nonzero_runtime_session() -> anyhow::Result<()> {
    let identity = create_test_identity()?;
    let server = QuicServer::bind(server_settings(&identity, 1))?;
    let server_address = server.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(server.run(shutdown_rx));

    let (endpoint, connection) = connect_client(server_address, &identity.certificate).await?;
    let hello = exchange_hello(&connection, 1, PROTOCOL_MINOR).await?;
    ensure!(
        hello.accepted,
        "valid bootstrap was rejected: {:?}",
        hello.reject_code
    );
    ensure!(
        hello.connection_id != 0,
        "accepted connection must have ConnectionId"
    );
    ensure!(hello.session_id != 0, "accepted connection must have SessionId");
    ensure!(hello.universe_id == 42, "unexpected universe id");
    ensure!(
        hello.reject_code == HandshakeRejectCode::None,
        "unexpected reject code"
    );

    connection.close(0_u32.into(), b"test complete");
    endpoint.wait_idle().await;
    stop_server(shutdown_tx, server_task).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minor_zero_session_cannot_claim_minor_one_reliable_capability() -> anyhow::Result<()> {
    let identity = create_test_identity()?;
    let (events_tx, mut events_rx) = mpsc::channel(8);
    let server = QuicServer::bind(server_settings(&identity, 1))?.with_session_events(events_tx);
    let server_address = server.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(server.run(shutdown_rx));

    let (endpoint, connection) = connect_client(server_address, &identity.certificate).await?;
    let hello = exchange_hello(&connection, 1, 0).await?;
    ensure!(
        hello.accepted,
        "protocol 1.0 client should be accepted by server 1.1"
    );
    ensure!(
        hello.protocol_minor == 0,
        "server did not negotiate protocol minor 0"
    );

    let admitted = timeout(Duration::from_secs(2), events_rx.recv())
        .await
        .context("timed out waiting for protocol 1.0 admitted session event")?
        .context("session event channel closed before protocol 1.0 admission")?;
    let reliable = match admitted {
        TransportSessionEvent::Admitted { reliable, .. } => reliable,
        other => return Err(anyhow!("unexpected protocol 1.0 admission event: {other:?}")),
    };
    ensure!(
        reliable.negotiated_protocol_minor() == 0,
        "protocol 1.0 admission carried the wrong negotiated minor"
    );
    ensure!(
        !reliable.supports_protocol_minor(1),
        "protocol 1.0 admission incorrectly exposes protocol 1.1 reliable capability"
    );

    connection.close(0_u32.into(), b"test complete");
    endpoint.wait_idle().await;
    stop_server(shutdown_tx, server_task).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minor_one_session_retains_minor_one_reliable_capability() -> anyhow::Result<()> {
    let identity = create_test_identity()?;
    let (events_tx, mut events_rx) = mpsc::channel(8);
    let server = QuicServer::bind(server_settings(&identity, 1))?.with_session_events(events_tx);
    let server_address = server.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(server.run(shutdown_rx));

    let (endpoint, connection) = connect_client(server_address, &identity.certificate).await?;

    let hello = exchange_hello(&connection, 1, 1).await?;

    ensure!(
        hello.accepted,
        "protocol 1.1 client should be accepted by server 1.2"
    );
    ensure!(
        hello.protocol_minor == 1,
        "server did not preserve negotiated protocol minor 1"
    );

    let admitted = timeout(Duration::from_secs(2), events_rx.recv())
        .await
        .context("timed out waiting for protocol 1.1 admitted session")?
        .context("session channel closed before protocol 1.1 admission")?;

    let reliable = match admitted {
        TransportSessionEvent::Admitted { reliable, .. } => reliable,
        other => {
            return Err(anyhow!("unexpected protocol 1.1 admission event: {other:?}"));
        }
    };

    ensure!(
        reliable.negotiated_protocol_minor() == 1,
        "protocol 1.1 admission carried wrong negotiated minor"
    );
    ensure!(
        reliable.supports_protocol_minor(1),
        "protocol 1.1 sender lost minor-1 capability"
    );
    ensure!(
        !reliable.supports_protocol_minor(2),
        "protocol 1.1 sender incorrectly exposes minor-2 capability"
    );

    connection.close(0_u32.into(), b"test complete");
    endpoint.wait_idle().await;
    stop_server(shutdown_tx, server_task).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minor_two_session_receives_minor_two_reliable_capability() -> anyhow::Result<()> {
    let identity = create_test_identity()?;
    let (events_tx, mut events_rx) = mpsc::channel(8);
    let server = QuicServer::bind(server_settings(&identity, 1))?.with_session_events(events_tx);
    let server_address = server.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(server.run(shutdown_rx));

    let (endpoint, connection) = connect_client(server_address, &identity.certificate).await?;

    let hello = exchange_hello(&connection, 1, 2).await?;

    ensure!(
        hello.accepted,
        "protocol 1.2 client should be accepted by server 1.2"
    );
    ensure!(
        hello.protocol_minor == 2,
        "server did not negotiate protocol minor 2"
    );

    let admitted = timeout(Duration::from_secs(2), events_rx.recv())
        .await
        .context("timed out waiting for protocol 1.2 admitted session")?
        .context("session channel closed before protocol 1.2 admission")?;

    let reliable = match admitted {
        TransportSessionEvent::Admitted { reliable, .. } => reliable,
        other => {
            return Err(anyhow!("unexpected protocol 1.2 admission event: {other:?}"));
        }
    };

    ensure!(
        reliable.negotiated_protocol_minor() == 2,
        "protocol 1.2 admission carried wrong negotiated minor"
    );
    ensure!(
        reliable.supports_protocol_minor(1),
        "protocol 1.2 sender lost minor-1 capability"
    );
    ensure!(
        reliable.supports_protocol_minor(2),
        "protocol 1.2 sender does not expose minor-2 capability"
    );

    connection.close(0_u32.into(), b"test complete");
    endpoint.wait_idle().await;
    stop_server(shutdown_tx, server_task).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_build_never_receives_runtime_session() -> anyhow::Result<()> {
    let identity = create_test_identity()?;
    let server = QuicServer::bind(server_settings(&identity, 2))?;
    let server_address = server.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(server.run(shutdown_rx));

    let (endpoint, connection) = connect_client(server_address, &identity.certificate).await?;
    let hello = exchange_hello(&connection, 1, PROTOCOL_MINOR).await?;
    ensure!(!hello.accepted, "outdated client build was accepted");
    ensure!(
        hello.connection_id != 0,
        "TLS-established connection must have ConnectionId"
    );
    ensure!(
        hello.session_id == 0,
        "rejected bootstrap must not receive SessionId"
    );
    ensure!(
        hello.reject_code == HandshakeRejectCode::ClientBuildTooOld,
        "unexpected reject code: {:?}",
        hello.reject_code
    );

    connection.close(
        BOOTSTRAP_RESPONSE_RECEIVED_CODE.into(),
        b"bootstrap rejection received",
    );
    endpoint.wait_idle().await;
    stop_server(shutdown_tx, server_task).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_session_supports_live_ingress_and_scoped_realtime_egress() -> anyhow::Result<()> {
    let identity = create_test_identity()?;
    let (events_tx, mut events_rx) = mpsc::channel(8);
    let server = QuicServer::bind(server_settings(&identity, 1))?.with_session_events(events_tx);
    let server_address = server.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(server.run(shutdown_rx));

    let (endpoint, connection) = connect_client(server_address, &identity.certificate).await?;
    let hello = exchange_hello(&connection, 1, PROTOCOL_MINOR).await?;
    ensure!(hello.accepted, "valid bootstrap was rejected");

    let admitted = timeout(Duration::from_secs(2), events_rx.recv())
        .await
        .context("timed out waiting for admitted session event")?
        .context("session event channel closed before admission")?;
    ensure!(
        matches!(
            &admitted,
            TransportSessionEvent::Admitted {
                connection_id,
                session_id,
                ..
            } if *connection_id == aurenfall_core::ConnectionId(hello.connection_id)
                && *session_id == aurenfall_core::SessionId(hello.session_id)
        ),
        "unexpected admission event: {admitted:?}"
    );
    let (realtime, reliable) = match &admitted {
        TransportSessionEvent::Admitted {
            realtime, reliable, ..
        } => (realtime.clone(), reliable.clone()),
        _ => return Err(anyhow!("expected admitted session event")),
    };
    ensure!(
        reliable.negotiated_protocol_minor() == hello.protocol_minor,
        "live reliable sender did not retain the negotiated protocol minor"
    );
    ensure!(
        reliable.supports_protocol_minor(PROTOCOL_MINOR),
        "protocol 1.1 admission did not expose protocol 1.1 reliable capability"
    );

    let authoritative_snapshot = SelfMovementSnapshot {
        server_tick: 77,
        x_mm: 400,
        y_mm: -25,
        z_mm: 0,
        last_processed_input_sequence: Some(7),
    };
    realtime.try_send(
        MessageKind::SelfMovementSnapshot,
        &authoritative_snapshot.encode(),
    )?;
    let outbound = timeout(Duration::from_secs(2), connection.read_datagram())
        .await
        .context("timed out waiting for realtime egress datagram")??;
    let (outbound_kind, outbound_payload) = decode_datagram_frame(&outbound, MAX_DATAGRAM_BYTES)?;
    ensure!(
        outbound_kind == MessageKind::SelfMovementSnapshot,
        "unexpected realtime egress message {outbound_kind:?}"
    );
    ensure!(
        SelfMovementSnapshot::decode(outbound_payload)? == authoritative_snapshot,
        "realtime egress snapshot payload changed"
    );

    let move_intent = MoveIntent::new(7, 12_000, -8_000)?;
    let datagram = encode_datagram_frame(MessageKind::MoveIntent, &move_intent.encode(), MAX_DATAGRAM_BYTES)?;
    connection
        .send_datagram(datagram.into())
        .context("failed to send test MoveIntent datagram")?;

    let movement = timeout(Duration::from_secs(2), events_rx.recv())
        .await
        .context("timed out waiting for MoveIntent event")?
        .context("session event channel closed before movement")?;
    ensure!(
        movement
            == TransportSessionEvent::MoveIntent {
                connection_id: aurenfall_core::ConnectionId(hello.connection_id),
                intent: move_intent,
            },
        "unexpected movement event: {movement:?}"
    );

    connection.close(0_u32.into(), b"test complete");
    endpoint.wait_idle().await;
    stop_server(shutdown_tx, server_task).await
}
