pub mod bootstrap;
pub mod environment_presentation;
pub mod frontier;
pub mod frontier_ack;
pub mod intents;
pub mod inventory;
pub mod prediction;
pub mod snapshots;
pub mod snapshots_v2;
pub mod terrain_authority;
pub mod water_surface_presentation;

// Reexportações públicas sem duplicatas
pub use bootstrap::{
    BootstrapCodecError, CLIENT_HELLO_BYTES, ClientHello, FRAME_HEADER_BYTES, FrameHeader,
    HandshakeRejectCode, SERVER_HELLO_BYTES, ServerHello,
};
pub use environment_presentation::{
    ENVIRONMENT_FAMILY_GROUND_COVER, ENVIRONMENT_FAMILY_KNOWN_MASK, ENVIRONMENT_FAMILY_ROCK,
    ENVIRONMENT_FAMILY_SHRUB, ENVIRONMENT_FAMILY_SMALL_PLANT, ENVIRONMENT_FAMILY_TREE,
    ENVIRONMENT_PRESENTATION_POLICY_VERSION, ENVIRONMENT_PRESENTATION_WIRE_HEADER_BYTES,
    EnvironmentPresentationPolicyError, EnvironmentPresentationPolicyV1,
    EnvironmentPresentationWireCodecError,
};
pub use frontier::{
    FRONTIER_MANIFEST_HEADER_BYTES, FRONTIER_MANIFEST_QUADRANT_BYTES, FRONTIER_MANIFEST_VERSION,
    FrontierManifest, FrontierManifestCodecError, FrontierQuadrantCoord,
};
pub use frontier_ack::{FRONTIER_MANIFEST_ACK_BYTES, FrontierManifestAck, FrontierManifestAckCodecError};
pub use intents::{
    IntentCodecError, MINE_INTENT_BYTES, MOVE_AXIS_MAX, MOVE_AXIS_MIN, MOVE_INTENT_BYTES, MineIntent,
    MoveIntent,
};
pub use prediction::{
    MOVEMENT_PREDICTION_PROFILE_BYTES, MOVEMENT_PREDICTION_PROFILE_VERSION, MovementPredictionProfile,
    PredictionProfileCodecError, PresentationCorrectionProfile,
};
pub use snapshots::{SELF_MOVEMENT_SNAPSHOT_BYTES, SelfMovementSnapshot, SnapshotCodecError};
pub use snapshots_v2::{
    MovementFlagsV2, SELF_MOVEMENT_SNAPSHOT_V2_BYTES, SelfMovementSnapshotV2, SnapshotV2CodecError,
};
pub use terrain_authority::{
    TERRAIN_AUTHORITY_CONTRACT_VERSION, TERRAIN_AUTHORITY_WIRE_HEADER_BYTES, TERRAIN_CELL_BLOCKED,
    TERRAIN_CELL_BUILDABLE, TERRAIN_CELL_CONNECTOR_CORRIDOR, TERRAIN_CELL_KNOWN_MASK, TERRAIN_CELL_WALKABLE,
    TERRAIN_CELL_WATER, TERRAIN_CONTROL_GRID_MAX_SIDE, TERRAIN_CONTROL_GRID_MIN_SIDE,
    TerrainAuthorityContractError, TerrainAuthorityContractV1, TerrainAuthorityWireCodecError,
};
pub use water_surface_presentation::{
    WATER_SURFACE_PRESENTATION_SAMPLE_BYTES, WATER_SURFACE_PRESENTATION_VERSION,
    WATER_SURFACE_PRESENTATION_WIRE_HEADER_BYTES, WaterSurfaceKindV1, WaterSurfacePresentationError,
    WaterSurfacePresentationV1, WaterSurfacePresentationWireCodecError, WaterSurfaceSampleV1,
};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 3;
pub const ENVIRONMENT_PRESENTATION_POLICY_MIN_PROTOCOL_MINOR: u16 = 1;
pub const WATER_SURFACE_PRESENTATION_MIN_PROTOCOL_MINOR: u16 = 2;
pub const AURENFALL_ALPN: &[u8] = b"aurenfall/1";

#[must_use]
pub fn negotiate_protocol_minor(
    client_major: u16,
    client_minor: u16,
    server_major: u16,
    server_minor: u16,
) -> Option<u16> {
    (client_major == server_major && client_minor <= server_minor).then_some(client_minor)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MessageKind {
    ClientHello = 1,
    ServerHello = 2,
    MovementPredictionProfile = 3,
    MoveIntent = 100,
    FrontierManifest = 200,
    EntityDelta = 201,
    SelfMovementSnapshot = 202,
    SelfMovementSnapshotV2 = 203,
    FrontierManifestAck = 204,
    TerrainAuthority = 205,
    EnvironmentPresentationPolicy = 206,
    WaterSurfacePresentation = 207,
    InventoryIntent = 300,
    InventoryResult = 301,
    MineIntent = 400,
}

impl TryFrom<u16> for MessageKind {
    type Error = BootstrapCodecError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::ClientHello),
            2 => Ok(Self::ServerHello),
            3 => Ok(Self::MovementPredictionProfile),
            100 => Ok(Self::MoveIntent),
            200 => Ok(Self::FrontierManifest),
            201 => Ok(Self::EntityDelta),
            202 => Ok(Self::SelfMovementSnapshot),
            203 => Ok(Self::SelfMovementSnapshotV2),
            204 => Ok(Self::FrontierManifestAck),
            205 => Ok(Self::TerrainAuthority),
            206 => Ok(Self::EnvironmentPresentationPolicy),
            207 => Ok(Self::WaterSurfacePresentation),
            300 => Ok(Self::InventoryIntent),
            301 => Ok(Self::InventoryResult),
            400 => Ok(Self::MineIntent),
            _ => Err(BootstrapCodecError::UnknownMessageKind(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_negotiation_accepts_same_major_and_supported_minor() {
        assert_eq!(negotiate_protocol_minor(1, 0, 1, 2), Some(0));
        assert_eq!(negotiate_protocol_minor(1, 1, 1, 2), Some(1));
        assert_eq!(negotiate_protocol_minor(1, 2, 1, 2), Some(2));
    }

    #[test]
    fn protocol_negotiation_rejects_future_minor_and_other_major() {
        assert_eq!(negotiate_protocol_minor(1, 3, 1, 2), None);
        assert_eq!(negotiate_protocol_minor(2, 0, 1, 1), None);
    }

    #[test]
    fn environment_presentation_policy_is_protocol_minor_one_capability() {
        assert_eq!(ENVIRONMENT_PRESENTATION_POLICY_MIN_PROTOCOL_MINOR, 1);
    }

    #[test]
    fn water_surface_presentation_is_protocol_minor_two_capability() {
        assert!(
            PROTOCOL_MINOR >= WATER_SURFACE_PRESENTATION_MIN_PROTOCOL_MINOR,
            "PROTOCOL_MINOR deve suportar a capacidade mínima de water surface"
        );
        assert_eq!(WATER_SURFACE_PRESENTATION_MIN_PROTOCOL_MINOR, 2);
    }

    #[test]
    fn frontier_control_messages_keep_reserved_world_message_kinds() -> Result<(), BootstrapCodecError> {
        assert_eq!(MessageKind::try_from(200)?, MessageKind::FrontierManifest);
        assert_eq!(MessageKind::try_from(204)?, MessageKind::FrontierManifestAck);
        assert_eq!(MessageKind::try_from(205)?, MessageKind::TerrainAuthority);
        assert_eq!(
            MessageKind::try_from(206)?,
            MessageKind::EnvironmentPresentationPolicy
        );
        assert_eq!(MessageKind::try_from(207)?, MessageKind::WaterSurfacePresentation);
        Ok(())
    }
}