use aurenfall_core::{CharacterId, ItemInstanceId};
use aurenfall_domain::items::{DroppedBagId, GridSlotCoord, ItemInstance, ItemRotation, EquipmentSlotKind};
use serde::{Deserialize, Serialize};

/// Pacote leve transmitido a 20 Hz por interesse espacial.
/// O cliente UE5 usa isso apenas para spawnar o Mesh 3D no chão.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BagWorldReplication {
    pub bag_id: DroppedBagId,
    pub original_owner: CharacterId,
    pub world_x: f32,
    pub world_y: f32,
}

/// Evento de remoção enviado quando a bolsa expira (decay) ou é esvaziada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DespawnBagNotification {
    pub bag_id: DroppedBagId,
}

/// Solicitação do cliente UE5 para abrir e inspecionar a mochila.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestOpenBag {
    pub bag_id: DroppedBagId,
}

/// Payload completo enviado exclusivamente para o jogador que abriu o container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BagContentsSnapshot {
    pub bag_id: DroppedBagId,
    pub items: Vec<ItemInstance>,
}

/// Saque de container para inventário com rotação e célula de destino.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestLootItem {
    pub bag_id: DroppedBagId,
    pub item_id: ItemInstanceId,
    pub target_slot: GridSlotCoord,
    pub rotation: ItemRotation,
    pub expected_revision: u64,
}

/// Movimentação ou rotação de peça dentro da própria grade do personagem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestMoveOrRotateItem {
    pub item_id: ItemInstanceId,
    pub target_slot: GridSlotCoord,
    pub rotation: ItemRotation,
    pub expected_revision: u64,
}

/// Fusão atômica de duas pilhas de itens idênticos no inventário.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestMergeStack {
    pub source_item_id: ItemInstanceId,
    pub target_item_id: ItemInstanceId,
    pub expected_source_revision: u64,
    pub expected_target_revision: u64,
}

/// Solicitação para destacar uma quantidade de uma pilha para uma nova célula da grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestSplitStack {
    pub source_item_id: ItemInstanceId,
    pub split_quantity: u32,
    pub target_slot: GridSlotCoord,
    pub rotation: ItemRotation,
    pub expected_source_revision: u64,
}

/// Resposta autoritativa de operações de saque.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LootResult {
    Success { item: ItemInstance },
    BagNotFound,
    ItemNotFound,
    OutOfRange,
    InventoryFull,
    ConcurrencyConflict,
}

/// Resposta de operações de manipulação interna do inventário.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InventoryActionResult {
    Success,
    ItemNotFound,
    InvalidPlacement,
    StackLimitExceeded,
    InvalidSplitQuantity,
    IncompatibleItems,
    IncompatibleSlot,
    SlotAlreadyOccupied,
    ConcurrencyConflict,
}

/// Intenção do jogador de largar um item no chão nas suas coordenadas atuais.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDropItem {
    pub item_id: ItemInstanceId,
    pub expected_revision: u64,
}

/// Resposta autoritativa da solicitação de drop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DropResult {
    Success {
        bag_id: DroppedBagId,
        dropped_item: ItemInstance,
    },
    ItemNotFound,
    ConcurrencyConflict,
}

/// Intenção do jogador de mover um item da grade para um slot de equipamento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEquipItem {
    pub item_id: ItemInstanceId,
    pub target_slot: EquipmentSlotKind,
    pub expected_revision: u64,
}

/// Intenção de desequipar uma peça do corpo para uma célula livre da grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestUnequipItem {
    pub source_slot: EquipmentSlotKind,
    pub target_slot: GridSlotCoord,
    pub rotation: ItemRotation,
    pub expected_revision: u64,
}