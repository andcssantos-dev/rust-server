use crate::MovementInput;
use crate::MovementRemainder;
use aurenfall_core::{CharacterId, WorldPositionMm};

/// Componente de posição base para todas as entidades espaciais.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position(pub WorldPositionMm);

/// Identidade autoritativa para personagens/jogadores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity(pub CharacterId);

/// Tag para identificar uma entidade como jogador.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Player;

/// Tag para identificar uma entidade como item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Item;

/// Intenção de movimento enviada pelo cliente ou gerada pela IA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveIntent(pub MovementInput);

/// Rastreia o resto sub-milimétrico fracionário acumulado entre ticks
/// para evitar perda de velocidade em cálculos de integração.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Remainder(pub(crate) MovementRemainder);

/// Tag de rastreamento para entidades modificadas no tick atual.
/// Deve ser adicionada ao mutar componentes e removida após sincronização.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dirty;

/// Estrutura leve para sincronização de atualizações espaciais.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialUpdate {
    pub identity: Identity,
    pub position: Position,
}

/// Marca uma entidade no ECS como um nó de mineração de pedra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RockNode {
    pub health: u32,
    pub max_health: u32,
}

impl RockNode {
    #[must_use]
    pub const fn new(health: u32) -> Self {
        Self {
            health,
            max_health: health,
        }
    }
}

/// Intenção enviada pelo jogador ao apertar o botão de minerar (E).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MineIntent {
    pub miner: CharacterId,
    pub power: u32,
}