use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::character::*;
use crate::world::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiomeDefinition {
    pub biome_id: String,
    pub display_name: String,
    pub allowed_terrains: Vec<String>,
    pub ambient_sounds: Vec<String>,
    pub base_temperature: f32,
    pub weather_types: Vec<String>,
    pub possible_mobs: Vec<MobSpawnRule>,
    pub resource_nodes: Vec<ResourceSpawnRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobSpawnRule {
    pub mob_id: String,
    pub weight: u32,
    pub min_level: u32,
    pub max_level: u32,
    pub active_hours: (u8, u8), // Ex: das 20h às 06h (noite)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpawnRule {
    pub resource_id: String,
    pub density_per_quadrant: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameDataRegistry {
    pub version_hash: String,
    pub character_creation: CharacterCreationConfig,
    pub attribute_scaling: AttributeScalingConfig,
    pub traits: HashMap<String, TraitDefinition>,
    pub reset_rules: ResetRulesConfig,
    pub universe: WorldUniverseConfig,
    pub zones: Vec<ZoneAreaDefinition>,
    pub biomes: HashMap<String, BiomeDefinition>,
    pub abyss_memory: AbyssMemoryConfig,
}

impl GameDataRegistry {
    /// Validação estrita para garantir que não existem inconsistências lógicas nos arquivos YAML
    pub fn validate(&self) -> Result<(), String> {
        if self.character_creation.starting_attribute_points == 0 {
            return Err("starting_attribute_points não pode ser zero".into());
        }
        if self.universe.quadrant_extent_cm <= 0.0 {
            return Err("quadrant_extent_cm precisa ser maior que zero".into());
        }
        if self.reset_rules.tiers.is_empty() {
            return Err("reset_rules precisa de ao menos um tier configurado".into());
        }
        if self.biomes.is_empty() {
            return Err("O registro de GameData precisa conter ao menos um bioma configurado".into());
        }
        Ok(())
    }
}

pub type SharedGameData = Arc<GameDataRegistry>;