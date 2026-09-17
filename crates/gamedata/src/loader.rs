use std::fs;
use std::path::Path;
use std::sync::Arc;
use blake3::Hasher;
use tracing::info;
use crate::schemas::*;

/// Registro Mestre mantido na memória RAM do servidor
#[derive(Debug, Clone)]
pub struct GameDataRegistry {
    /// Assinatura única (hash BLAKE3) dos arquivos carregados no disco
    pub content_hash: String,
    pub movement: MovementConfig,
    pub encumbrance: EncumbranceConfig,
    pub universe: UniverseConfig,
    pub abyss_memory: AbyssMemoryConfig,
    pub species: SpeciesDatabaseConfig,
    pub combat_ranks: CombatRanksConfig,
    pub character_creation: CharacterCreationConfig,
    pub attributes: AttributesConfig,
    pub reset_rules: ResetConfig,
    pub gathering: GatheringConfig,
    pub biomes: BiomeDatabaseConfig,
}

impl GameDataRegistry {
    /// Lê e valida todos os arquivos YAML da pasta informada (ex: "gamedata")
    pub fn load_from_directory<P: AsRef<Path>>(base_path: P) -> Result<Self, String> {
        let root = base_path.as_ref();
        let mut hasher = Hasher::new();

        // 1. Movimentação e Física
        let path = root.join("character/movement.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let movement: MovementConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 2. Sobrecarga e Peso
        let path = root.join("character/encumbrance.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let encumbrance: EncumbranceConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 3. Dimensões do Universo e Streaming
        let path = root.join("world/universe.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let universe: UniverseConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 4. Memória do Abismo (Sistema Oculto)
        let path = root.join("world/abyss_memory.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let abyss_memory: AbyssMemoryConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 5. Espécies de Criaturas
        let path = root.join("creatures/species.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let species: SpeciesDatabaseConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 6. Ranks de Combate
        let path = root.join("creatures/combat_ranks.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let combat_ranks: CombatRanksConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 7. Criação de Personagem
        let path = root.join("character/creation.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let character_creation: CharacterCreationConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 8. Fórmulas de Atributos Básicos
        let path = root.join("character/attributes.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let attributes: AttributesConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 9. Regras de Reset
        let path = root.join("character/reset.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let reset_rules: ResetConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 10. Recursos Coletáveis
        let path = root.join("gathering").join("resources.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let gathering: GatheringConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        // 11. Biomas e Ecologia do Mundo
        let path = root.join("world/biomes.yaml");
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Falha ao ler {}: {}", path.display(), e))?;
        hasher.update(raw.as_bytes());
        let biomes: BiomeDatabaseConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("Erro de sintaxe em {}: {}", path.display(), e))?;

        let content_hash = hasher.finalize().to_hex().to_string();

        info!("GameData carregado e validado com sucesso! Hash BLAKE3: [{}]", content_hash);

        let registry = Self {
            content_hash,
            movement,
            encumbrance,
            universe,
            abyss_memory,
            species,
            combat_ranks,
            character_creation,
            attributes,
            reset_rules,
            gathering,
            biomes,
        };

        registry.validate()?;
        Ok(registry)
    }

    /// Validação estrita das regras para evitar iniciar com valores inconsistentes
    pub fn validate(&self) -> Result<(), String> {
        if self.movement.speeds.speed_mm_per_second == 0 {
            return Err("A velocidade de movimento (speed_mm_per_second) precisa ser maior que zero.".into());
        }
        if self.universe.dimensions.quadrant_side_length_cm <= 0.0 {
            return Err("O tamanho do quadrante precisa ser maior que zero.".into());
        }
        if self.encumbrance.thresholds.is_empty() {
            return Err("É necessário definir ao menos uma faixa de sobrecarga de peso.".into());
        }
        if self.character_creation.starting_attribute_points == 0 {
            return Err("starting_attribute_points não pode ser zero.".into());
        }
        if self.combat_ranks.combat_ranks.is_empty() {
            return Err("Pelo menos um combat_rank deve ser registrado.".into());
        }
        if self.gathering.resources.is_empty() {
            return Err("É necessário registrar ao menos um recurso coletável.".into());
        }
        if self.biomes.biomes.is_empty() {
            return Err("É necessário registrar ao menos um bioma em world/biomes.yaml.".into());
        }
        let mut seen_numeric_ids = std::collections::HashSet::new();
        for (name, config) in &self.biomes.biomes {
            if config.numeric_id == 0 {
                return Err(format!("O bioma '{}' possui numeric_id inválido (zero).", name));
            }
            if !seen_numeric_ids.insert(config.numeric_id) {
                return Err(format!(
                    "Conflito de ID: O numeric_id '{}' do bioma '{}' já está em uso por outro bioma.",
                    config.numeric_id, name
                ));
            }
        }
        Ok(())
    }
}

pub type SharedGameData = Arc<GameDataRegistry>;