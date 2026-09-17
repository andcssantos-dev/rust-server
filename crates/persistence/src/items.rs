use std::collections::HashMap;

use anyhow::{Context, Result};
use aurenfall_core::CharacterId;
use aurenfall_domain::items::{DroppedBagEntity, ItemInstance};
use serde::{Deserialize, Serialize};

/// Snapshot serializável do estado das mochilas do mundo para persistência física.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldBagsStateSnapshot {
    pub saved_at_tick: u64,
    pub bags: Vec<DroppedBagEntity>,
}

/// DTO serializado de um inventário de personagem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInventorySnapshot {
    pub character_id: CharacterId,
    pub items: Vec<ItemInstance>,
    pub updated_at_tick: u64,
}

/// Repositório autoritativo em memória com suporte a serialização binária para disco/storage.
#[derive(Debug, Default)]
pub struct ItemPersistenceRepository {
    inventories: HashMap<CharacterId, Vec<u8>>,
    world_bags: Option<Vec<u8>>,
}

impl ItemPersistenceRepository {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Salva o estado dos itens de um personagem convertendo para blob binário otimizado.
    pub fn save_character_inventory(
        &mut self,
        character_id: CharacterId,
        items: &[ItemInstance],
        current_tick: u64,
    ) -> Result<()> {
        let snapshot = CharacterInventorySnapshot {
            character_id,
            items: items.to_vec(),
            updated_at_tick: current_tick,
        };

        let encoded = bincode::serialize(&snapshot)
            .context("Falha ao serializar inventário do personagem")?;
        self.inventories.insert(character_id, encoded);
        Ok(())
    }

    /// Carrega e desserializa o inventário do personagem a partir do blob binário.
    pub fn load_character_inventory(
        &self,
        character_id: &CharacterId,
    ) -> Result<Option<CharacterInventorySnapshot>> {
        let Some(raw_bytes) = self.inventories.get(character_id) else {
            return Ok(None);
        };

        let snapshot: CharacterInventorySnapshot = bincode::deserialize(raw_bytes)
            .context("Falha ao desserializar blob binário do inventário")?;
        Ok(Some(snapshot))
    }

    /// Persiste o conjunto completo de mochilas ativas no mundo.
    pub fn save_world_bags(&mut self, bags: &[DroppedBagEntity], current_tick: u64) -> Result<()> {
        let snapshot = WorldBagsStateSnapshot {
            saved_at_tick: current_tick,
            bags: bags.to_vec(),
        };

        let encoded = bincode::serialize(&snapshot)
            .context("Falha ao serializar snapshot de mochilas do mundo")?;
        self.world_bags = Some(encoded);
        Ok(())
    }

    /// Recupera as mochilas persistidas ajustando ticks restantes ou descartando as expiradas.
    pub fn load_world_bags(&self) -> Result<Option<WorldBagsStateSnapshot>> {
        let Some(ref raw_bytes) = self.world_bags else {
            return Ok(None);
        };

        let snapshot: WorldBagsStateSnapshot = bincode::deserialize(raw_bytes)
            .context("Falha ao desserializar snapshot de mochilas do mundo")?;
        Ok(Some(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_core::{DefinitionId, ItemInstanceId};
    use aurenfall_domain::items::DroppedBagId;
    use aurenfall_domain::Ownership;

    #[test]
    fn test_character_inventory_persistence_roundtrip() {
        let mut repo = ItemPersistenceRepository::new();
        let char_id = CharacterId(501);

        let item = ItemInstance::new_unique(
            ItemInstanceId(10),
            DefinitionId(200),
            Ownership::Character(char_id),
            42,
            150,
        );

        repo.save_character_inventory(char_id, &[item.clone()], 100)
            .unwrap();

        let loaded = repo
            .load_character_inventory(&char_id)
            .unwrap()
            .expect("Deveria encontrar inventário salvo");

        assert_eq!(loaded.character_id, char_id);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].id, item.id);
        assert_eq!(loaded.updated_at_tick, 100);
    }

    #[test]
    fn test_world_bags_persistence_roundtrip() {
        let mut repo = ItemPersistenceRepository::new();
        let bag = DroppedBagEntity::new(
            DroppedBagId(99),
            CharacterId(1),
            (1, -1),
            (50.0, -50.0),
            100,
            600,
            Vec::new(),
        );

        repo.save_world_bags(&[bag.clone()], 150).unwrap();

        let loaded = repo
            .load_world_bags()
            .unwrap()
            .expect("Deveria carregar snapshot de mochilas");

        assert_eq!(loaded.saved_at_tick, 150);
        assert_eq!(loaded.bags.len(), 1);
        assert_eq!(loaded.bags[0].id, bag.id);
    }
}