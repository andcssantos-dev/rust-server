use aurenfall_contracts::inventory::{
    BagContentsSnapshot, DropResult, InventoryActionResult, LootResult, RequestDropItem,
    RequestEquipItem, RequestLootItem, RequestMergeStack, RequestMoveOrRotateItem,
    RequestOpenBag, RequestSplitStack, RequestUnequipItem,
};
use aurenfall_core::{CharacterId, ItemInstanceId};
use aurenfall_domain::items::{
    CharacterEquipment, DroppedBagEntity, DroppedBagId, DroppedBagManager, EquipmentSlotKind,
    InventoryDimensions, InventoryGrid, ItemInstance,
};
use aurenfall_domain::Ownership;

#[derive(Debug, Clone, PartialEq)]
pub enum OpenBagResult {
    Success(BagContentsSnapshot),
    BagNotFound,
    OutOfRange,
}

#[derive(Debug)]
pub struct LootService;

impl LootService {
    pub const MAX_INTERACTION_DISTANCE_METERS: f32 = 3.5;

    pub fn handle_open_bag_request(
        manager: &DroppedBagManager,
        request: RequestOpenBag,
        player_pos: (f32, f32),
    ) -> OpenBagResult {
        let Some(bag) = manager.get_bag(request.bag_id) else {
            return OpenBagResult::BagNotFound;
        };

        let dx = bag.world_x - player_pos.0;
        let dy = bag.world_y - player_pos.1;
        let dist_sq = (dx * dx) + (dy * dy);
        let max_dist_sq =
            Self::MAX_INTERACTION_DISTANCE_METERS * Self::MAX_INTERACTION_DISTANCE_METERS;

        if dist_sq > max_dist_sq {
            return OpenBagResult::OutOfRange;
        }

        OpenBagResult::Success(BagContentsSnapshot {
            bag_id: bag.id,
            items: bag.items.clone(),
        })
    }

    /// Saque de item com suporte a rotação estilo SCUM/Tarkov.
    pub fn handle_loot_request<F>(
        manager: &mut DroppedBagManager,
        player_grid: &mut InventoryGrid,
        request: RequestLootItem,
        player_id: CharacterId,
        player_pos: (f32, f32),
        get_dimensions: F,
    ) -> LootResult
    where
        F: FnOnce(aurenfall_core::DefinitionId) -> Option<InventoryDimensions>,
    {
        let Some(bag) = manager.get_bag_mut(request.bag_id) else {
            return LootResult::BagNotFound;
        };

        let dx = bag.world_x - player_pos.0;
        let dy = bag.world_y - player_pos.1;
        let dist_sq = (dx * dx) + (dy * dy);
        let max_dist_sq =
            Self::MAX_INTERACTION_DISTANCE_METERS * Self::MAX_INTERACTION_DISTANCE_METERS;

        if dist_sq > max_dist_sq {
            return LootResult::OutOfRange;
        }

        let Some(item_idx) = bag.items.iter().position(|i| i.id == request.item_id) else {
            return LootResult::ItemNotFound;
        };

        if bag.items[item_idx].revision != request.expected_revision {
            return LootResult::ConcurrencyConflict;
        }

        let def_id = bag.items[item_idx].definition_id;
        let Some(dimensions) = get_dimensions(def_id) else {
            return LootResult::ItemNotFound;
        };

        if player_grid
            .can_place_at(request.target_slot, dimensions, request.rotation, None)
            .is_err()
        {
            return LootResult::InventoryFull;
        }

        let mut item = bag.items.swap_remove(item_idx);
        let _ = player_grid.place_item(item.id, request.target_slot, dimensions, request.rotation);
        item.transfer_ownership(Ownership::Character(player_id));

        LootResult::Success { item }
    }

    /// Manipulação interna: rotaciona e/ou move uma peça já existente na grade.
    pub fn handle_move_or_rotate_item(
        player_grid: &mut InventoryGrid,
        item: &mut ItemInstance,
        request: RequestMoveOrRotateItem,
    ) -> InventoryActionResult {
        if item.id != request.item_id {
            return InventoryActionResult::ItemNotFound;
        }

        if item.revision != request.expected_revision {
            return InventoryActionResult::ConcurrencyConflict;
        }

        if player_grid
            .rotate_or_move(item.id, request.target_slot, request.rotation)
            .is_err()
        {
            return InventoryActionResult::InvalidPlacement;
        }

        item.revision = item.revision.saturating_add(1);
        InventoryActionResult::Success
    }

    /// Merge autoritativo entre dois lotes empilháveis.
    /// Retorna `true` se a pilha de origem foi totalmente consumida e deve ser destruída da grade.
    pub fn handle_merge_stack(
        player_grid: &mut InventoryGrid,
        source: &mut ItemInstance,
        target: &mut ItemInstance,
        max_stack: u32,
        request: RequestMergeStack,
    ) -> Result<bool, InventoryActionResult> {
        if source.id != request.source_item_id || target.id != request.target_item_id {
            return Err(InventoryActionResult::ItemNotFound);
        }

        if source.revision != request.expected_source_revision
            || target.revision != request.expected_target_revision
        {
            return Err(InventoryActionResult::ConcurrencyConflict);
        }

        if source.definition_id != target.definition_id {
            return Err(InventoryActionResult::IncompatibleItems);
        }

        if target.quantity >= max_stack {
            return Err(InventoryActionResult::StackLimitExceeded);
        }

        let available_space = max_stack - target.quantity;
        let transfer_amount = source.quantity.min(available_space);

        source.quantity -= transfer_amount;
        target.quantity += transfer_amount;

        source.revision = source.revision.saturating_add(1);
        target.revision = target.revision.saturating_add(1);

        if source.quantity == 0 {
            player_grid.remove_item(source.id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Divide uma pilha existente, gerando uma nova `ItemInstance` com novo ID
    /// e posicionando-a na célula solicitada se houver espaço livre.
    pub fn handle_split_stack<F>(
        player_grid: &mut InventoryGrid,
        source: &mut ItemInstance,
        request: RequestSplitStack,
        new_item_id: ItemInstanceId,
        get_dimensions: F,
    ) -> Result<ItemInstance, InventoryActionResult>
    where
        F: FnOnce(aurenfall_core::DefinitionId) -> Option<InventoryDimensions>,
    {
        if source.id != request.source_item_id {
            return Err(InventoryActionResult::ItemNotFound);
        }

        if source.revision != request.expected_source_revision {
            return Err(InventoryActionResult::ConcurrencyConflict);
        }

        if request.split_quantity == 0 || request.split_quantity >= source.quantity {
            return Err(InventoryActionResult::InvalidSplitQuantity);
        }

        let Some(dimensions) = get_dimensions(source.definition_id) else {
            return Err(InventoryActionResult::ItemNotFound);
        };

        if player_grid
            .can_place_at(request.target_slot, dimensions, request.rotation, None)
            .is_err()
        {
            return Err(InventoryActionResult::InvalidPlacement);
        }

        let mut new_stack = ItemInstance::new_stackable(
            new_item_id,
            source.definition_id,
            source.owner.clone(),
            request.split_quantity,
        );

        let _ = player_grid.place_item(
            new_stack.id,
            request.target_slot,
            dimensions,
            request.rotation,
        );

        source.quantity -= request.split_quantity;
        source.revision = source.revision.saturating_add(1);
        new_stack.revision = 1;

        Ok(new_stack)
    }

    /// Executa o descarte de um item: remove da grade do jogador, transfere a posse
    /// para o mundo e spawna uma bolsa próxima às coordenadas informadas.
    pub fn handle_drop_item(
        manager: &mut DroppedBagManager,
        player_grid: &mut InventoryGrid,
        item: &mut ItemInstance,
        request: RequestDropItem,
        player_id: CharacterId,
        player_pos: (f32, f32),
        current_tick: u64,
        decay_ticks: u64,
        next_bag_id: DroppedBagId,
    ) -> DropResult {
        if item.id != request.item_id {
            return DropResult::ItemNotFound;
        }

        if item.revision != request.expected_revision {
            return DropResult::ConcurrencyConflict;
        }

        // 1. Remove da matriz de células do jogador
        player_grid.remove_item(item.id);

        // 2. Transfere a custódia para o setor correspondente do mundo (i64)
        let sector_x = (player_pos.0 / 64.0).floor() as i64;
        let sector_y = (player_pos.1 / 64.0).floor() as i64;
        item.transfer_ownership(Ownership::World {
            sector: aurenfall_core::SectorCoord {
                x: sector_x,
                y: sector_y,
            },
        });

        // 3. Cria a nova entidade de bolsa no chão contendo o item
        let new_bag = DroppedBagEntity::new(
            next_bag_id,
            player_id,
            (sector_x as i32, sector_y as i32),
            player_pos,
            current_tick,
            decay_ticks,
            vec![item.clone()],
        );

        manager.spawn_bag(new_bag);

        DropResult::Success {
            bag_id: next_bag_id,
            dropped_item: item.clone(),
        }
    }

    /// Transfere uma peça da grade de inventário para o slot do Paperdoll correspondente.
    pub fn handle_equip_item<F>(
        player_grid: &mut InventoryGrid,
        equipment: &mut CharacterEquipment,
        item: &mut ItemInstance,
        request: RequestEquipItem,
        get_allowed_slot: F,
    ) -> InventoryActionResult
    where
        F: FnOnce(aurenfall_core::DefinitionId) -> Option<EquipmentSlotKind>,
    {
        if item.id != request.item_id {
            return InventoryActionResult::ItemNotFound;
        }

        if item.revision != request.expected_revision {
            return InventoryActionResult::ConcurrencyConflict;
        }

        let Some(allowed_slot) = get_allowed_slot(item.definition_id) else {
            return InventoryActionResult::IncompatibleSlot;
        };

        if allowed_slot != request.target_slot {
            return InventoryActionResult::IncompatibleSlot;
        }

        if equipment.get_equipped(request.target_slot).is_some() {
            return InventoryActionResult::SlotAlreadyOccupied;
        }

        // Remove da grade e insere no slot
        player_grid.remove_item(item.id);
        item.revision = item.revision.saturating_add(1);
        equipment.equip_item(request.target_slot, item.clone());

        InventoryActionResult::Success
    }

    /// Remove uma peça do Paperdoll e a devolve para uma célula livre da grade.
    pub fn handle_unequip_item<F>(
        player_grid: &mut InventoryGrid,
        equipment: &mut CharacterEquipment,
        request: RequestUnequipItem,
        get_dimensions: F,
    ) -> Result<ItemInstance, InventoryActionResult>
    where
        F: FnOnce(aurenfall_core::DefinitionId) -> Option<InventoryDimensions>,
    {
        let Some(item) = equipment.get_equipped(request.source_slot) else {
            return Err(InventoryActionResult::ItemNotFound);
        };

        if item.revision != request.expected_revision {
            return Err(InventoryActionResult::ConcurrencyConflict);
        }

        let Some(dimensions) = get_dimensions(item.definition_id) else {
            return Err(InventoryActionResult::ItemNotFound);
        };

        // Valida se há espaço na grade
        if player_grid
            .can_place_at(request.target_slot, dimensions, request.rotation, None)
            .is_err()
        {
            return Err(InventoryActionResult::InvalidPlacement);
        }

        let mut unequipped_item = equipment.unequip_item(request.source_slot).unwrap();
        let _ = player_grid.place_item(
            unequipped_item.id,
            request.target_slot,
            dimensions,
            request.rotation,
        );

        unequipped_item.revision = unequipped_item.revision.saturating_add(1);
        Ok(unequipped_item)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_core::{CharacterId, DefinitionId, ItemInstanceId, SectorCoord};
    use aurenfall_domain::items::{
        DroppedBagEntity, DroppedBagId, GridSlotCoord, InventoryDimensions, ItemRotation,
    };

    #[test]
    fn test_open_bag_request_range_and_payload() {
        let mut manager = DroppedBagManager::new();
        let bag_id = DroppedBagId(1);
        let bag = DroppedBagEntity::new(
            bag_id,
            CharacterId(10),
            (0, 0),
            (10.0, 10.0),
            100,
            1000,
            Vec::new(),
        );
        manager.spawn_bag(bag);

        let req = RequestOpenBag { bag_id };

        // Fora de alcance (> 3.5m)
        let res_far = LootService::handle_open_bag_request(&manager, req.clone(), (15.0, 10.0));
        assert_eq!(res_far, OpenBagResult::OutOfRange);

        // Dentro de alcance (1m de distância)
        let res_near = LootService::handle_open_bag_request(&manager, req, (11.0, 10.0));
        match res_near {
            OpenBagResult::Success(snapshot) => assert_eq!(snapshot.bag_id, bag_id),
            _ => panic!("Deveria ter aberto com sucesso"),
        }
    }

    #[test]
    fn test_loot_service_scenarios() {
        let mut manager = DroppedBagManager::new();
        let mut grid = InventoryGrid::new(4, 4);
        let bag_id = DroppedBagId(1);
        let player_a = CharacterId(10);
        let player_b = CharacterId(20);
        let def_id = DefinitionId(1001);

        let item = ItemInstance::new_unique(
            ItemInstanceId(42),
            def_id,
            Ownership::World {
                sector: SectorCoord { x: 0, y: 0 },
            },
            999,
            120,
        );

        let bag = DroppedBagEntity::new(
            bag_id,
            CharacterId(99),
            (0, 0),
            (10.0, 10.0),
            100,
            1000,
            vec![item],
        );
        manager.spawn_bag(bag);

        let dimensions_lookup = |_id| Some(InventoryDimensions { width: 1, height: 2 });

        // 1. Fora de alcance (> 3.5m)
        let req_far = RequestLootItem {
            bag_id,
            item_id: ItemInstanceId(42),
            target_slot: GridSlotCoord { x: 0, y: 0 },
            rotation: ItemRotation::Normal,
            expected_revision: 1,
        };
        let res_far = LootService::handle_loot_request(
            &mut manager,
            &mut grid,
            req_far,
            player_a,
            (15.0, 10.0),
            dimensions_lookup,
        );
        assert_eq!(res_far, LootResult::OutOfRange);

        // 2. Inventário cheio / Colisão na posição solicitada
        let req_invalid_slot = RequestLootItem {
            bag_id,
            item_id: ItemInstanceId(42),
            target_slot: GridSlotCoord { x: 3, y: 3 },
            rotation: ItemRotation::Normal,
            expected_revision: 1,
        };
        let res_full = LootService::handle_loot_request(
            &mut manager,
            &mut grid,
            req_invalid_slot,
            player_a,
            (11.0, 10.0),
            dimensions_lookup,
        );
        assert_eq!(res_full, LootResult::InventoryFull);

        // 3. Saque com sucesso
        let req_ok = RequestLootItem {
            bag_id,
            item_id: ItemInstanceId(42),
            target_slot: GridSlotCoord { x: 0, y: 0 },
            rotation: ItemRotation::Normal,
            expected_revision: 1,
        };
        let res_ok = LootService::handle_loot_request(
            &mut manager,
            &mut grid,
            req_ok,
            player_a,
            (11.0, 10.0),
            dimensions_lookup,
        );
        match res_ok {
            LootResult::Success { item } => {
                assert_eq!(item.owner, Ownership::Character(player_a));
                assert_eq!(item.revision, 2);
            }
            _ => panic!("Deveria ter sacado com sucesso"),
        }

        // 4. Tentativa concorrente / Já sacado
        let req_late = RequestLootItem {
            bag_id,
            item_id: ItemInstanceId(42),
            target_slot: GridSlotCoord { x: 1, y: 0 },
            rotation: ItemRotation::Normal,
            expected_revision: 1,
        };
        let res_late = LootService::handle_loot_request(
            &mut manager,
            &mut grid,
            req_late,
            player_b,
            (10.5, 10.0),
            dimensions_lookup,
        );
        assert_eq!(res_late, LootResult::ItemNotFound);
    }

    #[test]
    fn test_rotation_placement_and_scum_style_orientations() {
        let mut grid = InventoryGrid::new(4, 4);
        let rifle_id = ItemInstanceId(101);
        let rifle_dim = InventoryDimensions { width: 1, height: 3 };

        assert!(grid
            .place_item(rifle_id, GridSlotCoord { x: 0, y: 0 }, rifle_dim, ItemRotation::Normal)
            .is_ok());

        assert!(grid
            .rotate_or_move(rifle_id, GridSlotCoord { x: 0, y: 0 }, ItemRotation::Rotated90)
            .is_ok());

        assert_eq!(grid.get_item_at(GridSlotCoord { x: 2, y: 0 }), Some(rifle_id));
        assert_eq!(grid.get_item_at(GridSlotCoord { x: 0, y: 2 }), None);
    }

    #[test]
    fn test_merge_stacks_full_and_partial() {
        let mut grid = InventoryGrid::new(4, 4);
        let def_ammo = DefinitionId(300);

        let mut stack_a = ItemInstance::new_stackable(
            ItemInstanceId(1),
            def_ammo,
            Ownership::Character(CharacterId(1)),
            20,
        );
        let mut stack_b = ItemInstance::new_stackable(
            ItemInstanceId(2),
            def_ammo,
            Ownership::Character(CharacterId(1)),
            15,
        );

        grid.place_item(
            stack_a.id,
            GridSlotCoord { x: 0, y: 0 },
            InventoryDimensions { width: 1, height: 1 },
            ItemRotation::Normal,
        )
        .unwrap();
        grid.place_item(
            stack_b.id,
            GridSlotCoord { x: 1, y: 0 },
            InventoryDimensions { width: 1, height: 1 },
            ItemRotation::Normal,
        )
        .unwrap();

        let req = RequestMergeStack {
            source_item_id: stack_b.id,
            target_item_id: stack_a.id,
            expected_source_revision: 1,
            expected_target_revision: 1,
        };

        let consumed =
            LootService::handle_merge_stack(&mut grid, &mut stack_b, &mut stack_a, 30, req)
                .unwrap();
        assert!(!consumed);
        assert_eq!(stack_a.quantity, 30);
        assert_eq!(stack_b.quantity, 5);
        assert_eq!(grid.get_item_at(GridSlotCoord { x: 1, y: 0 }), Some(stack_b.id));

        let req2 = RequestMergeStack {
            source_item_id: stack_b.id,
            target_item_id: stack_a.id,
            expected_source_revision: 2,
            expected_target_revision: 2,
        };
        let err = LootService::handle_merge_stack(&mut grid, &mut stack_b, &mut stack_a, 30, req2);
        assert_eq!(err, Err(InventoryActionResult::StackLimitExceeded));
    }

    #[test]
    fn test_split_stack_success_and_failures() {
        let mut grid = InventoryGrid::new(4, 4);
        let def_ammo = DefinitionId(300);
        let player = CharacterId(1);

        let mut stack = ItemInstance::new_stackable(
            ItemInstanceId(1),
            def_ammo,
            Ownership::Character(player),
            50,
        );
        grid.place_item(
            stack.id,
            GridSlotCoord { x: 0, y: 0 },
            InventoryDimensions { width: 1, height: 1 },
            ItemRotation::Normal,
        )
        .unwrap();

        let dim_lookup = |_id| Some(InventoryDimensions { width: 1, height: 1 });

        // 1. Falha: Quantidade inválida
        let req_invalid = RequestSplitStack {
            source_item_id: stack.id,
            split_quantity: 50,
            target_slot: GridSlotCoord { x: 1, y: 0 },
            rotation: ItemRotation::Normal,
            expected_source_revision: 1,
        };
        let err = LootService::handle_split_stack(
            &mut grid,
            &mut stack,
            req_invalid,
            ItemInstanceId(2),
            dim_lookup,
        );
        assert_eq!(err, Err(InventoryActionResult::InvalidSplitQuantity));

        // 2. Falha: Destino ocupado
        let req_collision = RequestSplitStack {
            source_item_id: stack.id,
            split_quantity: 20,
            target_slot: GridSlotCoord { x: 0, y: 0 },
            rotation: ItemRotation::Normal,
            expected_source_revision: 1,
        };
        let err_col = LootService::handle_split_stack(
            &mut grid,
            &mut stack,
            req_collision,
            ItemInstanceId(2),
            dim_lookup,
        );
        assert_eq!(err_col, Err(InventoryActionResult::InvalidPlacement));

        // 3. Sucesso: Divide 20 unidades para (1, 0)
        let req_ok = RequestSplitStack {
            source_item_id: stack.id,
            split_quantity: 20,
            target_slot: GridSlotCoord { x: 1, y: 0 },
            rotation: ItemRotation::Normal,
            expected_source_revision: 1,
        };
        let new_stack = LootService::handle_split_stack(
            &mut grid,
            &mut stack,
            req_ok,
            ItemInstanceId(2),
            dim_lookup,
        )
        .unwrap();

        assert_eq!(stack.quantity, 30);
        assert_eq!(stack.revision, 2);
        assert_eq!(new_stack.id, ItemInstanceId(2));
        assert_eq!(new_stack.quantity, 20);
        assert_eq!(
            grid.get_item_at(GridSlotCoord { x: 1, y: 0 }),
            Some(ItemInstanceId(2))
        );
    }

    #[test]
    fn test_drop_item_success_and_lifecycle() {
        let mut manager = DroppedBagManager::new();
        let mut grid = InventoryGrid::new(4, 4);
        let player = CharacterId(10);
        let item_id = ItemInstanceId(99);

        let mut item = ItemInstance::new_unique(
            item_id,
            DefinitionId(1),
            Ownership::Character(player),
            1234,
            100,
        );

        grid.place_item(
            item.id,
            GridSlotCoord { x: 0, y: 0 },
            InventoryDimensions { width: 1, height: 1 },
            ItemRotation::Normal,
        )
        .unwrap();

        let req = RequestDropItem {
            item_id,
            expected_revision: 1,
        };

        // Executa o drop no tick 500, com 300 ticks de decay
        let result = LootService::handle_drop_item(
            &mut manager,
            &mut grid,
            &mut item,
            req,
            player,
            (12.0, 15.0),
            500,
            300,
            DroppedBagId(50),
        );

        match result {
            DropResult::Success { bag_id, dropped_item } => {
                assert_eq!(bag_id, DroppedBagId(50));
                assert_eq!(dropped_item.revision, 2);
                assert!(matches!(dropped_item.owner, Ownership::World { .. }));
            }
            _ => panic!("O drop deveria ter sido realizado com sucesso"),
        }

        // Verifica se a célula do inventário foi liberada
        assert_eq!(grid.get_item_at(GridSlotCoord { x: 0, y: 0 }), None);

        // Verifica se a bolsa existe no mundo com os itens corretos
        let spawned_bag = manager.get_bag(DroppedBagId(50)).expect("Bolsa não encontrada");
        assert_eq!(spawned_bag.items.len(), 1);
        assert_eq!(spawned_bag.items[0].id, item_id);
    }

    #[test]
    fn test_equip_and_unequip_lifecycle() {
        let mut grid = InventoryGrid::new(4, 4);
        let mut equipment = CharacterEquipment::new();
        let weapon_id = ItemInstanceId(77);
        let weapon_def = DefinitionId(500);

        let mut weapon = ItemInstance::new_unique(
            weapon_id,
            weapon_def,
            Ownership::Character(CharacterId(1)),
            100,
            100,
        );

        let weapon_dim = InventoryDimensions { width: 1, height: 2 };
        grid.place_item(weapon.id, GridSlotCoord { x: 0, y: 0 }, weapon_dim, ItemRotation::Normal).unwrap();

        let slot_lookup = |_id| Some(EquipmentSlotKind::MainHand);
        let dim_lookup = |_id| Some(weapon_dim);

        // 1. Falha: Tentar equipar no slot errado (ex: Chest)
        let req_wrong_slot = RequestEquipItem {
            item_id: weapon_id,
            target_slot: EquipmentSlotKind::Chest,
            expected_revision: 1,
        };
        let err_slot = LootService::handle_equip_item(&mut grid, &mut equipment, &mut weapon, req_wrong_slot, slot_lookup);
        assert_eq!(err_slot, InventoryActionResult::IncompatibleSlot);

        // 2. Sucesso: Equipar no MainHand
        let req_equip_ok = RequestEquipItem {
            item_id: weapon_id,
            target_slot: EquipmentSlotKind::MainHand,
            expected_revision: 1,
        };
        let res_ok = LootService::handle_equip_item(&mut grid, &mut equipment, &mut weapon, req_equip_ok, slot_lookup);
        assert_eq!(res_ok, InventoryActionResult::Success);

        // Grade agora está vazia na célula (0, 0)
        assert_eq!(grid.get_item_at(GridSlotCoord { x: 0, y: 0 }), None);
        assert!(equipment.get_equipped(EquipmentSlotKind::MainHand).is_some());

        // 3. Sucesso: Desequipar de volta para a grade em (1, 1)
        let req_unequip = RequestUnequipItem {
            source_slot: EquipmentSlotKind::MainHand,
            target_slot: GridSlotCoord { x: 1, y: 1 },
            rotation: ItemRotation::Normal,
            expected_revision: 2,
        };
        let unequipped = LootService::handle_unequip_item(&mut grid, &mut equipment, req_unequip, dim_lookup).unwrap();

        assert_eq!(unequipped.id, weapon_id);
        assert_eq!(unequipped.revision, 3);
        assert!(equipment.get_equipped(EquipmentSlotKind::MainHand).is_none());
        assert_eq!(grid.get_item_at(GridSlotCoord { x: 1, y: 1 }), Some(weapon_id));
    }

}