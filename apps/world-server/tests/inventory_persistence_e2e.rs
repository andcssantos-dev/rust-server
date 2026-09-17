use std::time::Duration;
use tokio::sync::mpsc;

use aurenfall_core::{CharacterId, WorldPositionMm, ZoneId};
use aurenfall_domain::items::CharacterInventoryState;
use aurenfall_persistence::{InventoryRepository, PostgresSettings, connect, run_migrations};
use aurenfall_simulation::{
    CharacterMovementSettings, InventoryDespawnResponder, TraversalWorld, ZoneCommand, ZoneRuntime,
};

#[tokio::test]
#[ignore]
async fn test_full_inventory_save_load_lifecycle() -> anyhow::Result<()> {
    let connection_string = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/aurenfall_dev".to_string());

    let settings = PostgresSettings { connection_string };
    let client = connect(&settings).await?;
    run_migrations(&client).await?;

    let character_id = CharacterId(9999);

    // Garante que o personagem de teste comece limpo
    client
        .execute(
            "DELETE FROM character_inventories WHERE character_id = $1",
            &[&(character_id.0 as i64)],
        )
        .await?;

    let initial_state = CharacterInventoryState::new(10, 6);
    let initial_rev = 0;

    let (zone_tx, zone_rx) = mpsc::channel(32);
    let movement_settings = CharacterMovementSettings::new(4_000, 5, 250)?;
    let zone = ZoneRuntime::new(
        ZoneId(1),
        20,
        movement_settings,
        TraversalWorld::default(),
        zone_rx,
    )?;

    // 1. Spawna na Zona com inventário inicial
    zone_tx
        .send(ZoneCommand::SpawnCharacter {
            character_id,
            position: WorldPositionMm::ORIGIN,
            inventory: initial_state,
            inventory_revision: initial_rev,
        })
        .await?;

    // 2. Prepara canal de captura e despawna
    let (snap_tx, snap_rx) = tokio::sync::oneshot::channel();
    let responder = InventoryDespawnResponder::new(snap_tx);

    zone_tx
        .send(ZoneCommand::DespawnCharacter {
            character_id,
            responder: Some(responder),
        })
        .await?;

    // Executa a zona em background para processar os comandos
    let zone_handle = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_millis(150), zone.run()).await;
    });

    let (saved_state, saved_rev) = snap_rx.await?;

    // 3. Salva no banco via InventoryRepository
    let new_rev =
        InventoryRepository::save_inventory(&client, character_id, &saved_state, saved_rev).await?;
    assert!(new_rev > saved_rev);

    // 4. Carrega do banco e valida se foi persistido corretamente
    let loaded = InventoryRepository::load_inventory(&client, character_id).await?;
    let (loaded_state, loaded_rev) = loaded.expect("inventario persistido deve existir");

    assert_eq!(loaded_rev, new_rev);
    assert_eq!(loaded_state.grid.columns, saved_state.grid.columns);
    assert_eq!(loaded_state.grid.rows, saved_state.grid.rows);

    let _ = zone_handle.await;
    Ok(())
}