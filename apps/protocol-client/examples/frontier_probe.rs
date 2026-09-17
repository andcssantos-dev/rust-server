use std::{fs, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, anyhow, bail};
use aurenfall_contracts::{
    AURENFALL_ALPN, ClientHello, FrontierManifest, MessageKind, MovementPredictionProfile, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, ServerHello,
};
use aurenfall_transport::{read_single_frame, write_single_frame};
use quinn::{Endpoint, crypto::rustls::QuicClientConfig};
use rustls::{RootCertStore, pki_types::CertificateDer};
use tokio::time::timeout;
use tracing::info;

const CLIENT_BUILD: u32 = 3;
const MAX_CONTROL_FRAME_BYTES: usize = 4096;
const INITIAL_RELIABLE_WAIT: Duration = Duration::from_secs(5);
const EXPECTED_INITIAL_RELIABLE_FRAMES: usize = 2;
const PROBE_COMPLETE_CODE: u32 = 0x1004;

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
        .context("failed to open bootstrap stream")?;
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
        bail!("Aurenfall handshake rejected: {:?}", server_hello.reject_code);
    }

    let mut prediction_profile = None;
    let mut frontier_manifest = None;
    for _ in 0..EXPECTED_INITIAL_RELIABLE_FRAMES {
        let mut stream = timeout(INITIAL_RELIABLE_WAIT, connection.accept_uni())
            .await
            .context("timed out waiting for initial reliable bootstrap frame")??;
        let (frame_kind, frame_payload) = read_single_frame(&mut stream, MAX_CONTROL_FRAME_BYTES)
            .await
            .context("failed to read initial reliable bootstrap frame")?;

        match frame_kind {
            MessageKind::MovementPredictionProfile => {
                if prediction_profile.is_some() {
                    bail!("received duplicate MovementPredictionProfile bootstrap frame");
                }
                prediction_profile = Some(
                    MovementPredictionProfile::decode(&frame_payload)
                        .context("invalid MovementPredictionProfile bootstrap payload")?,
                );
            }
            MessageKind::FrontierManifest => {
                if frontier_manifest.is_some() {
                    bail!("received duplicate FrontierManifest bootstrap frame");
                }
                frontier_manifest = Some(
                    FrontierManifest::decode(&frame_payload)
                        .context("invalid FrontierManifest bootstrap payload")?,
                );
            }
            other => bail!("received unexpected initial reliable bootstrap message {other:?}"),
        }
    }

    let prediction_profile = prediction_profile.context("MovementPredictionProfile was not received")?;
    let frontier_manifest = frontier_manifest.context("FrontierManifest was not received")?;
    let first = frontier_manifest
        .quadrants
        .first()
        .context("FrontierManifest unexpectedly contains no quadrants")?;
    let last = frontier_manifest
        .quadrants
        .last()
        .context("FrontierManifest unexpectedly contains no quadrants")?;

    info!(
        universe_id = server_hello.universe_id,
        prediction_profile_version = prediction_profile.profile_version,
        manifest_version = frontier_manifest.manifest_version,
        manifest_revision = frontier_manifest.revision,
        generator_version = frontier_manifest.generator_version,
        quadrant_size_mm = frontier_manifest.quadrant_size_mm,
        revealed_quadrants = frontier_manifest.quadrants.len(),
        first_x = first.x,
        first_y = first.y,
        last_x = last.x,
        last_y = last.y,
        "frontier probe received complete server-owned reliable bootstrap"
    );

    connection.close(PROBE_COMPLETE_CODE.into(), b"frontier probe complete");
    endpoint.wait_idle().await;
    Ok(())
}
