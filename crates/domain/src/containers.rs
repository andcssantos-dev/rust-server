//! Módulo de Containers e Mochilas do Aurenfall.
//! 
//! Suporta árvore de containers, afixos mágicos, geometria 2D,
//! cálculo recursivo de peso, fusão/divisão de pilhas, auto-sort,
//! descarte (drop/destroy), depósito inteligente, sobrecarga e persistência (serde).

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Identificador numérico único de um container (ex: 1 = Bolsos, 2 = Mochila).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContainerId(pub u64);

/// Categorias de itens suportadas no jogo.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ItemCategory {
    General   = 1 << 0, // Itens diversos
    Herb      = 1 << 1, // Ervas e plantas
    Ore       = 1 << 2, // Minérios e pedras brutas
    Gem       = 1 << 3, // Pedras preciosas
    Equipment = 1 << 4, // Armas e armaduras
    Hazardous = 1 << 5, // Itens perigosos (pedras de fogo, venenos voláteis)
}

/// Filtro de categorias por máscara de bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemCategoryFilter(pub u32);

impl ItemCategoryFilter {
    pub const ANY: Self = Self(u32::MAX);

    pub fn only(category: ItemCategory) -> Self {
        Self(category as u32)
    }

    pub fn allow(&mut self, category: ItemCategory) {
        self.0 |= category as u32;
    }

    pub fn allows(&self, category: ItemCategory) -> bool {
        (self.0 & (category as u32)) != 0
    }
}

/// Faixas de sobrecarga de peso do personagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncumbranceTier {
    Light,        // Leve (0% a 50%): sem penalidades
    Medium,       // Médio (50% a 80%): pequeno gasto extra de estamina
    Heavy,        // Pesado (80% a 100%): velocidade reduzida
    Overburdened, // Sobrecarregado (> 100%): impede corrida e esquiva
}

/// Propriedades mágicas que uma bolsa pode possuir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContainerAffix {
    /// Reduz o peso do conteúdo em porcentagem (ex: 20 = 20% de redução).
    WeightReduction(u8),
    /// Bolsa térmica: retarda o apodrecimento (ex: 0.2 faz durar 5x mais).
    FoodPreservation(f32),
    /// Isolamento térmico/ignífugo: permite guardar itens em chamas sem dano.
    Fireproof,
}

/// Formato de armazenamento interno do container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerContent {
    Empty,
    Grid { columns: u8, rows: u8 },
    Slots { capacity: usize },
}

/// Representa um item guardado dentro de uma mochila com sua posição física 2D.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredItem {
    pub id: u64,                  // Identificador único da instância
    pub definition_id: u32,       // Tipo do item (ex: 500 = Pedra, 501 = Erva da Lua)
    pub category: ItemCategory,   // Categoria do item
    pub unit_weight_grams: u32,   // Peso unitário em gramas
    pub quantity: u32,            // Quantidade nesta pilha
    pub max_stack: u32,           // Limite máximo da pilha (1 para armas/armaduras)
    pub width: u8,                // Largura em slots
    pub height: u8,               // Altura em slots
    pub x: u8,                    // Posição horizontal na grade
    pub y: u8,                    // Posição vertical na grade
    pub rotated: bool,            // Se o item foi girado 90 graus
}

impl StoredItem {
    pub fn new(
        id: u64,
        definition_id: u32,
        category: ItemCategory,
        unit_weight_grams: u32,
        quantity: u32,
        max_stack: u32,
        width: u8,
        height: u8,
    ) -> Self {
        Self {
            id,
            definition_id,
            category,
            unit_weight_grams,
            quantity,
            max_stack,
            width,
            height,
            x: 0,
            y: 0,
            rotated: false,
        }
    }

    pub fn total_weight_grams(&self) -> u32 {
        self.unit_weight_grams.saturating_mul(self.quantity)
    }

    pub fn area(&self) -> u16 {
        (self.width as u16) * (self.height as u16)
    }

    pub fn effective_width(&self) -> u8 {
        if self.rotated { self.height } else { self.width }
    }

    pub fn effective_height(&self) -> u8 {
        if self.rotated { self.width } else { self.height }
    }

    pub fn collides_with(&self, other: &StoredItem) -> bool {
        let self_w = self.effective_width();
        let self_h = self.effective_height();
        let other_w = other.effective_width();
        let other_h = other.effective_height();

        self.x < other.x + other_w
            && self.x + self_w > other.x
            && self.y < other.y + other_h
            && self.y + self_h > other.y
    }
}

/// Representa uma mochila ou recipiente específico no jogo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInstance {
    pub id: ContainerId,
    pub parent_container: Option<ContainerId>,
    pub allowed_tags: ItemCategoryFilter,
    pub content: ContainerContent,
    pub items: Vec<StoredItem>,
    pub base_weight_grams: u32,
    pub affixes: Vec<ContainerAffix>,
    pub nesting_allowed: bool,
}

impl ContainerInstance {
    pub fn new_grid(id: ContainerId, columns: u8, rows: u8, base_weight_grams: u32) -> Self {
        Self {
            id,
            parent_container: None,
            allowed_tags: ItemCategoryFilter::ANY,
            content: ContainerContent::Grid { columns, rows },
            items: Vec::new(),
            base_weight_grams,
            affixes: Vec::new(),
            nesting_allowed: true,
        }
    }

    pub fn new_specialized(
        id: ContainerId,
        columns: u8,
        rows: u8,
        base_weight_grams: u32,
        allowed_category: ItemCategory,
    ) -> Self {
        Self {
            id,
            parent_container: None,
            allowed_tags: ItemCategoryFilter::only(allowed_category),
            content: ContainerContent::Grid { columns, rows },
            items: Vec::new(),
            base_weight_grams,
            affixes: Vec::new(),
            nesting_allowed: false,
        }
    }

    pub fn add_affix(&mut self, affix: ContainerAffix) {
        self.affixes.push(affix);
    }

    pub fn total_weight_reduction_percent(&self) -> u8 {
        let mut total = 0u16;
        for affix in &self.affixes {
            if let ContainerAffix::WeightReduction(percent) = affix {
                total += *percent as u16;
            }
        }
        total.min(100) as u8
    }

    pub fn is_fireproof(&self) -> bool {
        self.affixes.iter().any(|a| matches!(a, ContainerAffix::Fireproof))
    }
}

/// Árvore completa de containers do personagem.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CharacterContainerTree {
    pub root_id: Option<ContainerId>,
    pub containers: HashMap<ContainerId, ContainerInstance>,
}

impl CharacterContainerTree {
    pub fn new() -> Self {
        Self {
            root_id: None,
            containers: HashMap::new(),
        }
    }

    pub fn set_root(&mut self, root: ContainerInstance) {
        let id = root.id;
        self.root_id = Some(id);
        self.containers.insert(id, root);
    }

    pub fn add_container(&mut self, container: ContainerInstance) {
        self.containers.insert(container.id, container);
    }

    pub fn attach_to_parent(
        &mut self,
        bag_to_move: ContainerId,
        target_parent: ContainerId,
        max_depth: u32,
    ) -> Result<(), &'static str> {
        if self.would_create_cycle(bag_to_move, target_parent) {
            return Err("Operação inválida: criaria um ciclo infinito de mochilas.");
        }

        let parent = self.containers.get(&target_parent)
            .ok_or("Container de destino não encontrado.")?;

        if !parent.nesting_allowed {
            return Err("Este container não aceita outras bolsas dentro dele.");
        }

        let parent_depth = self.get_depth(target_parent);
        if parent_depth + 1 > max_depth {
            return Err("Profundidade máxima de bolsas aninhadas atingida.");
        }

        if let Some(bag) = self.containers.get_mut(&bag_to_move) {
            bag.parent_container = Some(target_parent);
            Ok(())
        } else {
            Err("Mochila a ser movida não encontrada.")
        }
    }

    pub fn would_create_cycle(&self, moving_container: ContainerId, target_container: ContainerId) -> bool {
        if moving_container == target_container {
            return true;
        }

        let mut current_parent = self.containers.get(&target_container).and_then(|c| c.parent_container);
        while let Some(parent_id) = current_parent {
            if parent_id == moving_container {
                return true;
            }
            current_parent = self.containers.get(&parent_id).and_then(|c| c.parent_container);
        }

        false
    }

    pub fn get_depth(&self, mut container_id: ContainerId) -> u32 {
        let mut depth = 0;
        while let Some(container) = self.containers.get(&container_id) {
            match container.parent_container {
                Some(parent_id) => {
                    depth += 1;
                    container_id = parent_id;
                }
                None => break,
            }
        }
        depth
    }

    pub fn calculate_container_weight(&self, container_id: ContainerId) -> Result<u32, &'static str> {
        let container = self.containers.get(&container_id)
            .ok_or("Container não encontrado.")?;

        let direct_items_weight: u32 = container.items.iter().map(|item| item.total_weight_grams()).sum();

        let mut child_bags_weight: u32 = 0;
        for child in self.containers.values() {
            if child.parent_container == Some(container_id) {
                let child_weight = self.calculate_container_weight(child.id)?;
                child_bags_weight = child_bags_weight.saturating_add(child_weight);
            }
        }

        let total_content_weight = direct_items_weight.saturating_add(child_bags_weight);
        let reduction_percent = container.total_weight_reduction_percent() as u64;
        let discount = (total_content_weight as u64 * reduction_percent / 100) as u32;
        let final_content_weight = total_content_weight.saturating_sub(discount);

        Ok(container.base_weight_grams.saturating_add(final_content_weight))
    }

    pub fn calculate_total_weight(&self) -> Result<u32, &'static str> {
        match self.root_id {
            Some(root_id) => self.calculate_container_weight(root_id),
            None => Ok(0),
        }
    }

    pub fn get_encumbrance_tier(&self, max_capacity_grams: u32) -> EncumbranceTier {
        let current_weight = self.calculate_total_weight().unwrap_or(0);

        if max_capacity_grams == 0 {
            return EncumbranceTier::Overburdened;
        }

        let percentage = (current_weight as u64 * 100) / max_capacity_grams as u64;

        if percentage <= 50 {
            EncumbranceTier::Light
        } else if percentage <= 80 {
            EncumbranceTier::Medium
        } else if percentage <= 100 {
            EncumbranceTier::Heavy
        } else {
            EncumbranceTier::Overburdened
        }
    }

    pub fn quick_insert(
        &mut self,
        container_id: ContainerId,
        mut item: StoredItem,
        allow_rotation: bool,
    ) -> Result<StoredItem, &'static str> {
        let container = self.containers.get_mut(&container_id)
            .ok_or("Container de destino não encontrado.")?;

        if !container.allowed_tags.allows(item.category) {
            return Err("Este item não é permitido neste tipo de bolsa.");
        }

        if item.category == ItemCategory::Hazardous && !container.is_fireproof() {
            return Err("Este item incandescente queima uma bolsa comum. Exige bolsa à prova de fogo.");
        }

        if item.max_stack > 1 {
            for existing in container.items.iter_mut() {
                if existing.definition_id == item.definition_id && existing.quantity < existing.max_stack {
                    let available_space = existing.max_stack - existing.quantity;
                    let amount_to_add = item.quantity.min(available_space);

                    existing.quantity += amount_to_add;
                    item.quantity -= amount_to_add;

                    if item.quantity == 0 {
                        return Ok(existing.clone());
                    }
                }
            }
        }

        let (columns, rows) = match container.content {
            ContainerContent::Grid { columns, rows } => (columns, rows),
            _ => return Err("Quick-insert só é suportado em containers do tipo Grade."),
        };

        let free_position = Self::find_free_slot(
            columns,
            rows,
            &container.items,
            item.width,
            item.height,
            allow_rotation,
        );

        if let Some((found_x, found_y, is_rotated)) = free_position {
            item.x = found_x;
            item.y = found_y;
            item.rotated = is_rotated;
            container.items.push(item.clone());
            Ok(item)
        } else {
            Err("Espaço insuficiente na mochila.")
        }
    }

    fn find_free_slot(
        columns: u8,
        rows: u8,
        existing_items: &[StoredItem],
        width: u8,
        height: u8,
        allow_rotation: bool,
    ) -> Option<(u8, u8, bool)> {
        for y in 0..rows {
            for x in 0..columns {
                let mut candidate = StoredItem {
                    id: 0,
                    definition_id: 0,
                    category: ItemCategory::General,
                    unit_weight_grams: 0,
                    quantity: 1,
                    max_stack: 1,
                    width,
                    height,
                    x,
                    y,
                    rotated: false,
                };

                if x + candidate.effective_width() <= columns && y + candidate.effective_height() <= rows {
                    let has_collision = existing_items.iter().any(|existing| candidate.collides_with(existing));
                    if !has_collision {
                        return Some((x, y, false));
                    }
                }

                if allow_rotation && width != height {
                    candidate.rotated = true;
                    if x + candidate.effective_width() <= columns && y + candidate.effective_height() <= rows {
                        let has_collision = existing_items.iter().any(|existing| candidate.collides_with(existing));
                        if !has_collision {
                            return Some((x, y, true));
                        }
                    }
                }
            }
        }

        None
    }

    pub fn split_stack(
        &mut self,
        container_id: ContainerId,
        source_item_id: u64,
        amount_to_split: u32,
        new_item_id: u64,
    ) -> Result<StoredItem, &'static str> {
        let container = self.containers.get_mut(&container_id)
            .ok_or("Container não encontrado.")?;

        let (columns, rows) = match container.content {
            ContainerContent::Grid { columns, rows } => (columns, rows),
            _ => return Err("Divisão de pilha só é suportada em grades."),
        };

        let item_idx = container.items.iter().position(|i| i.id == source_item_id)
            .ok_or("Item de origem não encontrado.")?;

        let original_item = &mut container.items[item_idx];

        if amount_to_split == 0 {
            return Err("A quantidade para dividir deve ser maior que zero.");
        }
        if amount_to_split >= original_item.quantity {
            return Err("A quantidade para dividir deve ser menor que a quantidade da pilha.");
        }

        let mut new_item = StoredItem::new(
            new_item_id,
            original_item.definition_id,
            original_item.category,
            original_item.unit_weight_grams,
            amount_to_split,
            original_item.max_stack,
            original_item.width,
            original_item.height,
        );

        original_item.quantity -= amount_to_split;

        let free_pos = Self::find_free_slot(
            columns,
            rows,
            &container.items,
            new_item.width,
            new_item.height,
            true,
        );

        match free_pos {
            Some((fx, fy, rotated)) => {
                new_item.x = fx;
                new_item.y = fy;
                new_item.rotated = rotated;
                container.items.push(new_item.clone());
                Ok(new_item)
            }
            None => {
                container.items[item_idx].quantity += amount_to_split;
                Err("Não há espaço livre na mochila para colocar a pilha dividida.")
            }
        }
    }

    pub fn auto_sort(&mut self, container_id: ContainerId) -> Result<(), &'static str> {
        let container = self.containers.get_mut(&container_id)
            .ok_or("Container não encontrado.")?;

        let (columns, rows) = match container.content {
            ContainerContent::Grid { columns, rows } => (columns, rows),
            _ => return Err("Auto-sort só é suportado em grades."),
        };

        let mut items_to_sort = std::mem::take(&mut container.items);

        items_to_sort.sort_by(|a, b| {
            b.area()
                .cmp(&a.area())
                .then_with(|| (b.category as u32).cmp(&(a.category as u32)))
                .then_with(|| b.definition_id.cmp(&a.definition_id))
        });

        for mut item in items_to_sort {
            let free_pos = Self::find_free_slot(
                columns,
                rows,
                &container.items,
                item.width,
                item.height,
                true,
            );

            if let Some((fx, fy, rotated)) = free_pos {
                item.x = fx;
                item.y = fy;
                item.rotated = rotated;
                container.items.push(item);
            } else {
                return Err("Falha catastrófica no auto-sort: item não coube ao reorganizar.");
            }
        }

        Ok(())
    }

    pub fn destroy_item(&mut self, container_id: ContainerId, item_id: u64) -> Result<(), &'static str> {
        let container = self.containers.get_mut(&container_id)
            .ok_or("Container não encontrado.")?;

        let idx = container.items.iter().position(|i| i.id == item_id)
            .ok_or("Item não encontrado no container.")?;

        container.items.remove(idx);
        Ok(())
    }

    pub fn drop_item(&mut self, container_id: ContainerId, item_id: u64) -> Result<StoredItem, &'static str> {
        let container = self.containers.get_mut(&container_id)
            .ok_or("Container não encontrado.")?;

        let idx = container.items.iter().position(|i| i.id == item_id)
            .ok_or("Item não encontrado no container.")?;

        Ok(container.items.remove(idx))
    }

    pub fn deposit_matching(
        &mut self,
        source_id: ContainerId,
        target_id: ContainerId,
    ) -> Result<usize, &'static str> {
        if source_id == target_id {
            return Err("Origem e destino não podem ser o mesmo container.");
        }

        let target_definitions: Vec<u32> = {
            let target = self.containers.get(&target_id)
                .ok_or("Container de destino não encontrado.")?;
            target.items.iter().map(|item| item.definition_id).collect()
        };

        let matching_item_ids: Vec<u64> = {
            let source = self.containers.get(&source_id)
                .ok_or("Container de origem não encontrado.")?;
            source.items.iter()
                .filter(|item| target_definitions.contains(&item.definition_id))
                .map(|item| item.id)
                .collect()
        };

        let mut transferred_count = 0;
        for item_id in matching_item_ids {
            if let Ok(removed_item) = self.drop_item(source_id, item_id) {
                match self.quick_insert(target_id, removed_item.clone(), true) {
                    Ok(_) => transferred_count += 1,
                    Err(_) => {
                        let source = self.containers.get_mut(&source_id).unwrap();
                        source.items.push(removed_item);
                        break;
                    }
                }
            }
        }

        Ok(transferred_count)
    }
}

// =========================================================================
// TESTES AUTOMATIZADOS
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_herb_bag_rejects_ore() {
        let herb_pouch = ContainerInstance::new_specialized(
            ContainerId(2),
            4, 4, 200,
            ItemCategory::Herb,
        );

        let mut tree = CharacterContainerTree::new();
        tree.add_container(herb_pouch);

        let herb = StoredItem::new(101, 1, ItemCategory::Herb, 50, 1, 1, 1, 1);
        let ore = StoredItem::new(102, 2, ItemCategory::Ore, 1500, 1, 1, 1, 1);

        assert!(tree.quick_insert(ContainerId(2), herb, false).is_ok());

        let result = tree.quick_insert(ContainerId(2), ore, false);
        assert_eq!(result, Err("Este item não é permitido neste tipo de bolsa."));
    }

    #[test]
    fn test_prevent_infinite_cycle() {
        let mut tree = CharacterContainerTree::new();
        let bag_a = ContainerInstance::new_grid(ContainerId(1), 6, 6, 500);
        let bag_b = ContainerInstance::new_grid(ContainerId(2), 4, 4, 300);

        tree.add_container(bag_a);
        tree.add_container(bag_b);

        assert!(tree.attach_to_parent(ContainerId(2), ContainerId(1), 2).is_ok());
        assert!(tree.attach_to_parent(ContainerId(1), ContainerId(2), 2).is_err());
    }

    #[test]
    fn test_max_depth_enforced() {
        let mut tree = CharacterContainerTree::new();
        let root = ContainerInstance::new_grid(ContainerId(1), 10, 6, 0);
        let bag = ContainerInstance::new_grid(ContainerId(2), 4, 4, 200);
        let pouch = ContainerInstance::new_grid(ContainerId(3), 2, 2, 100);

        tree.set_root(root);
        tree.add_container(bag);
        tree.add_container(pouch);

        assert!(tree.attach_to_parent(ContainerId(2), ContainerId(1), 1).is_ok());
        let result = tree.attach_to_parent(ContainerId(3), ContainerId(2), 1);
        assert_eq!(result, Err("Profundidade máxima de bolsas aninhadas atingida."));
    }

    #[test]
    fn test_multiple_magical_affixes_and_fireproof() {
        let mut tree = CharacterContainerTree::new();
        let mut special_bag = ContainerInstance::new_grid(ContainerId(1), 4, 4, 500);
        special_bag.add_affix(ContainerAffix::WeightReduction(30));
        special_bag.add_affix(ContainerAffix::FoodPreservation(0.2));
        special_bag.add_affix(ContainerAffix::Fireproof);

        tree.add_container(special_bag);

        let fire_stone = StoredItem::new(1, 999, ItemCategory::Hazardous, 1000, 1, 1, 1, 1);
        assert!(tree.quick_insert(ContainerId(1), fire_stone, false).is_ok());

        let common_bag = ContainerInstance::new_grid(ContainerId(2), 4, 4, 200);
        tree.add_container(common_bag);
        let fire_stone_2 = StoredItem::new(2, 999, ItemCategory::Hazardous, 1000, 1, 1, 1, 1);
        let err = tree.quick_insert(ContainerId(2), fire_stone_2, false);
        assert_eq!(err, Err("Este item incandescente queima uma bolsa comum. Exige bolsa à prova de fogo."));
    }

    #[test]
    fn test_stack_merge_on_quick_insert() {
        let mut tree = CharacterContainerTree::new();
        let bag = ContainerInstance::new_grid(ContainerId(1), 4, 4, 100);
        tree.add_container(bag);

        let single_stone = StoredItem::new(1, 500, ItemCategory::Ore, 100, 1, 99, 1, 1);
        tree.quick_insert(ContainerId(1), single_stone, false).unwrap();

        let more_stones = StoredItem::new(2, 500, ItemCategory::Ore, 100, 10, 99, 1, 1);
        tree.quick_insert(ContainerId(1), more_stones, false).unwrap();

        let bag_ref = tree.containers.get(&ContainerId(1)).unwrap();
        assert_eq!(bag_ref.items.len(), 1);
        assert_eq!(bag_ref.items[0].quantity, 11);
    }

    #[test]
    fn test_split_stack_success_and_rollback() {
        let mut tree = CharacterContainerTree::new();
        let bag = ContainerInstance::new_grid(ContainerId(1), 2, 1, 100);
        tree.add_container(bag);

        let stones = StoredItem::new(1, 500, ItemCategory::Ore, 100, 50, 99, 1, 1);
        tree.quick_insert(ContainerId(1), stones, false).unwrap();

        let split_item = tree.split_stack(ContainerId(1), 1, 20, 2).unwrap();
        assert_eq!(split_item.quantity, 20);

        let bag_ref = tree.containers.get(&ContainerId(1)).unwrap();
        assert_eq!(bag_ref.items.len(), 2);
        assert_eq!(bag_ref.items[0].quantity, 30);
        assert_eq!(bag_ref.items[1].quantity, 20);

        let err = tree.split_stack(ContainerId(1), 1, 10, 3);
        assert_eq!(err, Err("Não há espaço livre na mochila para colocar a pilha dividida."));

        let bag_ref_after = tree.containers.get(&ContainerId(1)).unwrap();
        assert_eq!(bag_ref_after.items[0].quantity, 30);
    }

    #[test]
    fn test_auto_sort_packs_largest_items_first() {
        let mut tree = CharacterContainerTree::new();
        let bag = ContainerInstance::new_grid(ContainerId(1), 4, 4, 100);
        tree.add_container(bag);

        let potion = StoredItem::new(1, 10, ItemCategory::General, 100, 1, 1, 1, 1);
        tree.quick_insert(ContainerId(1), potion, false).unwrap();

        let axe = StoredItem::new(2, 20, ItemCategory::Equipment, 2000, 1, 1, 2, 2);
        tree.quick_insert(ContainerId(1), axe, false).unwrap();

        tree.auto_sort(ContainerId(1)).unwrap();

        let bag_ref = tree.containers.get(&ContainerId(1)).unwrap();
        assert_eq!(bag_ref.items[0].id, 2);
        assert_eq!(bag_ref.items[0].x, 0);
        assert_eq!(bag_ref.items[0].y, 0);
        assert_eq!(bag_ref.items[1].id, 1);
    }

    #[test]
    fn test_destroy_and_drop_item() {
        let mut tree = CharacterContainerTree::new();
        let bag = ContainerInstance::new_grid(ContainerId(1), 4, 4, 100);
        tree.add_container(bag);

        let potion = StoredItem::new(1, 10, ItemCategory::General, 100, 1, 1, 1, 1);
        let sword = StoredItem::new(2, 20, ItemCategory::Equipment, 1000, 1, 1, 1, 2);

        tree.quick_insert(ContainerId(1), potion, false).unwrap();
        tree.quick_insert(ContainerId(1), sword, false).unwrap();

        assert!(tree.destroy_item(ContainerId(1), 1).is_ok());
        assert_eq!(tree.containers.get(&ContainerId(1)).unwrap().items.len(), 1);

        let dropped = tree.drop_item(ContainerId(1), 2).unwrap();
        assert_eq!(dropped.id, 2);
        assert_eq!(tree.containers.get(&ContainerId(1)).unwrap().items.len(), 0);
    }

    #[test]
    fn test_deposit_matching_transfers_only_existing_types() {
        let mut tree = CharacterContainerTree::new();
        let mut backpack = ContainerInstance::new_grid(ContainerId(1), 6, 6, 100);
        let mut storage_chest = ContainerInstance::new_grid(ContainerId(2), 6, 6, 500);

        let chest_iron = StoredItem::new(1, 100, ItemCategory::Ore, 500, 1, 99, 1, 1);
        storage_chest.items.push(chest_iron);

        let player_iron = StoredItem::new(2, 100, ItemCategory::Ore, 500, 5, 99, 1, 1);
        let player_apple = StoredItem::new(3, 200, ItemCategory::General, 50, 1, 20, 1, 1);
        backpack.items.push(player_iron);
        backpack.items.push(player_apple);

        tree.add_container(backpack);
        tree.add_container(storage_chest);

        let transferred = tree.deposit_matching(ContainerId(1), ContainerId(2)).unwrap();
        assert_eq!(transferred, 1);

        let backpack_ref = tree.containers.get(&ContainerId(1)).unwrap();
        assert_eq!(backpack_ref.items.len(), 1);
        assert_eq!(backpack_ref.items[0].definition_id, 200);

        let chest_ref = tree.containers.get(&ContainerId(2)).unwrap();
        assert_eq!(chest_ref.items[0].quantity, 6);
    }

    #[test]
    fn test_encumbrance_tiers() {
        let mut tree = CharacterContainerTree::new();
        let root = ContainerInstance::new_grid(ContainerId(1), 10, 6, 0);
        tree.set_root(root);

        let max_capacity = 10_000;

        assert_eq!(tree.get_encumbrance_tier(max_capacity), EncumbranceTier::Light);

        let medium_weight = StoredItem::new(1, 1, ItemCategory::General, 6000, 1, 1, 1, 1);
        tree.quick_insert(ContainerId(1), medium_weight, false).unwrap();
        assert_eq!(tree.get_encumbrance_tier(max_capacity), EncumbranceTier::Medium);

        let heavy_weight = StoredItem::new(2, 2, ItemCategory::General, 3000, 1, 1, 1, 1);
        tree.quick_insert(ContainerId(1), heavy_weight, false).unwrap();
        assert_eq!(tree.get_encumbrance_tier(max_capacity), EncumbranceTier::Heavy);

        let extra_weight = StoredItem::new(3, 3, ItemCategory::General, 2000, 1, 1, 1, 1);
        tree.quick_insert(ContainerId(1), extra_weight, false).unwrap();
        assert_eq!(tree.get_encumbrance_tier(max_capacity), EncumbranceTier::Overburdened);
    }
}