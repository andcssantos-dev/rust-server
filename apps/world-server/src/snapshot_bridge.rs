use std::collections::{HashMap, hash_map::Entry};

use anyhow::{Context, bail};
use aurenfall_contracts::{MessageKind, SelfMovementSnapshotV2};
use aurenfall_core::{CharacterId, ConnectionId};
use aurenfall_simulation::MovementSnapshot;
use aurenfall_transport::{RealtimeConnectionSender, RealtimeSendError};


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotRouteOutcome {
    Sent,
    DroppedNoRoute,
    DroppedRealtime,
}

#[derive(Debug, Default)]
pub struct SelfSnapshotBridge {
    realtime_by_connection: HashMap<ConnectionId, RealtimeConnectionSender>,
    connection_by_character: HashMap<CharacterId, ConnectionId>,
}

impl SelfSnapshotBridge {
    pub fn register_connection(&mut self, realtime: RealtimeConnectionSender) -> anyhow::Result<()> {
        let connection_id = realtime.connection_id();
        match self.realtime_by_connection.entry(connection_id) {
            Entry::Vacant(entry) => {
                entry.insert(realtime);
                Ok(())
            }
            Entry::Occupied(_) => bail!(
                "connection {} already has realtime snapshot capability",
                connection_id.0
            ),
        }
    }

    pub fn bind_character(
        &mut self,
        character_id: CharacterId,
        connection_id: ConnectionId,
    ) -> anyhow::Result<()> {
        if !self.realtime_by_connection.contains_key(&connection_id) {
            bail!(
                "cannot bind character {} to connection {} without realtime capability",
                character_id.0,
                connection_id.0
            );
        }
        match self.connection_by_character.entry(character_id) {
            Entry::Vacant(entry) => {
                entry.insert(connection_id);
                Ok(())
            }
            Entry::Occupied(entry) => bail!(
                "character {} already has realtime route through connection {}",
                character_id.0,
                entry.get().0
            ),
        }
    }

    #[must_use]
    pub fn connection_id_for_character(&self, character_id: CharacterId) -> Option<ConnectionId> {
        self.connection_by_character.get(&character_id).copied()
    }

    pub fn remove_connection(&mut self, connection_id: ConnectionId) {
        self.realtime_by_connection.remove(&connection_id);
        self.connection_by_character
            .retain(|_, bound_connection| *bound_connection != connection_id);
    }

    pub fn route(&self, snapshot: MovementSnapshot) -> anyhow::Result<SnapshotRouteOutcome> {
        let Some(connection_id) = self.connection_id_for_character(snapshot.character_id()) else {
            return Ok(SnapshotRouteOutcome::DroppedNoRoute);
        };
        let Some(realtime) = self.realtime_by_connection.get(&connection_id) else {
            return Ok(SnapshotRouteOutcome::DroppedNoRoute);
        };

        let position = snapshot.position();
        let wire = SelfMovementSnapshotV2 {
            server_tick: snapshot.server_tick().0,
            x_mm: position.x(),
            y_mm: position.y(),
            z_mm: position.z(),
            last_processed_input_sequence: snapshot.last_processed_input_sequence().map(|s| s.0),
            active_movement_sequence: snapshot.active_movement_sequence().map(|s| s.0),
        };
        
        match realtime.try_send(MessageKind::SelfMovementSnapshotV2, &wire.encode_wire()) {
            Ok(()) => Ok(SnapshotRouteOutcome::Sent),
            Err(RealtimeSendError::Frame(error)) => {
                Err(error).context("failed to encode authoritative self movement snapshot v2")
            }
            Err(
                RealtimeSendError::Transport { .. }
                | RealtimeSendError::ChannelFull { .. }
                | RealtimeSendError::ChannelClosed { .. },
            ) => Ok(SnapshotRouteOutcome::DroppedRealtime),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_core::{IntentSequence, ServerTick, WorldPositionMm};

    #[test]
    fn snapshot_bridge_sends_self_scoped_authoritative_v2_payload() -> anyhow::Result<()> {
        let connection_id = ConnectionId(7);
        let character_id = CharacterId(99);
        let (realtime, mut datagrams) = RealtimeConnectionSender::bounded_channel(connection_id, 2)?;
        let mut bridge = SelfSnapshotBridge::default();
        bridge.register_connection(realtime)?;
        bridge.bind_character(character_id, connection_id)?;
        assert_eq!(
            bridge.connection_id_for_character(character_id),
            Some(connection_id)
        );

        let snapshot = MovementSnapshot::new(
            character_id,
            ServerTick(9001),
            WorldPositionMm::new(400, -125, 42),
            Some(IntentSequence(103)),
            Some(IntentSequence(103)),
        );
        assert_eq!(bridge.route(snapshot)?, SnapshotRouteOutcome::Sent);

        let datagram = datagrams.try_recv()?;
        assert_eq!(datagram.kind, MessageKind::SelfMovementSnapshotV2);
        let decoded = SelfMovementSnapshotV2::decode_wire(&datagram.payload)?;
        assert_eq!(decoded.server_tick, 9001);
        assert_eq!(decoded.x_mm, 400);
        assert_eq!(decoded.y_mm, -125);
        assert_eq!(decoded.z_mm, 42);
        assert_eq!(decoded.last_processed_input_sequence, Some(103));
        assert_eq!(decoded.active_movement_sequence, Some(103));
        Ok(())
    }

    #[test]
    fn snapshot_bridge_drops_after_connection_route_is_removed() -> anyhow::Result<()> {
        let connection_id = ConnectionId(7);
        let character_id = CharacterId(99);
        let (realtime, _datagrams) = RealtimeConnectionSender::bounded_channel(connection_id, 1)?;
        let mut bridge = SelfSnapshotBridge::default();
        bridge.register_connection(realtime)?;
        bridge.bind_character(character_id, connection_id)?;
        bridge.remove_connection(connection_id);

        assert_eq!(bridge.connection_id_for_character(character_id), None);
        let snapshot =
            MovementSnapshot::new(character_id, ServerTick(1), WorldPositionMm::ORIGIN, None, None);
        assert_eq!(bridge.route(snapshot)?, SnapshotRouteOutcome::DroppedNoRoute);
        Ok(())
    }
}
