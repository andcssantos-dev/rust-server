use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident, $inner:ty) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[repr(transparent)]
        pub struct $name(pub $inner);
    };
}

id_type!(UniverseId, u64);
id_type!(ZoneId, u64);
id_type!(EntityId, u64);
id_type!(AccountId, u64);
id_type!(CharacterId, u64);
id_type!(ContainerId, u64);
id_type!(ItemInstanceId, u128);
id_type!(DefinitionId, u32);
id_type!(ServerTick, u64);
id_type!(ConnectionId, u64);
id_type!(SessionId, u64);
id_type!(IntentSequence, u64);
