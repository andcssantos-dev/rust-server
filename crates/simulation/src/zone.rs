use std::{
    collections::{HashMap, hash_map::Entry},
    fmt,
    sync::Arc,
    time::Duration,
};

use aurenfall_contracts::inventory::{
    DropResult, InventoryActionResult, LootResult, RequestDropItem, RequestEquipItem,
    RequestLootItem, RequestMergeStack, RequestMoveOrRotateItem, RequestSplitStack,
    RequestUnequipItem,
};

use aurenfall_domain::items::{
    CharacterInventoryState, DroppedBagId, DroppedBagManager, InventoryDimensions,
};

pub type CharacterInventoryComponent = CharacterInventoryState;

use anyhow::{Context, bail};
use aurenfall_core::{CharacterId, ItemInstanceId, IntentSequence, ServerTick, WorldPositionMm, ZoneId};
use hecs::Entity;
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    time::MissedTickBehavior,
};
use tracing::info;


use crate::{
    CharacterMovementSettings, MovementInput, MovementRules, MovementSnapshot, TraversalWorld,
    components::{Dirty, Identity, MoveIntent, Player, Position},
    systems::{RockDestroyedEvent, simulation_tick},
};

#[derive(Debug, Clone, PartialEq)]
pub enum ZoneCommand {
    SpawnCharacter {
        character_id: CharacterId,
        position: WorldPositionMm,
        inventory: CharacterInventoryState,
        inventory_revision: u64,
    },
    DespawnCharacter {
        character_id: CharacterId,
        responder: Option<InventoryDespawnResponder>,
    },
    SnapshotInventory {
        character_id: CharacterId,
        responder: InventoryDespawnResponder,
    },
    MoveIntent {
        character_id: CharacterId,
        sequence: u64,
        input: MovementInput,
    },
    SpawnRock {
        position: WorldPositionMm,
        health: u32,
    },
    MineIntent {
        character_id: CharacterId,
        power: u32,
    },
    InventoryIntent {
        character_id: CharacterId,
        intent: PlayerInventoryIntent,
    },
}

pub struct ZoneRuntime {
    id: ZoneId,
    tick_rate_hz: u32,
    commands: mpsc::Receiver<ZoneCommand>,
    movement_snapshots: Option<mpsc::Sender<MovementSnapshot>>,
    gameplay_events: Option<mpsc::Sender<RockDestroyedEvent>>,
    tick: ServerTick,
    movement_rules: MovementRules,
    traversal_world: TraversalWorld,
    world: hecs::World,
    entity_map: HashMap<CharacterId, Entity>,
    last_processed_inputs: HashMap<CharacterId, IntentSequence>,
    pub dropped_bags: DroppedBagManager,
    pub pending_inventory_intents: Vec<(CharacterId, PlayerInventoryIntent)>,
    pub next_bag_id: u64,
    pub inventory_revisions: HashMap<CharacterId, u64>,
}

impl fmt::Debug for ZoneRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZoneRuntime")
            .field("id", &self.id)
            .field("tick_rate_hz", &self.tick_rate_hz)
            .field("tick", &self.tick)
            .field("movement_rules", &self.movement_rules)
            .field("traversal_world", &self.traversal_world)
            .field("entity_count", &self.world.len())
            .field("entity_map", &self.entity_map)
            .field("last_processed_inputs", &self.last_processed_inputs)
            .finish()
    }
}

impl ZoneRuntime {
    pub fn new(
        id: ZoneId,
        tick_rate_hz: u32,
        movement_settings: CharacterMovementSettings,
        traversal_world: TraversalWorld,
        commands: mpsc::Receiver<ZoneCommand>,
    ) -> anyhow::Result<Self> {
        if tick_rate_hz == 0 {
            bail!("zone tick rate must be > 0");
        }
        let movement_rules = MovementRules::new(movement_settings, tick_rate_hz)?;
        Ok(Self {
            id,
            tick_rate_hz,
            commands,
            movement_snapshots: None,
            gameplay_events: None,
            tick: ServerTick(0),
            movement_rules,
            traversal_world,
            world: hecs::World::new(),
            entity_map: HashMap::new(),
            last_processed_inputs: HashMap::new(),
            dropped_bags: DroppedBagManager::new(),
            pending_inventory_intents: Vec::new(),
            next_bag_id: 0,
            inventory_revisions: HashMap::new(),
        })
    }

    #[must_use]
    pub fn with_movement_snapshots(mut self, snapshots: mpsc::Sender<MovementSnapshot>) -> Self {
        self.movement_snapshots = Some(snapshots);
        self
    }

    #[must_use]
    pub fn with_gameplay_events(mut self, events: mpsc::Sender<RockDestroyedEvent>) -> Self {
        self.gameplay_events = Some(events);
        self
    }

    #[must_use]
    pub fn get_character_inventory(
        &self,
        character_id: CharacterId,
    ) -> Option<(CharacterInventoryState, u64)> {
        let &entity = self.entity_map.get(&character_id)?;
        let inventory = self.world.get::<&CharacterInventoryComponent>(entity).ok()?;
        let revision = self.inventory_revisions.get(&character_id).copied().unwrap_or(1);
        Some(((*inventory).clone(), revision))
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut ticker = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(self.tick_rate_hz)));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        info!(
            zone_id = self.id.0,
            tick_rate_hz = self.tick_rate_hz,
            "zone single-writer loop started"
        );

        loop {
            tokio::select! {
                _ = ticker.tick() => self.step()?,
                command = self.commands.recv() => {
                    match command {
                        Some(command) => self.accept(command)?,
                        None => break,
                    }
                }
            }
        }

        info!(
            zone_id = self.id.0,
            tick = self.tick.0,
            "zone loop stopped"
        );
        Ok(())
    }

    fn accept(&mut self, command: ZoneCommand) -> anyhow::Result<()> {
        match command {
            ZoneCommand::SpawnCharacter {
                character_id,
                position,
                inventory,
                inventory_revision,
            } => {
                self.traversal_world
                    .validate_placement(position, self.movement_rules.character_radius_mm())?;

                match self.entity_map.entry(character_id) {
                    Entry::Vacant(entry) => {
                        let entity = self.world.spawn((
                            Identity(character_id),
                            Position(position),
                            crate::components::Remainder::default(),
                            Player,
                            Dirty,
                            inventory,
                        ));
                        entry.insert(entity);
                        self.inventory_revisions.insert(character_id, inventory_revision);
                    }
                    Entry::Occupied(_) => {
                        bail!("duplicate spawn for character {}", character_id.0);
                    }
                }
            }
            ZoneCommand::DespawnCharacter { character_id, responder } => {
                let entity = self.entity_map.remove(&character_id).context("unknown character")?;
                self.last_processed_inputs.remove(&character_id);
                let revision = self.inventory_revisions.remove(&character_id).unwrap_or(0);

                let inventory_state = if let Ok(inv) = self.world.get::<&CharacterInventoryState>(entity) {
                    (*inv).clone()
                } else {
                    CharacterInventoryState::new(10, 6)
                };

                self.world.despawn(entity)?;

                if let Some(responder) = responder {
                    responder.send((inventory_state, revision));
                }
            }
            ZoneCommand::SnapshotInventory { character_id, responder } => {
                if let Some(&entity) = self.entity_map.get(&character_id) {
                    let revision = self.inventory_revisions.get(&character_id).copied().unwrap_or(0);
                    let inventory_state = if let Ok(inv) = self.world.get::<&CharacterInventoryState>(entity) {
                        (*inv).clone()
                    } else {
                        CharacterInventoryState::new(10, 6)
                    };
                    responder.send((inventory_state, revision));
                }
            }
            ZoneCommand::MoveIntent {
                character_id,
                sequence,
                input,
            } => {
                let &entity = self.entity_map.get(&character_id).context("unknown character")?;
                self.world.insert_one(entity, MoveIntent(input))?;
                self.last_processed_inputs.insert(character_id, aurenfall_core::IntentSequence(sequence));
            }
            ZoneCommand::SpawnRock { position, health } => {
                self.world.spawn((
                    crate::components::Position(position),
                    crate::components::RockNode::new(health),
                ));
            }
            ZoneCommand::MineIntent { character_id, power } => {
                let &entity = self.entity_map.get(&character_id).context("unknown character")?;
                self.world.insert_one(entity, crate::components::MineIntent {
                    miner: character_id,
                    power,
                })?;
            }
            ZoneCommand::InventoryIntent {
                character_id,
                intent,
            } => {
                self.pending_inventory_intents.push((character_id, intent));
            }
        }
        Ok(())
    }

    fn step(&mut self) -> anyhow::Result<()> {
        self.tick.0 = self.tick.0.checked_add(1).context("tick overflow")?;

        let (spatial_updates, destroyed_rocks) = simulation_tick(
            &mut self.world,
            &self.traversal_world,
            &self.movement_rules,
        )?;

        let _inv_events = self.process_inventory_intents(|_| None, |_| None);

        if let Some(events_tx) = self.gameplay_events.as_ref() {
            for event in destroyed_rocks {
                let _ = events_tx.try_send(event);
            }
        }

        if let Some(snapshot_tx) = self.movement_snapshots.as_ref() {
            for update in spatial_updates {
                let character_id = update.identity.0;
                let last_seq = self.last_processed_inputs.get(&character_id).copied();

                let snapshot = MovementSnapshot::new(
                    character_id,
                    self.tick,
                    update.position.0,
                    last_seq,
                    last_seq,
                );

                let _ = snapshot_tx.try_send(snapshot);
            }
        }

        Ok(())
    }

    /// Processa a fila de intenções de inventário manipulando os componentes do ECS.
    pub fn process_inventory_intents<FD, FS>(
        &mut self,
        get_dimensions: FD,
        get_allowed_slot: FS,
    ) -> Vec<InventoryEvent>
    where
        FD: Fn(aurenfall_core::DefinitionId) -> Option<InventoryDimensions> + Copy,
        FS: Fn(aurenfall_core::DefinitionId) -> Option<aurenfall_domain::items::EquipmentSlotKind> + Copy,
    {
        let _expired_bags = self.dropped_bags.tick_decay(self.tick.0);

        let mut events = Vec::new();
        let intents = std::mem::take(&mut self.pending_inventory_intents);

        for (player_id, intent) in intents {
            let Some(&entity) = self.entity_map.get(&player_id) else {
                continue;
            };

            let player_pos = {
                let Ok(pos) = self.world.get::<&Position>(entity) else {
                    continue;
                };
                (pos.0.x() as f32 / 1000.0, pos.0.y() as f32 / 1000.0)
            };

            let Ok(mut inv_guard) = self.world.get::<&mut CharacterInventoryComponent>(entity) else {
                continue;
            };

            // Desestrutura os campos para o compilador permitir empréstimos simultâneos distintos
            let CharacterInventoryComponent {
                grid,
                equipment,
                items,
            } = &mut *inv_guard;

            match intent {
                PlayerInventoryIntent::Loot(req) => {
                    let result = crate::loot::LootService::handle_loot_request(
                        &mut self.dropped_bags,
                        grid,
                        req,
                        player_id,
                        player_pos,
                        get_dimensions,
                    );

                    if let LootResult::Success { ref item } = result {
                        items.insert(item.id, item.clone());
                    }

                    events.push(InventoryEvent::LootProcessed { player_id, result });
                }

                PlayerInventoryIntent::MoveOrRotate(req) => {
                    let result = if let Some(item) = items.get_mut(&req.item_id) {
                        crate::loot::LootService::handle_move_or_rotate_item(grid, item, req)
                    } else {
                        InventoryActionResult::ItemNotFound
                    };

                    events.push(InventoryEvent::ActionProcessed { player_id, result });
                }

                PlayerInventoryIntent::Merge(req) => {
                    let result = match (items.remove(&req.source_item_id), items.get_mut(&req.target_item_id)) {
                        (Some(mut source), Some(target)) => {
                            let max_stack = 100;
                            let merge_res = crate::loot::LootService::handle_merge_stack(
                                grid,
                                &mut source,
                                target,
                                max_stack,
                                req,
                            );

                            match merge_res {
                                Ok(true) => InventoryActionResult::Success,
                                Ok(false) => {
                                    items.insert(source.id, source);
                                    InventoryActionResult::Success
                                }
                                Err(err) => {
                                    items.insert(source.id, source);
                                    err
                                }
                            }
                        }
                        (Some(source), None) => {
                            items.insert(source.id, source);
                            InventoryActionResult::ItemNotFound
                        }
                        _ => InventoryActionResult::ItemNotFound,
                    };

                    events.push(InventoryEvent::ActionProcessed { player_id, result });
                }

                PlayerInventoryIntent::Split { request, new_item_id } => {
                    let result = if let Some(source) = items.get_mut(&request.source_item_id) {
                        match crate::loot::LootService::handle_split_stack(
                            grid,
                            source,
                            request,
                            new_item_id,
                            get_dimensions,
                        ) {
                            Ok(new_item) => {
                                items.insert(new_item.id, new_item);
                                InventoryActionResult::Success
                            }
                            Err(err) => err,
                        }
                    } else {
                        InventoryActionResult::ItemNotFound
                    };

                    events.push(InventoryEvent::ActionProcessed { player_id, result });
                }

                PlayerInventoryIntent::Drop(req) => {
                    let result = if let Some(mut item) = items.remove(&req.item_id) {
                        self.next_bag_id = self.next_bag_id.saturating_add(1);
                        let bag_id = DroppedBagId(self.next_bag_id);

                        let drop_res = crate::loot::LootService::handle_drop_item(
                            &mut self.dropped_bags,
                            grid,
                            &mut item,
                            req,
                            player_id,
                            player_pos,
                            self.tick.0,
                            600,
                            bag_id,
                        );

                        if !matches!(drop_res, DropResult::Success { .. }) {
                            items.insert(item.id, item);
                        }

                        drop_res
                    } else {
                        DropResult::ItemNotFound
                    };

                    events.push(InventoryEvent::DropProcessed { player_id, result });
                }

                PlayerInventoryIntent::Equip(req) => {
                    let result = if let Some(item) = items.get_mut(&req.item_id) {
                        crate::loot::LootService::handle_equip_item(
                            grid,
                            equipment,
                            item,
                            req,
                            get_allowed_slot,
                        )
                    } else {
                        InventoryActionResult::ItemNotFound
                    };

                    events.push(InventoryEvent::ActionProcessed { player_id, result });
                }

                PlayerInventoryIntent::Unequip(req) => {
                    let result = match crate::loot::LootService::handle_unequip_item(
                        grid,
                        equipment,
                        req,
                        get_dimensions,
                    ) {
                        Ok(item) => {
                            items.insert(item.id, item);
                            InventoryActionResult::Success
                        }
                        Err(err) => err,
                    };

                    events.push(InventoryEvent::ActionProcessed { player_id, result });
                }
            }
        }

        events
    }


}

/// Intenções de inventário enviadas pelos clientes da zona.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerInventoryIntent {
    Loot(RequestLootItem),
    MoveOrRotate(RequestMoveOrRotateItem),
    Merge(RequestMergeStack),
    Split {
        request: RequestSplitStack,
        new_item_id: ItemInstanceId,
    },
    Drop(RequestDropItem),
    Equip(RequestEquipItem),
    Unequip(RequestUnequipItem),
}

#[derive(Debug, Clone)]
pub enum InventoryEvent {
    LootProcessed {
        player_id: CharacterId,
        result: LootResult,
    },
    ActionProcessed {
        player_id: CharacterId,
        result: InventoryActionResult,
    },
    DropProcessed {
        player_id: CharacterId,
        result: DropResult,
    },
}

#[derive(Debug, Clone)]
pub struct InventoryDespawnResponder(
    Arc<Mutex<Option<oneshot::Sender<(CharacterInventoryState, u64)>>>>,
);

impl InventoryDespawnResponder {
    pub fn new(sender: oneshot::Sender<(CharacterInventoryState, u64)>) -> Self {
        Self(Arc::new(Mutex::new(Some(sender))))
    }

    pub fn send(&self, payload: (CharacterInventoryState, u64)) {
        if let Ok(mut lock) = self.0.try_lock() {
            if let Some(tx) = lock.take() {
                let _ = tx.send(payload);
            }
        }
    }
}

impl PartialEq for InventoryDespawnResponder {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StaticTraversalBlocker;

    fn movement_settings(
        speed_mm_per_second: u32,
        timeout_ticks: u64,
    ) -> anyhow::Result<CharacterMovementSettings> {
        CharacterMovementSettings::new(speed_mm_per_second, timeout_ticks, 250)
    }

    fn empty_zone(speed_mm_per_second: u32, timeout_ticks: u64) -> anyhow::Result<ZoneRuntime> {
        let (_tx, rx) = mpsc::channel(8);
        ZoneRuntime::new(
            ZoneId(3),
            20,
            movement_settings(speed_mm_per_second, timeout_ticks)?,
            TraversalWorld::default(),
            rx,
        )
    }

    #[test]
    fn zone_owns_spawn_move_tick_and_despawn_lifecycle() -> anyhow::Result<()> {
        let mut zone = empty_zone(4_000, 5)?;
        let character_id = CharacterId(7);
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;

        zone.accept(ZoneCommand::SpawnCharacter {
            character_id,
            position: WorldPositionMm::ORIGIN,
            inventory: CharacterInventoryState::new(10, 6),
            inventory_revision: 1,
        })?;
        zone.accept(ZoneCommand::MoveIntent {
            character_id,
            sequence: IntentSequence(1),
            input,
        })?;
        zone.step()?;

        let &entity = zone
            .entity_map
            .get(&character_id)
            .context("spawned character must remain owned in ECS")?;
        
        let position_val = {
            let pos = zone
                .world
                .get::<&Position>(entity)
                .context("character must have a Position component")?;
            pos.0
        };
        assert_eq!(position_val, WorldPositionMm::new(200, 0, 0));

        zone.accept(ZoneCommand::DespawnCharacter { character_id, responder: None,})?;
        assert!(!zone.entity_map.contains_key(&character_id));
        assert!(!zone.last_processed_inputs.contains_key(&character_id));
        assert!(!zone.world.contains(entity));
        Ok(())
    }

    #[test]
    fn zone_emits_authoritative_snapshot_after_processing_input() -> anyhow::Result<()> {
        let (snapshot_tx, mut snapshot_rx) = mpsc::channel(8);
        let mut zone = empty_zone(4_000, 5)?.with_movement_snapshots(snapshot_tx);
        let character_id = CharacterId(7);
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;

        zone.accept(ZoneCommand::SpawnCharacter {
            character_id,
            position: WorldPositionMm::ORIGIN,
            inventory: CharacterInventoryState::new(10, 6),
            inventory_revision: 1,
        })?;
        zone.accept(ZoneCommand::MoveIntent {
            character_id,
            sequence: IntentSequence(9),
            input,
        })?;
        zone.step()?;

        let snapshot = snapshot_rx.try_recv()?;
        assert_eq!(snapshot.character_id(), character_id);
        assert_eq!(snapshot.server_tick(), ServerTick(1));
        assert_eq!(snapshot.position(), WorldPositionMm::new(200, 0, 0));
        assert_eq!(snapshot.last_processed_input_sequence(), Some(IntentSequence(9)));
        Ok(())
    }

    #[test]
    fn zone_clamps_authoritative_movement_at_static_wall() -> anyhow::Result<()> {
        let (_tx, rx) = mpsc::channel(8);
        let mut traversal = TraversalWorld::default();
        traversal.add_static_blocker(StaticTraversalBlocker::new(650, 750, -2_000, 2_000)?);
        let mut zone = ZoneRuntime::new(ZoneId(3), 20, movement_settings(4_000, 10)?, traversal, rx)?;
        let character_id = CharacterId(7);
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;

        zone.accept(ZoneCommand::SpawnCharacter {
            character_id,
            position: WorldPositionMm::ORIGIN,
            inventory: CharacterInventoryState::new(10, 6),
            inventory_revision: 1,
        })?;
        zone.accept(ZoneCommand::MoveIntent {
            character_id,
            sequence: IntentSequence(1),
            input,
        })?;
        zone.step()?;
        zone.step()?;
        zone.step()?;

        let &entity = zone
            .entity_map
            .get(&character_id)
            .context("character must remain owned in ECS")?;
        let pos = zone
            .world
            .get::<&Position>(entity)
            .context("character must have a Position component")?;
        assert_eq!(pos.0, WorldPositionMm::new(400, 0, 0));
        Ok(())
    }

    #[test]
    fn zone_rejects_spawn_overlapping_static_blocker() -> anyhow::Result<()> {
        let (_tx, rx) = mpsc::channel(8);
        let mut traversal = TraversalWorld::default();
        traversal.add_static_blocker(StaticTraversalBlocker::new(650, 750, -2_000, 2_000)?);
        let mut zone = ZoneRuntime::new(ZoneId(3), 20, movement_settings(4_000, 5)?, traversal, rx)?;

        assert!(
            zone.accept(ZoneCommand::SpawnCharacter {
                character_id: CharacterId(7),
                position: WorldPositionMm::new(500, 0, 0),
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 1,
            })
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn zone_rejects_duplicate_spawn() -> anyhow::Result<()> {
        let mut zone = empty_zone(4_000, 5)?;
        let command = ZoneCommand::SpawnCharacter {
            character_id: CharacterId(7),
            position: WorldPositionMm::ORIGIN,
            inventory: CharacterInventoryState::new(10, 6),
            inventory_revision: 1,
        };

        zone.accept(command.clone())?;
        assert!(zone.accept(command).is_err());
        Ok(())
    }

    #[test]
    fn mining_rock_reduces_health_and_destroys_node() -> anyhow::Result<()> {
        let mut zone = empty_zone(4_000, 5)?;
        let character_id = CharacterId(42);

        zone.accept(ZoneCommand::SpawnCharacter {
            character_id,
            position: WorldPositionMm::ORIGIN,
            inventory: CharacterInventoryState::new(10, 6),
            inventory_revision: 1,
        })?;

        zone.accept(ZoneCommand::SpawnRock {
            position: WorldPositionMm::new(500, 0, 0),
            health: 20,
        })?;

        zone.accept(ZoneCommand::MineIntent {
            character_id,
            power: 10,
        })?;
        zone.step()?;

        let rock_count = zone.world.query::<&crate::components::RockNode>().iter().count();
        assert_eq!(rock_count, 1);

        zone.accept(ZoneCommand::MineIntent {
            character_id,
            power: 15,
        })?;
        zone.step()?;

        let rock_count_after = zone.world.query::<&crate::components::RockNode>().iter().count();
        assert_eq!(rock_count_after, 0);

        Ok(())
    }
}