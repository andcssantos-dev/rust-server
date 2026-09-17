use std::{fs, path::PathBuf};

use anyhow::Context;
use rcgen::{CertifiedKey, generate_simple_self_signed};

fn main() -> anyhow::Result<()> {
    let output_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/tls"));
    fs::create_dir_all(&output_dir).with_context(|| format!("failed to create {}", output_dir.display()))?;

    let certificate_path = output_dir.join("dev-cert.der");
    let private_key_path = output_dir.join("dev-key.der");
    if certificate_path.exists() && private_key_path.exists() {
        println!(
            "Development TLS identity already exists at {}",
            output_dir.display()
        );
        return Ok(());
    }

    let CertifiedKey { cert, signing_key } = generate_simple_self_signed(vec!["localhost".to_string()])
        .context("failed to generate development TLS identity")?;
    fs::write(&certificate_path, cert.der().as_ref())
        .with_context(|| format!("failed to write {}", certificate_path.display()))?;
    fs::write(&private_key_path, signing_key.serialize_der())
        .with_context(|| format!("failed to write {}", private_key_path.display()))?;

    println!(
        "Created development TLS certificate: {}",
        certificate_path.display()
    );
    println!(
        "Created development TLS private key: {}",
        private_key_path.display()
    );
    Ok(())
}
