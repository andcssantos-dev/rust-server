use aurenfall_contracts::{MineIntent, MoveIntent};
use aurenfall_core::CharacterId;
use aurenfall_simulation::movement::MovementInput;
use aurenfall_simulation::ZoneCommand;
use tokio::sync::mpsc;
use tracing::warn;

pub async fn dispatch_move_intent(
    character_id: CharacterId,
    intent: MoveIntent,
    zone_tx: &mpsc::Sender<ZoneCommand>,
) {
    let input = MovementInput::from_axes(intent.axis_x, intent.axis_y)
        .unwrap_or(MovementInput::ZERO);

    let command = ZoneCommand::MoveIntent {
        character_id,
        sequence: intent.sequence,
        input,
    };

    if let Err(e) = zone_tx.send(command).await {
        warn!(%e, character_id = character_id.0, "Falha ao encaminhar MoveIntent");
    }
}

pub async fn dispatch_mine_intent(
    character_id: CharacterId,
    _intent: MineIntent,
    zone_tx: &mpsc::Sender<ZoneCommand>,
) {
    let command = ZoneCommand::MineIntent {
        character_id,
        power: 1,
    };

    if let Err(e) = zone_tx.send(command).await {
        warn!(%e, character_id = character_id.0, "Falha ao encaminhar MineIntent");
    }
}