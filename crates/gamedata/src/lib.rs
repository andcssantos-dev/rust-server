use std::{fs, path::Path};

use anyhow::{bail, Context};
use serde::Deserialize;

// Módulos internos
pub mod items;
pub mod loader;
pub mod schemas;

// Reexportações públicas para fácil importação em outros crates
pub use items::ItemRegistry;
pub use loader::{GameDataRegistry, SharedGameData};
pub use schemas::*;

/// Manifesto mestre que valida a versão do conteúdo do jogo
#[derive(Debug, Clone, Deserialize)]
pub struct GameDataManifest {
    pub schema_version: u32,
    pub content_version: String,
    pub universe_generator_version: u32,
}

/// Lê e valida o arquivo manifest.yaml do diretório gamedata
pub fn load_manifest(path: &Path) -> anyhow::Result<GameDataManifest> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("unable to read GameData manifest {}", path.display()))?;
    let manifest: GameDataManifest =
        serde_yaml::from_str(&source).context("invalid GameData manifest YAML")?;

    if manifest.schema_version == 0 {
        bail!("schema_version must be > 0");
    }
    if manifest.content_version.trim().is_empty() {
        bail!("content_version cannot be empty");
    }
    if manifest.universe_generator_version == 0 {
        bail!("universe_generator_version must be > 0");
    }
    Ok(manifest)
}