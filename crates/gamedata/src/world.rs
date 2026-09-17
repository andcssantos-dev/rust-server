use serde::{Deserialize, Serialize};

/// Configurações gerais do mapa infinito
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldUniverseConfig {
    /// Tamanho físico de um quadrante em centímetros (Unreal Units)
    pub quadrant_extent_cm: f64,
    /// Raio de visão/streaming ativo de quadrantes ao redor do jogador
    pub client_reveal_radius_quadrants: i32,
    /// Distância da fronteira onde o scheduler começa a preparar em background
    pub async_prep_margin_quadrants: i32,
}

/// Zonas de jogo (Solitária, PvE, PvP, Zona Cinza)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneAreaDefinition {
    pub zone_id: String,
    pub zone_type: String, // "SOLITARY", "PVE", "PVP_RED", "GRAY_HUB"
    pub size_km_squared: u64,
    pub density_modifier: f32,
    pub allows_pvp: bool,
    pub permits_quadrant_claim_takeover: bool,
}

/// Memória do Abismo (Sistema Oculto)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbyssMemoryConfig {
    pub max_memory_score: f32,
    pub memory_decay_per_hour: f32,
    pub violence_kill_weight: f32,
    pub nature_felling_weight: f32,
    pub resource_waste_weight: f32,
}