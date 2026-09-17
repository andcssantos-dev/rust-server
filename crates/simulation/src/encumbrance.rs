//! Módulo de Sobrecarga e Efeitos Físicos do Inventário na Simulação (ECS).
//! 
//! Conecta a árvore de containers do personagem à física de movimento,
//! aplicando penalidades progressivas de velocidade, restrições de mobilidade
//! e permitindo a coleta segura de itens do chão do mundo.

use aurenfall_domain::containers::{
    CharacterContainerTree, ContainerId, ContainerInstance, EncumbranceTier, StoredItem,
};

/// Componente do ECS anexado à entidade do personagem.
/// Guarda a árvore viva de mochilas e o limite de peso corporal suportado.
#[derive(Debug, Clone)]
pub struct CharacterContainersComponent {
    /// A árvore completa com todas as mochilas equipadas e itens
    pub tree: CharacterContainerTree,
    /// Capacidade máxima recomendada de carga em gramas (ex: 30000 = 30 kg)
    pub max_capacity_grams: u32,
}

impl CharacterContainersComponent {
    /// Cria um novo componente com uma capacidade configurada em gramas.
    pub fn new(tree: CharacterContainerTree, max_capacity_grams: u32) -> Self {
        Self {
            tree,
            max_capacity_grams,
        }
    }

    /// Cria o inventário inicial padrão para novos personagens.
    /// Gera os bolsos básicos do corpo (grade 10x6) e limite padrão de 30 kg.
    pub fn default_starting(character_capacity_grams: u32) -> Self {
        let mut tree = CharacterContainerTree::new();
        // Container Raiz (ID 1: bolsos do cinto/corpo, 10 colunas por 6 linhas, peso 0g)
        let root = ContainerInstance::new_grid(ContainerId(1), 10, 6, 0);
        tree.set_root(root);

        Self {
            tree,
            max_capacity_grams: character_capacity_capacity_or_default(character_capacity_grams),
        }
    }

    /// Avalia a faixa de peso atual com base na capacidade configurada.
    pub fn current_tier(&self) -> EncumbranceTier {
        self.tree.get_encumbrance_tier(self.max_capacity_grams)
    }

    /// Calcula os efeitos físicos que devem ser aplicados à movimentação neste tick.
    pub fn evaluate_movement_effects(&self) -> MovementEncumbranceEffect {
        let tier = self.current_tier();
        MovementEncumbranceEffect::from_tier(tier)
    }

    /// Tenta coletar um item do mundo e guardá-lo no inventário do personagem.
    /// Se for bem-sucedido, retorna o novo estado físico de movimento atualizado.
    pub fn pickup_item(&mut self, item: StoredItem) -> Result<MovementEncumbranceEffect, &'static str> {
        let target_container = self.tree.root_id
            .ok_or("Personagem não possui container raiz configurado.")?;

        // Tenta inserir via Quick-Insert com suporte a rotação e agrupamento de pilhas
        self.tree.quick_insert(target_container, item, true)?;

        // Retorna imediatamente o novo efeito físico calculado após o acréscimo de peso
        Ok(self.evaluate_movement_effects())
    }
}

/// Função auxiliar para garantir uma capacidade mínima saudável.
fn character_capacity_capacity_or_default(capacity: u32) -> u32 {
    if capacity == 0 {
        30_000 // Padrão de 30 kg
    } else {
        capacity
    }
}

/// Efeitos físicos que o peso atual do inventário causa na simulação do personagem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementEncumbranceEffect {
    pub tier: EncumbranceTier,
    /// Multiplicador de velocidade (1.0 = normal, 0.6 = 60% da velocidade, etc.)
    pub speed_multiplier: f32,
    /// Se o personagem ainda tem fôlego/agilidade para disparar corrida rápida
    pub can_sprint: bool,
    /// Se o personagem consegue realizar rolamentos ou esquivas
    pub can_dodge: bool,
}

impl MovementEncumbranceEffect {
    /// Converte a faixa de sobrecarga nas penalidades mecânicas correspondentes.
    pub fn from_tier(tier: EncumbranceTier) -> Self {
        match tier {
            EncumbranceTier::Light => Self {
                tier,
                speed_multiplier: 1.0,
                can_sprint: true,
                can_dodge: true,
            },
            EncumbranceTier::Medium => Self {
                tier,
                speed_multiplier: 0.9,
                can_sprint: true,
                can_dodge: true,
            },
            EncumbranceTier::Heavy => Self {
                tier,
                speed_multiplier: 0.6,
                can_sprint: false, // Bloqueia corrida no estado Pesado
                can_dodge: true,
            },
            EncumbranceTier::Overburdened => Self {
                tier,
                speed_multiplier: 0.25, // Caminhada arrastada
                can_sprint: false,
                can_dodge: false, // Bloqueia esquivas quando sobrecarregado
            },
        }
    }

    /// Aplica a penalidade sobre uma velocidade base informada em milímetros por segundo.
    pub fn apply_to_speed(&self, base_speed_mm_per_sec: f32) -> f32 {
        base_speed_mm_per_sec * self.speed_multiplier
    }
}

/// Intenção de comando de movimento enviada pelo cliente ou pela inteligência artificial.
#[derive(Debug, Clone, Copy)]
pub struct CharacterMovementInput {
    pub wants_to_sprint: bool,
    pub wants_to_dodge: bool,
}

/// Resultado autoritativo calculado pelo servidor para o tick atual.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalculatedMovementTick {
    pub effective_speed_mm_per_sec: f32,
    pub is_sprinting: bool,
    pub dodge_allowed: bool,
}

/// Sistema autoritativo do ECS que processa o movimento considerando o peso do inventário.
pub fn process_movement_tick(
    containers: &CharacterContainersComponent,
    input: CharacterMovementInput,
    base_walk_speed_mm_per_sec: f32,
    sprint_bonus_multiplier: f32,
) -> CalculatedMovementTick {
    let effect = containers.evaluate_movement_effects();

    // 1. Valida se a corrida pode ser ativada
    let is_sprinting = input.wants_to_sprint && effect.can_sprint;

    // 2. Calcula a velocidade base (com bônus de corrida se permitido)
    let speed_before_encumbrance = if is_sprinting {
        base_walk_speed_mm_per_sec * sprint_bonus_multiplier
    } else {
        base_walk_speed_mm_per_sec
    };

    // 3. Aplica a penalidade de peso calculada
    let effective_speed = effect.apply_to_speed(speed_before_encumbrance);

    // 4. Valida se a esquiva solicitada é permitida
    let dodge_allowed = input.wants_to_dodge && effect.can_dodge;

    CalculatedMovementTick {
        effective_speed_mm_per_sec: effective_speed,
        is_sprinting,
        dodge_allowed,
    }
}

// =========================================================================
// TESTES AUTOMATIZADOS DO ECS
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_domain::containers::ItemCategory;

    #[test]
    fn test_encumbrance_movement_penalties() {
        let mut component = CharacterContainersComponent::default_starting(20_000);
        let base_speed = 5000.0; // 5000 mm/s = 5 m/s

        // 1. Vazio: Leve
        let effect = component.evaluate_movement_effects();
        assert_eq!(effect.tier, EncumbranceTier::Light);
        assert_eq!(effect.speed_multiplier, 1.0);
        assert!(effect.can_sprint);
        assert!(effect.can_dodge);
        assert_eq!(effect.apply_to_speed(base_speed), 5000.0);

        // 2. Coleta 14 kg de pedra (70% da capacidade): Médio
        let medium_rock = StoredItem::new(1, 100, ItemCategory::Ore, 14_000, 1, 1, 1, 1);
        let updated_effect = component.pickup_item(medium_rock).unwrap();
        assert_eq!(updated_effect.tier, EncumbranceTier::Medium);
        assert_eq!(updated_effect.speed_multiplier, 0.9);
        assert_eq!(updated_effect.apply_to_speed(base_speed), 4500.0);

        // 3. Coleta mais 4 kg (total 18 kg = 90%): Pesado
        let heavy_rock = StoredItem::new(2, 100, ItemCategory::Ore, 4_000, 1, 1, 1, 1);
        let updated_effect = component.pickup_item(heavy_rock).unwrap();
        assert_eq!(updated_effect.tier, EncumbranceTier::Heavy);
        assert_eq!(updated_effect.speed_multiplier, 0.6);
        assert!(!updated_effect.can_sprint, "Não deve permitir sprint no estado Pesado!");
        assert!(updated_effect.can_dodge);
        assert_eq!(updated_effect.apply_to_speed(base_speed), 3000.0);

        // 4. Coleta mais 5 kg (total 23 kg = 115%): Sobrecarregado!
        let extra_rock = StoredItem::new(3, 100, ItemCategory::Ore, 5_000, 1, 1, 1, 1);
        let updated_effect = component.pickup_item(extra_rock).unwrap();
        assert_eq!(updated_effect.tier, EncumbranceTier::Overburdened);
        assert_eq!(updated_effect.speed_multiplier, 0.25);
        assert!(!updated_effect.can_sprint);
        assert!(!updated_effect.can_dodge, "Não deve permitir esquiva quando sobrecarregado!");
        assert_eq!(updated_effect.apply_to_speed(base_speed), 1250.0);
    }

    #[test]
    fn test_authoritative_movement_tick_simulation() {
        let mut component = CharacterContainersComponent::default_starting(10_000);
        let walk_speed = 4000.0;
        let sprint_multiplier = 1.5;

        // 1. Cenário Leve
        let light_input = CharacterMovementInput {
            wants_to_sprint: true,
            wants_to_dodge: true,
        };
        let tick = process_movement_tick(&component, light_input, walk_speed, sprint_multiplier);
        assert!(tick.is_sprinting);
        assert!(tick.dodge_allowed);
        assert_eq!(tick.effective_speed_mm_per_sec, 6000.0);

        // 2. Coleta 9 kg de minério (90% de 10 kg -> Pesado)
        let heavy_ore = StoredItem::new(1, 100, ItemCategory::Ore, 9_000, 1, 1, 1, 1);
        component.pickup_item(heavy_ore).unwrap();

        let heavy_input = CharacterMovementInput {
            wants_to_sprint: true, // Tentando correr pesado
            wants_to_dodge: true,
        };
        let tick_heavy = process_movement_tick(&component, heavy_input, walk_speed, sprint_multiplier);
        assert!(!tick_heavy.is_sprinting, "Servidor autoritativo DEVE negar a corrida!");
        assert!(tick_heavy.dodge_allowed);
        assert_eq!(tick_heavy.effective_speed_mm_per_sec, 2400.0);

        // 3. Coleta mais 3 kg (total 12 kg = 120% -> Sobrecarregado)
        let extra_ore = StoredItem::new(2, 100, ItemCategory::Ore, 3_000, 1, 1, 1, 1);
        component.pickup_item(extra_ore).unwrap();

        let over_input = CharacterMovementInput {
            wants_to_sprint: false,
            wants_to_dodge: true, // Tentando esquivar sobrecarregado
        };
        let tick_over = process_movement_tick(&component, over_input, walk_speed, sprint_multiplier);
        assert!(!tick_over.dodge_allowed, "Servidor autoritativo DEVE bloquear esquiva!");
        assert_eq!(tick_over.effective_speed_mm_per_sec, 1000.0);
    }

    #[test]
    fn test_pickup_fails_gracefully_when_container_full() {
        // Cria um personagem com uma capacidade pequena e uma grade minúscula de 1x1
        let mut tree = CharacterContainerTree::new();
        let tiny_root = ContainerInstance::new_grid(ContainerId(1), 1, 1, 0);
        tree.set_root(tiny_root);
        let mut component = CharacterContainersComponent::new(tree, 5000);

        // O primeiro item de tamanho 1x1 entra e ocupa o único espaço
        let gem1 = StoredItem::new(1, 10, ItemCategory::Gem, 100, 1, 1, 1, 1);
        assert!(component.pickup_item(gem1).is_ok());

        // O segundo item tenta entrar, mas não cabe
        let gem2 = StoredItem::new(2, 11, ItemCategory::Gem, 100, 1, 1, 1, 1);
        let result = component.pickup_item(gem2);

        // O erro é retornado e o inventário continua com apenas 1 item íntegro
        assert_eq!(result, Err("Espaço insuficiente na mochila."));
        let root_ref = component.tree.containers.get(&ContainerId(1)).unwrap();
        assert_eq!(root_ref.items.len(), 1);
    }
}