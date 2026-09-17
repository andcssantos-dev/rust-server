use std::collections::{HashMap, hash_map::Entry};

use anyhow::{Context, bail};
use aurenfall_core::{CharacterId, WorldPositionMm, ZoneId};
use aurenfall_session::{AuthorizedIntent, IntentPayload};
use aurenfall_simulation::{MovementInput, InventoryDespawnResponder, ZoneCommand};
use aurenfall_domain::items::CharacterInventoryState;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldRouteOutcome {
    Routed,
    DroppedBackpressure,
}

#[derive(Debug, Default)]
pub struct WorldRouter {
    zones: HashMap<ZoneId, mpsc::Sender<ZoneCommand>>,
}

impl WorldRouter {
    pub fn register_zone(
        &mut self,
        zone_id: ZoneId,
        sender: mpsc::Sender<ZoneCommand>,
    ) -> anyhow::Result<()> {
        match self.zones.entry(zone_id) {
            Entry::Vacant(entry) => {
                entry.insert(sender);
                Ok(())
            }
            Entry::Occupied(_) => bail!("zone {} is already registered with world router", zone_id.0),
        }
    }

    pub fn spawn_character(
        &self,
        zone_id: ZoneId,
        character_id: CharacterId,
        position: WorldPositionMm,
        inventory: CharacterInventoryState,
        inventory_revision: u64,
    ) -> anyhow::Result<()> {
        self.route_control(
            zone_id,
            ZoneCommand::SpawnCharacter {
                character_id,
                position,
                inventory,
                inventory_revision,
            },
            "spawn character",
        )
    }

  

    pub fn despawn_character(
        &self,
        zone_id: ZoneId,
        character_id: CharacterId,
        responder: Option<InventoryDespawnResponder>,
    ) -> anyhow::Result<()> {
        self.route_control(
            zone_id,
            ZoneCommand::DespawnCharacter {
                character_id,
                responder,
            },
            "despawn character",
        )
    }

    pub fn snapshot_inventory(
        &self,
        zone_id: ZoneId,
        character_id: CharacterId,
        responder: InventoryDespawnResponder,
    ) -> anyhow::Result<()> {
        self.route_control(
            zone_id,
            ZoneCommand::SnapshotInventory {
                character_id,
                responder,
            },
            "snapshot inventory",
        )
    }

    pub fn send_zone_command(&self, zone_id: ZoneId, command: ZoneCommand) -> anyhow::Result<()> {
        let sender = self.sender_for_zone(zone_id)?;
        match sender.try_send(command) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Descarta sob contrapressao para evitar engasgo do loop de rede
                Ok(())
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                bail!("zone command queue closed for zone {}", zone_id.0)
            }
        }
    }

    pub fn route_authorized(&self, intent: AuthorizedIntent) -> anyhow::Result<WorldRouteOutcome> {
        let zone_id = intent.zone_id();
        let sender = self.sender_for_zone(zone_id)?;
        let command = authorized_to_zone_command(intent)?;

        match sender.try_send(command) {
            Ok(()) => Ok(WorldRouteOutcome::Routed),
            Err(mpsc::error::TrySendError::Full(_)) => Ok(WorldRouteOutcome::DroppedBackpressure),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                bail!("zone {} command queue is closed", zone_id.0)
            }
        }
    }

    fn route_control(
        &self,
        zone_id: ZoneId,
        command: ZoneCommand,
        action: &'static str,
    ) -> anyhow::Result<()> {
        let sender = self.sender_for_zone(zone_id)?;
        match sender.try_send(command) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                bail!("cannot {action}: zone {} command queue is full", zone_id.0)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                bail!("cannot {action}: zone {} command queue is closed", zone_id.0)
            }
        }
    }

    fn sender_for_zone(&self, zone_id: ZoneId) -> anyhow::Result<&mpsc::Sender<ZoneCommand>> {
        self.zones
            .get(&zone_id)
            .with_context(|| format!("world router has no registered zone {}", zone_id.0))
    }

    


}

fn authorized_to_zone_command(intent: AuthorizedIntent) -> anyhow::Result<ZoneCommand> {
    let IntentPayload::Move { axis_x, axis_y } = intent.payload();
    let input = MovementInput::from_axes(axis_x, axis_y)
        .with_context(|| format!("authorized movement contains invalid axes ({axis_x}, {axis_y})"))?;
    Ok(ZoneCommand::MoveIntent {
        character_id: intent.character_id(),
        sequence: intent.sequence().0,
        input,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_core::{AccountId, ConnectionId, IntentSequence, SessionId};
    use aurenfall_session::AuthoritativeSession;

    fn authorized_move(
        zone_id: ZoneId,
        sequence: IntentSequence,
        axis_x: i16,
        axis_y: i16,
    ) -> anyhow::Result<AuthorizedIntent> {
        let mut session = AuthoritativeSession::new(SessionId(11), ConnectionId(7))?;
        session.authenticate(AccountId(41))?;
        session.bind_character(CharacterId(99), zone_id)?;
        session.activate_world()?;
        Ok(session.authorize_intent(sequence, IntentPayload::Move { axis_x, axis_y })?)
    }

    #[test]
    fn lifecycle_commands_route_to_server_owned_zone() -> anyhow::Result<()> {
        let (zone_tx, mut zone_rx) = mpsc::channel(2);
        let mut router = WorldRouter::default();
        router.register_zone(ZoneId(3), zone_tx)?;

        router.spawn_character(
            ZoneId(3),
            CharacterId(99),
            WorldPositionMm::ORIGIN,
            CharacterInventoryState::new(10, 6),
            1,
        )?;
        router.despawn_character(ZoneId(3), CharacterId(99), None)?;

        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::SpawnCharacter {
                character_id: CharacterId(99),
                position: WorldPositionMm::ORIGIN,
                inventory: CharacterInventoryState::new(10, 6),
                inventory_revision: 1,
            }
        );
        assert_eq!(
            zone_rx.try_recv()?,
            ZoneCommand::DespawnCharacter {
                character_id: CharacterId(99),
                responder: None,
            }
        );
        Ok(())
    }

    #[test]
    fn authorized_move_routes_server_owned_identity_to_zone() -> anyhow::Result<()> {
        let (zone_tx, mut zone_rx) = mpsc::channel(2);
        let mut router = WorldRouter::default();
        router.register_zone(ZoneId(3), zone_tx)?;

        let outcome =
            router.route_authorized(authorized_move(ZoneId(3), IntentSequence(17), 16_384, -8_192)?)?;
        assert_eq!(outcome, WorldRouteOutcome::Routed);

        let command = zone_rx.try_recv()?;
        let input = MovementInput::from_axes(16_384, -8_192).context("test movement input must be valid")?;
        assert_eq!(
            command,
            ZoneCommand::MoveIntent {
                character_id: CharacterId(99),
                sequence: 17,
                input,
            }
        );
        Ok(())
    }

    #[test]
    fn movement_backpressure_drops_without_blocking_session_runtime() -> anyhow::Result<()> {
        let (zone_tx, _zone_rx) = mpsc::channel(1);
        let input = MovementInput::from_axes(1, 0).context("test movement input must be valid")?;
        zone_tx.try_send(ZoneCommand::MoveIntent {
            character_id: CharacterId(1),
            sequence: 1,
            input,
        })?;

        let mut router = WorldRouter::default();
        router.register_zone(ZoneId(3), zone_tx)?;
        let outcome = router.route_authorized(authorized_move(ZoneId(3), IntentSequence(2), 2, 0)?)?;

        assert_eq!(outcome, WorldRouteOutcome::DroppedBackpressure);
        Ok(())
    }

    #[test]
    fn lifecycle_backpressure_is_not_silently_dropped() -> anyhow::Result<()> {
        let (zone_tx, _zone_rx) = mpsc::channel(1);
        zone_tx.try_send(ZoneCommand::DespawnCharacter {
            character_id: CharacterId(1),
            responder: None,
        })?;

        let mut router = WorldRouter::default();
        router.register_zone(ZoneId(3), zone_tx)?;
        assert!(
            router
                .spawn_character(
                    ZoneId(3),
                    CharacterId(99),
                    WorldPositionMm::ORIGIN,
                    CharacterInventoryState::new(10, 6),
                    1,
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn unknown_authoritative_zone_is_a_consistency_error() -> anyhow::Result<()> {
        let router = WorldRouter::default();
        let result = router.route_authorized(authorized_move(ZoneId(99), IntentSequence(1), 1, 0)?);

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn invalid_internal_movement_axes_do_not_reach_zone() -> anyhow::Result<()> {
        let (zone_tx, mut zone_rx) = mpsc::channel(2);
        let mut router = WorldRouter::default();
        router.register_zone(ZoneId(3), zone_tx)?;

        let result = router.route_authorized(authorized_move(ZoneId(3), IntentSequence(1), i16::MIN, 0)?);

        assert!(result.is_err());
        assert!(zone_rx.try_recv().is_err());
        Ok(())
    }
}