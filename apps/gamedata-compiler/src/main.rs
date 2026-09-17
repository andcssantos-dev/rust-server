use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use aurenfall_core::DefinitionId;
use aurenfall_domain::items::ItemDefinition;
use serde::Deserialize;
use walkdir::WalkDir;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct RawItemFile {
    pub session: u32,
    pub category: String,
    pub items: Vec<RawItemDefinition>,
}

#[derive(Debug, Deserialize)]
struct RawDimensions {
    pub width: u8,
    pub height: u8,
}

#[derive(Debug, Deserialize)]
struct RawItemDefinition {
    pub id: u32,
    pub code_name: String,
    pub base_weight_grams: u32,
    pub dimensions: RawDimensions,
    pub max_durability: u32,
    pub combat: Option<aurenfall_domain::items::WeaponCombatStats>,
}

fn main() -> Result<()> {
    // 1. Mantém a validação inicial do manifest.yaml do Aurenfall
    let manifest_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("gamedata/manifest.yaml"));

    let manifest = aurenfall_gamedata::load_manifest(&manifest_path)?;
    println!(
        "[gamedata-compiler] GameData valid: content_version={}, generator_version={}",
        manifest.content_version, manifest.universe_generator_version
    );

    // 2. Compilação da base modular de itens
    let items_input_dir = Path::new("gamedata/items");
    let output_dir = Path::new("target/gamedata");
    fs::create_dir_all(output_dir)
        .context("Falha ao criar diretório de saída target/gamedata")?;

    let mut compiled_items: Vec<ItemDefinition> = Vec::new();
    let mut seen_ids: HashSet<u32> = HashSet::new();

    if items_input_dir.exists() {
        for entry in WalkDir::new(items_input_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file()
                && path.extension().map_or(false, |ext| ext == "yaml" || ext == "yml")
            {
                println!("[gamedata-compiler] -> Processando: {}", path.display());
                parse_and_validate_file(path, &mut compiled_items, &mut seen_ids)?;
            }
        }
    } else {
        println!(
            "[gamedata-compiler] Aviso: Diretório '{}' não encontrado. Pulando compilação de itens.",
            items_input_dir.display()
        );
    }

    println!(
        "[gamedata-compiler] Validados {} itens com sucesso.",
        compiled_items.len()
    );

    // 3. Emissão do arquivo binário otimizado
    let output_path = output_dir.join("items.bin");
    let encoded_data = bincode::serialize(&compiled_items)
        .context("Falha na serialização bincode dos itens")?;

    let mut file = File::create(&output_path)
        .with_context(|| format!("Falha ao criar arquivo {}", output_path.display()))?;
    file.write_all(&encoded_data)?;

    println!(
        "[gamedata-compiler] Binário emitido em: {} ({} bytes)",
        output_path.display(),
        encoded_data.len()
    );

    Ok(())
}

fn parse_and_validate_file(
    path: &Path,
    compiled_items: &mut Vec<ItemDefinition>,
    seen_ids: &mut HashSet<u32>,
) -> Result<()> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Não foi possível ler {}", path.display()))?;

    let raw_file: RawItemFile = serde_yaml::from_str(&content)
        .with_context(|| format!("Erro de parse YAML em {}", path.display()))?;

    for raw in raw_file.items {
        if !seen_ids.insert(raw.id) {
            bail!(
                "ID duplicado detectado: {} ('{}') em {}",
                raw.id,
                raw.code_name,
                path.display()
            );
        }

        let item = ItemDefinition {
            id: DefinitionId(raw.id),
            code_name: raw.code_name,
            base_weight_grams: raw.base_weight_grams,
            dimensions: aurenfall_domain::items::InventoryDimensions {
                width: raw.dimensions.width,
                height: raw.dimensions.height,
            },
            max_durability: raw.max_durability,
            max_stack: 1,
            equip_slot: None,
            combat: raw.combat,
        };

        compiled_items.push(item);
    }

    Ok(())
}