use aurenfall_core::{CharacterId, DefinitionId, ItemInstanceId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use crate::Ownership;

/// Estado físico dinâmico de durabilidade do item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurabilityState {
    pub current: u32,
    pub max: u32,
}

/// Nível de aprimoramento (+0, +1, +7...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct EnhancementLevel(pub u8);

/// Modificador procedual único (afixo positivo ou defixo negativo) materializado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolledModifier {
    pub modifier_id: u32,
    pub tier: u8,
    pub rolled_value: f32,
}

/// Contadores agregados do ciclo de vida que formam o valor histórico da peça.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ItemHistoricalCounters {
    pub kill_count: u32,
    pub repair_count: u32,
    pub owner_count: u32,
}

/// A representação física e autoritativa de um objeto único no mundo de Aurenfall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemInstance {
    pub id: ItemInstanceId,
    pub definition_id: DefinitionId,
    pub owner: Ownership,
    pub revision: u64,
    pub quantity: u32,

    // Semente determinística usada na rolagem de atributos originais
    pub seed: u64,

    // Condição e aprimoramento
    pub durability: DurabilityState,
    pub enhancement: EnhancementLevel,

    // Composição procedural e modificadores resolvidos
    pub modifiers: Vec<RolledModifier>,

    // Soquetes contendo outras instâncias (ex: gemas ou chips instalados)
    pub sockets: Vec<Option<ItemInstanceId>>,

    // Histórico e causalidade acumulada
    pub history: ItemHistoricalCounters,
}

/// Ocupação em células na grade do inventário (ex: 1x3, 2x4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryDimensions {
    pub width: u8,
    pub height: u8,
}

/// Tipo de empunhadura do item para conectar com os sockets e animações da UE5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GripType {
    None,
    MeleeOneHand,
    MeleeTwoHand,
    OneHandedPistol,
    TwoHandedRifle,
    Shield,
}

/// Dados base de combate da arma.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeaponCombatStats {
    pub grip: GripType,
    pub base_damage: f32,
    pub attack_speed: f32,
    pub range_meters: f32,
    pub stamina_cost: f32,
}

/// A definição canônica e imutável de um item (lida do GameData).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemDefinition {
    pub id: DefinitionId,
    pub code_name: String,
    pub base_weight_grams: u32,
    pub dimensions: InventoryDimensions,
    pub max_durability: u32,
    pub max_stack: u32,
    pub combat: Option<WeaponCombatStats>,
    pub equip_slot: Option<EquipmentSlotKind>,
}

/// Valores numéricos finais consolidados para uso imediato no tick de combate.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedWeaponStats {
    pub grip: GripType,
    pub effective_damage: f32,
    pub attack_speed: f32,
    pub range_meters: f32,
    pub stamina_cost: f32,
}

impl ItemInstance {
    #[must_use]
    pub fn new_unique(
        id: ItemInstanceId,
        definition_id: DefinitionId,
        owner: Ownership,
        seed: u64,
        max_durability: u32,
    ) -> Self {
        Self {
            id,
            definition_id,
            owner,
            revision: 1,
            quantity: 1,
            seed,
            durability: DurabilityState {
                current: max_durability,
                max: max_durability,
            },
            enhancement: EnhancementLevel(0),
            modifiers: Vec::new(),
            sockets: Vec::new(),
            history: ItemHistoricalCounters::default(),
        }
    }

    #[must_use]
    pub fn new_stackable(
        id: ItemInstanceId,
        definition_id: DefinitionId,
        owner: Ownership,
        quantity: u32,
    ) -> Self {
        Self {
            id,
            definition_id,
            owner,
            revision: 1,
            quantity,
            seed: 0,
            durability: DurabilityState { current: 0, max: 0 },
            enhancement: EnhancementLevel(0),
            modifiers: Vec::new(),
            sockets: Vec::new(),
            history: ItemHistoricalCounters::default(),
        }
    }

    /// Transfere a custódia da peça de forma autoritativa mantendo toda a sua identidade.
    pub fn transfer_ownership(&mut self, new_owner: Ownership) {
        if self.owner != new_owner {
            self.owner = new_owner;
            self.history.owner_count = self.history.owner_count.saturating_add(1);
            self.revision = self.revision.saturating_add(1);
        }
    }

    /// Resolve os atributos finais de combate combinando template estático com a instância dinâmica.
    pub fn resolve_weapon_stats(&self, template: &ItemDefinition) -> Option<ResolvedWeaponStats> {
        let combat = template.combat.as_ref()?;

        let upgrade_mult = 1.0 + (self.enhancement.0 as f32 * 0.06);
        let mut damage = combat.base_damage * upgrade_mult;

        if self.durability.current == 0 {
            damage *= 0.30;
        }

        for modifier in &self.modifiers {
            if modifier.modifier_id == 101 {
                damage *= 1.0 + (modifier.rolled_value / 100.0);
            }
        }

        Some(ResolvedWeaponStats {
            grip: combat.grip,
            effective_damage: damage,
            attack_speed: combat.attack_speed,
            range_meters: combat.range_meters,
            stamina_cost: combat.stamina_cost,
        })
    }
}

/// Identificador único da entidade da mochila caída no mundo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DroppedBagId(pub u64);

/// Entidade estática deixada no quadrante quando um jogador morre.
/// Atua como container temporário das instâncias de itens reais.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DroppedBagEntity {
    pub id: DroppedBagId,
    pub original_owner: CharacterId,
    pub quadrant_x: i32,
    pub quadrant_y: i32,
    pub world_x: f32,
    pub world_y: f32,
    pub spawned_at_tick: u64,
    pub decay_after_ticks: u64,
    pub items: Vec<ItemInstance>,
}

impl DroppedBagEntity {
    #[must_use]
    pub fn new(
        id: DroppedBagId,
        owner: CharacterId,
        quadrant: (i32, i32),
        world_pos: (f32, f32),
        current_tick: u64,
        ttl_ticks: u64,
        inventory_items: Vec<ItemInstance>,
    ) -> Self {
        Self {
            id,
            original_owner: owner,
            quadrant_x: quadrant.0,
            quadrant_y: quadrant.1,
            world_x: world_pos.0,
            world_y: world_pos.1,
            spawned_at_tick: current_tick,
            decay_after_ticks: ttl_ticks,
            items: inventory_items,
        }
    }

    /// Verifica se a mochila já atingiu o tempo limite de permanência no chão.
    #[inline]
    pub fn is_expired(&self, current_tick: u64) -> bool {
        current_tick.saturating_sub(self.spawned_at_tick) >= self.decay_after_ticks
    }

    /// Extrai uma instância da mochila pelo ID mantendo a integridade dos itens restantes.
    pub fn take_item(&mut self, item_id: ItemInstanceId) -> Option<ItemInstance> {
        let index = self.items.iter().position(|i| i.id == item_id)?;
        Some(self.items.swap_remove(index))
    }
}

/// Gerenciador de mochilas e containers caídos no mundo indexados por quadrante.
#[derive(Debug, Default)]
pub struct DroppedBagManager {
    /// Mapeamento direto de BagId para a entidade completa.
    bags: HashMap<DroppedBagId, DroppedBagEntity>,
    /// Spatial Hash Grid: coordenadas do quadrante (qx, qy) -> lista de IDs de mochilas presentes.
    spatial_grid: HashMap<(i32, i32), Vec<DroppedBagId>>,
}

impl DroppedBagManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra uma nova mochila no mundo e a indexa no quadrante correto.
    pub fn spawn_bag(&mut self, bag: DroppedBagEntity) {
        let bag_id = bag.id;
        let coord = (bag.quadrant_x, bag.quadrant_y);

        self.bags.insert(bag_id, bag);
        self.spatial_grid.entry(coord).or_default().push(bag_id);
    }

    /// Retorna uma referência somente leitura à mochila para inspeção visual ou checagens.
    #[must_use]
    pub fn get_bag(&self, bag_id: DroppedBagId) -> Option<&DroppedBagEntity> {
        self.bags.get(&bag_id)
    }

    /// Retorna uma referência mutável à mochila para saque e modificação.
    pub fn get_bag_mut(&mut self, bag_id: DroppedBagId) -> Option<&mut DroppedBagEntity> {
        self.bags.get_mut(&bag_id)
    }

    /// Retorna todas as mochilas contidas em um quadrante específico.
    pub fn get_bags_in_quadrant(&self, quadrant: (i32, i32)) -> Vec<&DroppedBagEntity> {
        self.spatial_grid
            .get(&quadrant)
            .map(|ids| ids.iter().filter_map(|id| self.bags.get(id)).collect())
            .unwrap_or_default()
    }

    /// Executa o tick do coletor de lixo: remove mochilas expiradas da memória
    /// e retorna os IDs das entidades removidas para notificar clientes/banco de dados.
    pub fn tick_decay(&mut self, current_tick: u64) -> Vec<DroppedBagId> {
        let mut expired_ids = Vec::new();

        // 1. Identifica quais mochilas estouraram o TTL
        for (id, bag) in &self.bags {
            if bag.is_expired(current_tick) {
                expired_ids.push(*id);
            }
        }

        // 2. Remove as mochilas expiradas do storage e da Spatial Grid
        for id in &expired_ids {
            if let Some(removed_bag) = self.bags.remove(id) {
                let coord = (removed_bag.quadrant_x, removed_bag.quadrant_y);
                if let Some(grid_list) = self.spatial_grid.get_mut(&coord) {
                    grid_list.retain(|b_id| b_id != id);
                    if grid_list.is_empty() {
                        self.spatial_grid.remove(&coord);
                    }
                }
            }
        }

        expired_ids
    }


}

/// Coordenada da célula superior esquerda de inserção na grade do inventário.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GridSlotCoord {
    pub x: u8,
    pub y: u8,
}

/// Registro da localização de uma peça na grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridPlacedItem {
    pub origin: GridSlotCoord,
    pub rotation: ItemRotation,
    pub dimensions: InventoryDimensions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryGrid {
    pub columns: u8,
    pub rows: u8,
    cells: Vec<Option<ItemInstanceId>>,
    placements: HashMap<ItemInstanceId, GridPlacedItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridPlacementError {
    OutOfBounds,
    Collision,
}

impl InventoryGrid {
    #[must_use]
    pub fn new(columns: u8, rows: u8) -> Self {
        let total_cells = (columns as usize) * (rows as usize);
        Self {
            columns,
            rows,
            cells: vec![None; total_cells],
            placements: HashMap::new(),
        }
    }

    #[inline]
    fn index_of(&self, x: u8, y: u8) -> Option<usize> {
        if x >= self.columns || y >= self.rows {
            None
        } else {
            Some((y as usize * self.columns as usize) + x as usize)
        }
    }

    #[must_use]
    pub fn get_placement(&self, item_id: ItemInstanceId) -> Option<&GridPlacedItem> {
        self.placements.get(&item_id)
    }

    #[must_use]
    pub fn get_item_at(&self, slot: GridSlotCoord) -> Option<ItemInstanceId> {
        let idx = self.index_of(slot.x, slot.y)?;
        self.cells[idx]
    }

    /// Valida se uma peça cabe na coordenada com a rotação solicitada.
    /// Se `ignore_item` for passado (ex: o próprio item durante rotação/deslocamento), suas células atuais são ignoradas.
    pub fn can_place_at(
        &self,
        origin: GridSlotCoord,
        base_dimensions: InventoryDimensions,
        rotation: ItemRotation,
        ignore_item: Option<ItemInstanceId>,
    ) -> Result<(), GridPlacementError> {
        let eff = base_dimensions.oriented(rotation);

        if origin.x.saturating_add(eff.width) > self.columns
            || origin.y.saturating_add(eff.height) > self.rows
        {
            return Err(GridPlacementError::OutOfBounds);
        }

        for dy in 0..eff.height {
            for dx in 0..eff.width {
                let cell_x = origin.x + dy_offset(dx);
                let cell_y = origin.y + dy;
                let idx = self.index_of(cell_x, cell_y).ok_or(GridPlacementError::OutOfBounds)?;

                if let Some(occupant) = self.cells[idx] {
                    if Some(occupant) != ignore_item {
                        return Err(GridPlacementError::Collision);
                    }
                }
            }
        }

        Ok(())
    }

    /// Posiciona o item na grade atualizando a matriz de ocupação e o registro de posição.
    pub fn place_item(
        &mut self,
        item_id: ItemInstanceId,
        origin: GridSlotCoord,
        base_dimensions: InventoryDimensions,
        rotation: ItemRotation,
    ) -> Result<(), GridPlacementError> {
        self.can_place_at(origin, base_dimensions, rotation, None)?;
        let eff = base_dimensions.oriented(rotation);

        for dy in 0..eff.height {
            for dx in 0..eff.width {
                if let Some(idx) = self.index_of(origin.x + dx, origin.y + dy) {
                    self.cells[idx] = Some(item_id);
                }
            }
        }

        self.placements.insert(
            item_id,
            GridPlacedItem {
                origin,
                rotation,
                dimensions: base_dimensions,
            },
        );

        Ok(())
    }

    /// Remove a peça da grade liberando os slots ocupados.
    pub fn remove_item(&mut self, item_id: ItemInstanceId) -> bool {
        let Some(placement) = self.placements.remove(&item_id) else {
            return false;
        };

        let eff = placement.dimensions.oriented(placement.rotation);
        for dy in 0..eff.height {
            for dx in 0..eff.width {
                if let Some(idx) = self.index_of(placement.origin.x + dx, placement.origin.y + dy) {
                    if self.cells[idx] == Some(item_id) {
                        self.cells[idx] = None;
                    }
                }
            }
        }

        true
    }

    /// Rotaciona a peça no local atual ou reposiciona para novo slot com rotação.
    pub fn rotate_or_move(
        &mut self,
        item_id: ItemInstanceId,
        new_origin: GridSlotCoord,
        new_rotation: ItemRotation,
    ) -> Result<(), GridPlacementError> {
        let placement = self.placements.get(&item_id).copied().ok_or(GridPlacementError::OutOfBounds)?;
        self.can_place_at(new_origin, placement.dimensions, new_rotation, Some(item_id))?;

        self.remove_item(item_id);
        self.place_item(item_id, new_origin, placement.dimensions, new_rotation)
    }
}

#[inline]
const fn dy_offset(dx: u8) -> u8 {
    dx
}

/// Orientação geométrica de uma peça na grade 2D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ItemRotation {
    #[default]
    Normal,
    Rotated90,
}

impl InventoryDimensions {
    /// Retorna as dimensões efetivas considerando a orientação na grade.
    #[must_use]
    pub const fn oriented(self, rotation: ItemRotation) -> Self {
        match rotation {
            ItemRotation::Normal => self,
            ItemRotation::Rotated90 => Self {
                width: self.height,
                height: self.width,
            },
        }
    }
}

/// Slots anatômicos disponíveis no Paperdoll do personagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EquipmentSlotKind {
    Head,
    Chest,
    Hands,
    Legs,
    Feet,
    MainHand,
    OffHand,
}

/// Estado dos slots de equipamento vestíveis do personagem.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CharacterEquipment {
    pub slots: HashMap<EquipmentSlotKind, ItemInstance>,
}

impl CharacterEquipment {
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }

    #[must_use]
    pub fn get_equipped(&self, slot: EquipmentSlotKind) -> Option<&ItemInstance> {
        self.slots.get(&slot)
    }

    pub fn equip_item(&mut self, slot: EquipmentSlotKind, item: ItemInstance) -> Option<ItemInstance> {
        self.slots.insert(slot, item)
    }

    pub fn unequip_item(&mut self, slot: EquipmentSlotKind) -> Option<ItemInstance> {
        self.slots.remove(&slot)
    }
}

/// Estado persistível completo do inventário e equipamentos do personagem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterInventoryState {
    pub grid: InventoryGrid,
    pub equipment: CharacterEquipment,
    pub items: HashMap<ItemInstanceId, ItemInstance>,
}

impl CharacterInventoryState {
    #[must_use]
    pub fn new(columns: u8, rows: u8) -> Self {
        Self {
            grid: InventoryGrid::new(columns, rows),
            equipment: CharacterEquipment::new(),
            items: HashMap::new(),
        }
    }
}

#[test]
fn test_inventory_grid_placement_and_collision() {
    let mut grid = InventoryGrid::new(4, 4);
    let rifle_id = ItemInstanceId(10);
    let pistol_id = ItemInstanceId(20);

    let rifle_dim = InventoryDimensions { width: 1, height: 3 };
    let pistol_dim = InventoryDimensions { width: 2, height: 1 };

    // 1. Inserção bem-sucedida do rifle em (0, 0)
    assert!(grid.place_item(rifle_id, GridSlotCoord { x: 0, y: 0 }, rifle_dim, ItemRotation::Normal).is_ok());

    // 2. Colisão: tentar colocar a pistola em (0, 1) onde o rifle já ocupa
    assert_eq!(
        grid.place_item(pistol_id, GridSlotCoord { x: 0, y: 1 }, pistol_dim, ItemRotation::Normal),
        Err(GridPlacementError::Collision)
    );

    // 3. Fora dos limites: colocar fora da borda (3, 0) com largura 2
    assert_eq!(
        grid.place_item(pistol_id, GridSlotCoord { x: 3, y: 0 }, pistol_dim, ItemRotation::Normal),
        Err(GridPlacementError::OutOfBounds)
    );

    // 4. Inserção válida da pistola em (1, 0)
    assert!(grid.place_item(pistol_id, GridSlotCoord { x: 1, y: 0 }, pistol_dim, ItemRotation::Normal).is_ok());

    // 5. Remoção do rifle e liberação da célula (0, 1)
    assert!(grid.remove_item(rifle_id));
    assert!(grid.can_place_at(GridSlotCoord { x: 0, y: 1 }, pistol_dim, ItemRotation::Normal, None).is_ok());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_weapon_stats_with_enhancement() {
        let def_id = DefinitionId(1001);
        let template = ItemDefinition {
            id: def_id,
            code_name: "pipe_iron_rusty".to_string(),
            base_weight_grams: 1800,
            dimensions: InventoryDimensions { width: 1, height: 3 },
            max_durability: 120,
            max_stack: 1,
            equip_slot: Some(EquipmentSlotKind::MainHand),
            combat: Some(WeaponCombatStats {
                grip: GripType::MeleeOneHand,
                base_damage: 26.0,
                attack_speed: 1.15,
                range_meters: 1.8,
                stamina_cost: 14.0,
            }),
        };

        let mut instance = ItemInstance::new_unique(
            ItemInstanceId(1),
            def_id,
            Ownership::World { sector: aurenfall_core::SectorCoord { x: 0, y: 0 } },
            42,
            120,
        );

        instance.enhancement = EnhancementLevel(2);

        let resolved = instance.resolve_weapon_stats(&template).unwrap();
        assert!((resolved.effective_damage - (26.0 * 1.12)).abs() < 0.001);
    }

    #[test]
    fn test_dropped_bag_lifecycle_and_take() {
        let char_id = CharacterId(101);
        let bag_id = DroppedBagId(500);
        let def_id = DefinitionId(1001);

        let item = ItemInstance::new_unique(
            ItemInstanceId(999),
            def_id,
            Ownership::Character(char_id),
            12345,
            120,
        );

        let mut bag = DroppedBagEntity::new(
            bag_id,
            char_id,
            (4, -2),
            (150.5, -320.0),
            1000,
            600,
            vec![item],
        );

        assert!(!bag.is_expired(1500));
        assert!(bag.is_expired(1600));

        let taken = bag.take_item(ItemInstanceId(999));
        assert!(taken.is_some());
        assert_eq!(bag.items.len(), 0);

        let mut taken_item = taken.unwrap();
        let looter_id = CharacterId(202);
        taken_item.transfer_ownership(Ownership::Character(looter_id));

        assert_eq!(taken_item.owner, Ownership::Character(looter_id));
        assert_eq!(taken_item.history.owner_count, 1);
        assert_eq!(taken_item.revision, 2);
    }

    #[test]
    fn test_dropped_bag_manager_spatial_and_decay() {
        let mut manager = DroppedBagManager::new();
        let bag_id = DroppedBagId(1);
        let quadrant = (2, 3);

        let bag = DroppedBagEntity::new(
            bag_id,
            CharacterId(10),
            quadrant,
            (210.0, 315.0),
            100,
            50,
            Vec::new(),
        );

        manager.spawn_bag(bag);

        let in_quadrant = manager.get_bags_in_quadrant(quadrant);
        assert_eq!(in_quadrant.len(), 1);
        assert_eq!(in_quadrant[0].id, bag_id);

        let removed = manager.tick_decay(140);
        assert!(removed.is_empty());
        assert_eq!(manager.get_bags_in_quadrant(quadrant).len(), 1);

        let removed = manager.tick_decay(150);
        assert_eq!(removed, vec![bag_id]);
        assert_eq!(manager.get_bags_in_quadrant(quadrant).len(), 0);
    }
}