use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configurações da tela de criação de personagem
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterCreationConfig {
    pub starting_attribute_points: u32,
    pub max_qualities_allowed: usize,
    pub max_defects_allowed: usize,
    pub base_attribute_minimum: u32,
    pub base_attribute_maximum: u32,
}

/// Fórmulas de conversão de Atributos Básicos
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeScalingConfig {
    // Força
    pub strength_attack_power_min: f32,
    pub strength_attack_power_max: f32,
    pub strength_carry_weight_bonus: f32,

    // Defesa
    pub defense_damage_reduction_pct: f32,
    pub defense_flat_armor: f32,

    // Agilidade
    pub agility_speed_factor: f32,
    pub agility_attack_speed_pct: f32,

    // Vitalidade
    pub vitality_health_per_point: f32,
    pub vitality_regen_per_second: f32,

    // Inteligência
    pub intelligence_energy_per_point: f32,
    pub intelligence_cooldown_reduction_pct: f32,
}

/// Representa uma Qualidade ou Defeito selecionável
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_defect: bool,
    /// Tags de contexto que influenciam o primeiro bioma procedural
    pub seed_context_tags: Vec<String>,
    /// Modificadores de regras em porcentagem ou valores fixos
    pub modifiers: HashMap<String, f32>,
}

/// Regras do ciclo de Reset (Estilo Mu Online)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetTierConfig {
    pub reset_count_min: u32,
    pub reset_count_max: u32,
    pub required_character_level: u32,
    pub bonus_attribute_points: u32,
    pub bonus_skill_points: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetRulesConfig {
    pub max_resets: u32,
    pub require_unequip_items: bool,
    pub retain_sub_skills: bool,
    pub retain_discovered_quadrants: bool,
    pub tiers: Vec<ResetTierConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubSkillsConfig {
    pub validation_rules: SubSkillValidationRules,
    pub sub_skills: HashMap<String, SubSkillDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubSkillValidationRules {
    pub allow_miss_action_xp: bool,
    pub allow_invalid_target_xp: bool,
    pub min_action_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubSkillDefinition {
    pub max_level: u32,
    pub base_xp_required: f32,
    pub xp_scaling_factor: f32,
    pub xp_per_valid_hit: f32,
}