use aurenfall_core::{CharacterId, DefinitionId, ItemInstanceId};
use aurenfall_domain::items::{CharacterInventoryState, ItemInstance};
use aurenfall_domain::Ownership;
use aurenfall_persistence::{connect, run_migrations, InventoryRepository, PostgresSettings};

fn test_postgres_settings() -> PostgresSettings {
    let connection_string = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "host=localhost user=postgres password=postgres dbname=aurenfall_test".to_string()
    });
    PostgresSettings { connection_string }
}

#[tokio::test]
async fn roundtrip_inventory_persistence_and_concurrency() -> anyhow::Result<()> {
    let settings = test_postgres_settings();
    let client = match connect(&settings).await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Ignorando teste de banco (sem conexao PostgreSQL local): {err}");
            return Ok(());
        }
    };

    run_migrations(&client).await?;

    let character_id = CharacterId(9999);
    let mut initial_state = CharacterInventoryState::new(10, 6);

    // Cria um item empilhável usando o construtor oficial do domain
    let test_item = ItemInstance::new_stackable(
        ItemInstanceId(101),
        DefinitionId(500),
        Ownership::Character(character_id),
        5,
    );
    initial_state.items.insert(test_item.id, test_item);

    // 1. Salva a revisão inicial (esperada: 0 -> nova linha criada com rev 1)
    let rev_1 = InventoryRepository::save_inventory(&client, character_id, &initial_state, 0).await?;
    assert_eq!(rev_1, 1);

    // 2. Carrega e valida roundtrip
    let loaded = InventoryRepository::load_inventory(&client, character_id).await?;
    let (loaded_state, loaded_rev) = loaded.expect("inventario deve existir no banco");
    assert_eq!(loaded_rev, 1);
    assert_eq!(loaded_state.items.len(), 1);
    assert_eq!(loaded_state.items.get(&ItemInstanceId(101)).map(|i| i.quantity), Some(5));

    // 3. Atualiza estado para revisao 2
    let rev_2 = InventoryRepository::save_inventory(&client, character_id, &loaded_state, 1).await?;
    assert_eq!(rev_2, 2);

    // 4. Conflito de concorrência: tentar salvar com revisão defasada (esperando 1 quando já está em 2)
    let stale_save = InventoryRepository::save_inventory(&client, character_id, &loaded_state, 1).await;
    assert!(stale_save.is_err(), "deve rejeitar save com versao defasada");

    Ok(())
}