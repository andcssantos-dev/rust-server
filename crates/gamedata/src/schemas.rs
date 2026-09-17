use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// -----------------------------------------------------------------------------
// 1. ESTRUTURAS DE FÍSICA E MOVIMENTO
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovementSpeedsConfig {
    /// Velocidade padrão de caminhada em milímetros por segundo (ex: 4000 mm/s = 4 m/s)
    pub speed_mm_per_second: u32,
    /// Velocidade de corrida rápida em milímetros por segundo (sprint)
    pub sprint_speed_mm_per_second: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovementPhysicsConfig {
    /// Quantidade de ticks sem novo comando antes de zerar o input
    pub input_timeout_ticks: u64,
    /// Raio de colisão do corpo do personagem em milímetros (ex: 250 mm = 25 cm)
    pub character_radius_mm: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovementConfig {
    pub speeds: MovementSpeedsConfig,
    pub physics: MovementPhysicsConfig,
}

// -----------------------------------------------------------------------------
// 2. ESTRUTURAS DE PESO E SOBRECARGA
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncumbranceThreshold {
    pub trigger_percentage: f32,
    pub speed_multiplier: f32,
    pub stamina_drain_multiplier: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncumbranceBaseCapacity {
    pub starting_max_weight_kg: f32,
    pub bonus_weight_per_strength_point: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncumbranceConfig {
    pub base_capacity: EncumbranceBaseCapacity,
    pub thresholds: Vec<EncumbranceThreshold>,
}

// -----------------------------------------------------------------------------
// 3. ESTRUTURAS DO UNIVERSO E STREAMING
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseDimensions {
    pub total_universe_extent_km: u64,
    pub quadrant_side_length_cm: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseStreaming {
    pub client_reveal_radius_quadrants: i32,
    pub server_prep_margin_quadrants: i32,
    pub quadrant_cache_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseConfig {
    pub dimensions: UniverseDimensions,
    pub streaming: UniverseStreaming,
}

// -----------------------------------------------------------------------------
// 4. SCHEMAS DA MEMÓRIA DO ABISMO (SISTEMA OCULTO)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbyssMemoryThresholds {
    pub max_score_cap: f32,
    pub passive_decay_per_hour: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbyssMemoryWeights {
    pub violence_per_kill: f32,
    pub nature_deforestation_per_tree: f32,
    pub waste_per_abandoned_resource: f32,
    pub preservation_per_action: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbyssMemoryConfig {
    pub thresholds: AbyssMemoryThresholds,
    pub weights: AbyssMemoryWeights,
}

// -----------------------------------------------------------------------------
// 4. SCHEMAS DE COMPOSIÇÃO DE CRIATURAS
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatureBodyProfile {
    pub base_scale: f32,
    pub scale_variance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatureNaturalDefenses {
    pub base_armor: f32,
    pub poison_resistance_pct: f32,
    pub fire_resistance_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatureLocomotion {
    pub walk_speed_cm_s: f32,
    pub run_speed_cm_s: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatureSenses {
    pub vision_range_meters: f32,
    pub hearing_range_meters: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatureSpeciesDefinition {
    pub id: String,
    pub name: String,
    pub creature_type: String,
    pub body: CreatureBodyProfile,
    pub natural_defenses: CreatureNaturalDefenses,
    pub locomotion: CreatureLocomotion,
    pub senses: CreatureSenses,
    pub allowed_archetypes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeciesDatabaseConfig {
    pub species: Vec<CreatureSpeciesDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatRankDefinition {
    pub id: String,
    pub name: String,
    pub health_multiplier: f32,
    pub damage_multiplier: f32,
    pub armor_multiplier: f32,
    pub trait_budget: u32,
    pub reward_multiplier: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatRanksConfig {
    pub combat_ranks: Vec<CombatRankDefinition>,
}

// -----------------------------------------------------------------------------
// 5. SCHEMAS DE PERSONAGEM
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterCreationConfig {
    pub starting_attribute_points: u32,
    pub max_qualities_allowed: usize,
    pub max_defects_allowed: usize,
    pub base_attribute_minimum: u32,
    pub base_attribute_maximum: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrengthScaling {
    pub attack_power_min_per_point: f32,
    pub attack_power_max_per_point: f32,
    pub carry_weight_bonus_kg: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefenseScaling {
    pub damage_reduction_pct_per_point: f32,
    pub flat_armor_per_point: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgilityScaling {
    pub move_speed_bonus_per_point: f32,
    pub attack_speed_bonus_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VitalityScaling {
    pub health_points_per_point: f32,
    pub health_regen_per_second: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceScaling {
    pub energy_points_per_point: f32,
    pub energy_regen_bonus_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributesConfig {
    pub strength: StrengthScaling,
    pub defense: DefenseScaling,
    pub agility: AgilityScaling,
    pub vitality: VitalityScaling,
    pub intelligence: IntelligenceScaling,
}

// -----------------------------------------------------------------------------
// 6. SCHEMAS DE RESET DE PERSONAGEM
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetTier {
    pub reset_min: u32,
    pub reset_max: u32,
    pub required_level: u32,
    pub bonus_attribute_points: u32,
    pub bonus_skill_points: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetGeneral {
    pub max_resets_allowed: u32,
    pub require_unequip_items: bool,
    pub retain_sub_skills: bool,
    pub retain_discovered_quadrants: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetConfig {
    pub general: ResetGeneral,
    pub tiers: Vec<ResetTier>,
}

// -----------------------------------------------------------------------------
// 7. SCHEMAS DE RECURSOS COLETÁVEIS (MINERAÇÃO E MADEIRA)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDropEntry {
    pub item_id: String,
    pub min_quantity: u32,
    pub max_quantity: u32,
    pub drop_chance_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDefinition {
    pub id: String,
    pub name: String,
    pub resource_type_id: u64,
    pub max_health: u32,
    pub required_tool_type: String,
    pub minimum_tool_tier: u32,
    pub base_xp_reward: f32,
    pub drops: Vec<ResourceDropEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatheringConfig {
    pub resources: Vec<ResourceDefinition>,
}

// -----------------------------------------------------------------------------
// 8. SCHEMAS DOS BIOMAS E AMBIENTES
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiomeAdvancedConfig {
    pub biome_id: String,
    pub numeric_id: u8,
    pub display_name: String,
    pub description: String,
    pub allowed_terrains: Vec<String>,
    pub structural_features: Vec<String>,
    pub base_temperature: f32,
    pub weather_types: Vec<String>,
    pub ambient_sounds: Vec<String>,
    pub environmental_modifiers: Vec<EnvironmentalModifier>,
    pub possible_mobs: Vec<AdvancedMobSpawnRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentalModifier {
    pub effect_id: String,
    pub interval_seconds: u32,
    pub attribute_impact: String,
    pub value_change: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedMobSpawnRule {
    pub mob_id: String,
    pub weight: u32,
    pub min_level: u32,
    pub max_level: u32,
    pub active_time_range: (u8, u8),
    pub quadrant_density_cap: u32,
}

// -----------------------------------------------------------------------------
// 9. BANCO DE DADOS DE BIOMAS DO MUNDO
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiomeDatabaseConfig {
    pub biomes: HashMap<String, BiomeAdvancedConfig>,
}