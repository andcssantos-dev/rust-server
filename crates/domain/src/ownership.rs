use aurenfall_core::{CharacterId, ContainerId, SectorCoord};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ownership {
    Character(CharacterId),
    Container(ContainerId),
    World { sector: SectorCoord },
}
