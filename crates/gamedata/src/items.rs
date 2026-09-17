use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use aurenfall_core::DefinitionId;
use aurenfall_domain::items::ItemDefinition;

#[derive(Debug, Default, Clone)]
pub struct ItemRegistry {
    definitions: HashMap<DefinitionId, ItemDefinition>,
}

impl ItemRegistry {
    pub fn load_from_binary<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(&path)
            .with_context(|| format!("Falha ao abrir binário em {}", path.as_ref().display()))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        let items: Vec<ItemDefinition> = bincode::deserialize(&buffer)
            .context("Falha ao desserializar items.bin")?;

        let mut definitions = HashMap::with_capacity(items.len());
        for item in items {
            definitions.insert(item.id, item);
        }

        Ok(Self { definitions })
    }

    #[inline]
    pub fn get(&self, id: &DefinitionId) -> Option<&ItemDefinition> {
        self.definitions.get(id)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}