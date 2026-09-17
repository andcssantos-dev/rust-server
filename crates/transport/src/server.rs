use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, anyhow, bail};
use aurenfall_contracts::{
    AURENFALL_ALPN, ClientHello, HandshakeRejectCode, MessageKind, MineIntent, MoveIntent,
    PROTOCOL_MAJOR, PROTOCOL_MINOR, ServerHello, negotiate_protocol_minor,
};
use aurenfall_core::{ConnectionId, SessionId, UniverseId};
use quinn::{Endpoint, Incoming, VarInt, crypto::rustls::QuicServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::{
    sync::{Semaphore, mpsc, watch},
    task::JoinSet,
    time::{Instant, timeout_at},
};
use tracing::{debug, info, warn};

use crate::{
    RealtimeConnectionSender, ReliableConnectionSender, TransportSessionEvent, decode_datagram_frame,
    read_single_frame, reliable_ingress::run_live_reliable_control_ingress, write_single_frame,
};

const HANDSHAKE_REJECTED_CODE: u32 = 0x1001;
const SESSION_INGRESS_UNAVAILABLE_CODE: u32 = 0x1003;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialReliableFrame {
    pub kind: MessageKind,
    pub payload: Vec<u8>,
}

impl InitialReliableFrame {
    #[must_use]
    pub fn new(kind: MessageKind, payload: Vec<u8>) -> Self {
        Self { kind, payload }
    }
}

#[derive(Debug, Clone)]
pub struct QuicServerSettings {
    pub bind_address: SocketAddr,
    pub certificate_der_path: PathBuf,
    pub private_key_der_path: PathBuf,
    pub universe_id: UniverseId,
    pub max_connections: usize,
    pub max_datagram_bytes: usize,
    pub max_control_frame_bytes: usize,
    pub max_bidi_streams: u32,
    pub max_uni_streams: u32,
    pub handshake_timeout: Duration,
    pub rejection_close_grace: Duration,
    pub idle_timeout: Duration,
    pub datagram_receive_buffer_bytes: usize,
    pub datagram_send_buffer_bytes: usize,
    pub minimum_client_build: u32,
}

#[derive(Debug)]
pub struct QuicServer {
    endpoint: Endpoint,
    settings: Arc<QuicServerSettings>,
    capacity: Arc<Semaphore>,
    next_connection_id: Arc<AtomicU64>,
    next_session_id: Arc<AtomicU64>,
    session_events: Option<mpsc::Sender<TransportSessionEvent>>,
    initial_reliable_frames: Arc<Vec<InitialReliableFrame>>,
}

impl QuicServer {
    pub fn bind(settings: QuicServerSettings) -> anyhow::Result<Self> {
        if settings.max_connections == 0 {
            bail!("QUIC max_connections must be > 0");
        }
        let server_config = build_server_config(&settings)?;
        let endpoint = Endpoint::server(server_config, settings.bind_address)
            .with_context(|| format!("failed to bind QUIC endpoint at {}", settings.bind_address))?;
        Ok(Self {
            endpoint,
            capacity: Arc::new(Semaphore::new(settings.max_connections)),
            settings: Arc::new(settings),
            next_connection_id: Arc::new(AtomicU64::new(1)),
            next_session_id: Arc::new(AtomicU64::new(1)),
            session_events: None,
            initial_reliable_frames: Arc::new(Vec::new()),
        })
    }

    #[must_use]
    pub fn with_session_events(mut self, session_events: mpsc::Sender<TransportSessionEvent>) -> Self {
        self.session_events = Some(session_events);
        self
    }

    pub fn with_initial_reliable_frame(mut self, frame: InitialReliableFrame) -> anyhow::Result<Self> {
        if frame.payload.len() > self.settings.max_control_frame_bytes {
            bail!(
                "initial reliable frame payload {} exceeds configured control maximum {}",
                frame.payload.len(),
                self.settings.max_control_frame_bytes
            );
        }
        Arc::make_mut(&mut self.initial_reliable_frames).push(frame);
        Ok(self)
    }

    pub fn local_addr(&self) -> anyhow::Result<SocketAddr> {
        self.endpoint
            .local_addr()
            .context("failed to read QUIC local address")
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
        let mut connections = JoinSet::new();
        info!(bind = %self.local_addr()?, "QUIC transport listening");

        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                incoming = self.endpoint.accept() => {
                    match incoming {
                        Some(incoming) => self.admit(incoming, &mut connections),
                        None => break,
                    }
                }
                completed = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Err(error)) = completed {
                        warn!(%error, "QUIC connection task panicked");
                    }
                }
            }
        }

        self.endpoint.close(VarInt::from_u32(0), b"server shutdown");
        self.endpoint.wait_idle().await;
        while let Some(completed) = connections.join_next().await {
            if let Err(error) = completed {
                warn!(%error, "QUIC connection task panicked during shutdown");
            }
        }
        info!("QUIC transport stopped");
        Ok(())
    }

    fn admit(&self, incoming: Incoming, connections: &mut JoinSet<()>) {
        let remote = incoming.remote_address();
        let Ok(permit) = Arc::clone(&self.capacity).try_acquire_owned() else {
            warn!(%remote, "QUIC connection refused: capacity reached");
            incoming.refuse();
            return;
        };

        let settings = Arc::clone(&self.settings);
        let connection_ids = Arc::clone(&self.next_connection_id);
        let session_ids = Arc::clone(&self.next_session_id);
        let session_events = self.session_events.clone();
        let initial_reliable_frames = Arc::clone(&self.initial_reliable_frames);
        connections.spawn(async move {
            let _permit = permit;
            if let Err(error) = handle_connection(
                incoming,
                settings,
                connection_ids,
                session_ids,
                session_events,
                initial_reliable_frames,
            )
            .await
            {
                warn!(%remote, %error, "QUIC session ended with error");
            }
        });
    }
}

async fn handle_connection(
    incoming: Incoming,
    settings: Arc<QuicServerSettings>,
    connection_ids: Arc<AtomicU64>,
    session_ids: Arc<AtomicU64>,
    session_events: Option<mpsc::Sender<TransportSessionEvent>>,
    initial_reliable_frames: Arc<Vec<InitialReliableFrame>>,
) -> anyhow::Result<()> {
    let remote = incoming.remote_address();
    let deadline = Instant::now() + settings.handshake_timeout;
    let connecting = incoming.accept().context("failed to accept QUIC connection")?;
    let connection = timeout_at(deadline, connecting)
        .await
        .context("QUIC cryptographic handshake timed out")??;
    let connection_id = ConnectionId(allocate_runtime_id(&connection_ids)?);

    let (mut send, mut recv) = timeout_at(deadline, connection.accept_bi())
        .await
        .context("Aurenfall bootstrap stream timed out")??;
    let (kind, payload) = timeout_at(
        deadline,
        read_single_frame(&mut recv, settings.max_control_frame_bytes),
    )
    .await
    .context("Aurenfall ClientHello timed out")??;
    if kind != MessageKind::ClientHello {
        bail!("first Aurenfall bootstrap frame must be ClientHello");
    }

    let hello = ClientHello::decode(&payload).context("invalid Aurenfall ClientHello")?;
    let negotiated_minor = negotiate_protocol_minor(
        hello.protocol_major,
        hello.protocol_minor,
        PROTOCOL_MAJOR,
        PROTOCOL_MINOR,
    );
    let reject_code = if negotiated_minor.is_none() {
        HandshakeRejectCode::IncompatibleProtocol
    } else if hello.client_build < settings.minimum_client_build {
        HandshakeRejectCode::ClientBuildTooOld
    } else {
        HandshakeRejectCode::None
    };
    let accepted = reject_code == HandshakeRejectCode::None;
    let negotiated_minor = negotiated_minor.unwrap_or(PROTOCOL_MINOR);
    let session_id = if accepted {
        SessionId(allocate_runtime_id(&session_ids)?)
    } else {
        SessionId(0)
    };

    let mut server_nonce = [0_u8; 16];
    getrandom::fill(&mut server_nonce)
        .map_err(|error| anyhow!("failed to create server handshake nonce: {error}"))?;
    let server_hello = ServerHello {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: negotiated_minor,
        accepted,
        reject_code,
        connection_id: connection_id.0,
        session_id: session_id.0,
        universe_id: settings.universe_id.0,
        client_nonce_echo: hello.client_nonce,
        server_nonce,
    };
    timeout_at(
        deadline,
        write_single_frame(
            &mut send,
            MessageKind::ServerHello,
            &server_hello.encode(),
            settings.max_control_frame_bytes,
        ),
    )
    .await
    .context("Aurenfall ServerHello timed out")??;

    if !accepted {
        info!(
            %remote,
            connection_id = connection_id.0,
            client_protocol_major = hello.protocol_major,
            client_protocol_minor = hello.protocol_minor,
            client_build = hello.client_build,
            ?reject_code,
            "Aurenfall handshake rejected"
        );
        await_rejected_peer_close(&connection, settings.rejection_close_grace).await;
        return Ok(());
    }

    for frame in initial_reliable_frames.iter() {
        let control_deadline = Instant::now() + settings.handshake_timeout;
        let mut control_send = timeout_at(control_deadline, connection.open_uni())
            .await
            .context("Aurenfall initial reliable control stream timed out")??;
        timeout_at(
            control_deadline,
            write_single_frame(
                &mut control_send,
                frame.kind,
                &frame.payload,
                settings.max_control_frame_bytes,
            ),
        )
        .await
        .context("Aurenfall initial reliable control frame timed out")??;
    }

    let realtime =
        RealtimeConnectionSender::new(connection_id, connection.clone(), settings.max_datagram_bytes);
    let reliable = ReliableConnectionSender::new(
        connection_id,
        connection.clone(),
        settings.max_control_frame_bytes,
        settings.handshake_timeout,
    )
    .context("failed to create reliable control capability")?
    .with_negotiated_protocol_minor(negotiated_minor);
    if let Some(events) = session_events.as_ref()
        && let Err(error) = events.try_send(TransportSessionEvent::Admitted {
            connection_id,
            session_id,
            realtime,
            reliable,
        })
    {
        connection.close(
            VarInt::from_u32(SESSION_INGRESS_UNAVAILABLE_CODE),
            b"session ingress unavailable",
        );
        bail!("failed to admit live session into bounded session ingress: {error}");
    }

    info!(
        %remote,
        connection_id = connection_id.0,
        session_id = session_id.0,
        client_build = hello.client_build,
        protocol_major = hello.protocol_major,
        protocol_minor = negotiated_minor,
        "Aurenfall session admitted"
    );

    if let Some(events) = session_events {
        let reliable_ingress = tokio::spawn(run_live_reliable_control_ingress(
            connection.clone(),
            settings.max_control_frame_bytes,
            remote,
            connection_id,
            events.clone(),
        ));
        run_live_datagram_ingress(&connection, &settings, remote, connection_id, session_id, &events).await;
        reliable_ingress.abort();
        let _ = reliable_ingress.await;
    } else {
        let reason = connection.closed().await;
        debug!(
            %remote,
            connection_id = connection_id.0,
            session_id = session_id.0,
            ?reason,
            "Aurenfall session closed"
        );
    }
    Ok(())
}

async fn run_live_datagram_ingress(
    connection: &quinn::Connection,
    settings: &QuicServerSettings,
    remote: SocketAddr,
    connection_id: ConnectionId,
    session_id: SessionId,
    events: &mpsc::Sender<TransportSessionEvent>,
) {
    loop {
        let datagram = match connection.read_datagram().await {
            Ok(datagram) => datagram,
            Err(reason) => {
                debug!(
                    %remote,
                    connection_id = connection_id.0,
                    session_id = session_id.0,
                    ?reason,
                    "Aurenfall live datagram ingress stopped"
                );
                break;
            }
        };

        let (kind, payload) = match decode_datagram_frame(&datagram, settings.max_datagram_bytes) {
            Ok(frame) => frame,
            Err(error) => {
                warn!(
                    %remote,
                    connection_id = connection_id.0,
                    %error,
                    "dropping invalid Aurenfall datagram frame"
                );
                continue;
            }
        };

        match kind {
            MessageKind::MoveIntent => {
                let intent = match MoveIntent::decode(payload) {
                    Ok(intent) => intent,
                    Err(error) => {
                        warn!(
                            %remote,
                            connection_id = connection_id.0,
                            %error,
                            "dropping invalid MoveIntent payload"
                        );
                        continue;
                    }
                };

                match events.try_send(TransportSessionEvent::MoveIntent {
                    connection_id,
                    intent,
                }) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        debug!(
                            %remote,
                            connection_id = connection_id.0,
                            sequence = intent.sequence,
                            "dropping replaceable movement intent: session event queue full"
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        warn!(
                            %remote,
                            connection_id = connection_id.0,
                            "session event receiver closed; terminating live connection"
                        );
                        connection.close(
                            VarInt::from_u32(SESSION_INGRESS_UNAVAILABLE_CODE),
                            b"session ingress unavailable",
                        );
                        break;
                    }
                }
            }
            MessageKind::MineIntent => {
                let intent = match MineIntent::decode(payload) {
                    Ok(intent) => intent,
                    Err(error) => {
                        warn!(
                            %remote,
                            connection_id = connection_id.0,
                            %error,
                            "dropping invalid MineIntent payload"
                        );
                        continue;
                    }
                };

                tracing::info!(
                    connection_id = connection_id.0,
                    sequence = intent.sequence,
                    resource_id = intent.resource_id,
                    "RECEBIDO MineIntent no transporte QUIC!"
                );

                let _ = events.try_send(TransportSessionEvent::MineIntent {
                    connection_id,
                    intent,
                });
            }
            _ => {
                warn!(
                    %remote,
                    connection_id = connection_id.0,
                    ?kind,
                    "dropping unsupported realtime datagram message"
                );
            }
        }
    }

    if events
        .send(TransportSessionEvent::Disconnected {
            connection_id,
            session_id,
        })
        .await
        .is_err()
    {
        debug!(
            connection_id = connection_id.0,
            session_id = session_id.0,
            "session runtime already stopped before disconnect cleanup"
        );
    }
}

async fn await_rejected_peer_close(connection: &quinn::Connection, grace: Duration) {
    let deadline = Instant::now() + grace;
    match timeout_at(deadline, connection.closed()).await {
        Ok(reason) => {
            debug!(
                ?reason,
                "rejected client acknowledged bootstrap by closing connection"
            );
        }
        Err(_) => {
            debug!("rejected client did not close within grace period; forcing QUIC close");
            connection.close(
                VarInt::from_u32(HANDSHAKE_REJECTED_CODE),
                b"Aurenfall handshake rejected",
            );
        }
    }
}

fn allocate_runtime_id(counter: &AtomicU64) -> anyhow::Result<u64> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| anyhow!("runtime identifier space exhausted"))
}

fn build_server_config(settings: &QuicServerSettings) -> anyhow::Result<quinn::ServerConfig> {
    let certificate = fs::read(&settings.certificate_der_path).with_context(|| {
        format!(
            "failed to read TLS certificate {}",
            settings.certificate_der_path.display()
        )
    })?;
    let private_key = fs::read(&settings.private_key_der_path).with_context(|| {
        format!(
            "failed to read TLS private key {}",
            settings.private_key_der_path.display()
        )
    })?;

    let certificates = vec![CertificateDer::from(certificate)];
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("failed to configure TLS 1.3")?
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .context("invalid TLS certificate/private key pair")?;
    tls.alpn_protocols = vec![AURENFALL_ALPN.to_vec()];

    let quic_crypto = QuicServerConfig::try_from(tls).context("TLS config is not QUIC compatible")?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(VarInt::from_u32(settings.max_bidi_streams));
    transport.max_concurrent_uni_streams(VarInt::from_u32(settings.max_uni_streams));
    transport.max_idle_timeout(Some(
        settings
            .idle_timeout
            .try_into()
            .context("QUIC idle timeout is outside representable range")?,
    ));
    transport.datagram_receive_buffer_size(Some(settings.datagram_receive_buffer_bytes));
    transport.datagram_send_buffer_size(settings.datagram_send_buffer_bytes);
    server_config.transport_config(Arc::new(transport));
    Ok(server_config)
}
