use aurenfall_contracts::{
    EnvironmentPresentationPolicyV1, MessageKind, TerrainAuthorityContractV1,
    WaterSurfacePresentationV1,
};

#[derive(Debug, Clone)]
pub struct EncodedQuadrantContracts {
    pub terrain_payload: Vec<u8>,
    pub environment_payload: Vec<u8>,
    pub water_payload: Vec<u8>,
}

impl EncodedQuadrantContracts {
    pub fn serialize(
        terrain: &TerrainAuthorityContractV1,
        environment: &EnvironmentPresentationPolicyV1,
        water: &WaterSurfacePresentationV1,
    ) -> anyhow::Result<Self> {
        let terrain_payload = terrain.encode_wire()?.to_vec();
        let environment_payload = environment.encode_wire()?.to_vec();
        let water_payload = water.encode_wire()?.to_vec();

        Ok(Self {
            terrain_payload,
            environment_payload,
            water_payload,
        })
    }

    pub fn message_kinds() -> (MessageKind, MessageKind, MessageKind) {
        (
            MessageKind::TerrainAuthority,
            MessageKind::EnvironmentPresentationPolicy,
            MessageKind::WaterSurfacePresentation,
        )
    }
}