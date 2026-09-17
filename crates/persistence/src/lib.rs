use anyhow::Context;
use tokio_postgres::{Client, NoTls};
pub mod items;
pub use items::*;
pub mod inventory;
pub use inventory::InventoryRepository;
pub mod position_buffer;
pub mod character;
pub use character::CharacterRepository;

#[derive(Debug, Clone)]
pub struct PostgresSettings {
    pub connection_string: String,
}

pub async fn connect(settings: &PostgresSettings) -> anyhow::Result<Client> {
    let (client, connection) = tokio_postgres::connect(&settings.connection_string, NoTls)
        .await
        .context("PostgreSQL connection failed")?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::error!(%error, "PostgreSQL connection task failed");
        }
    });

    Ok(client)
}

/// Aplica as migrações estruturais do PostgreSQL embutidas no binário.
pub async fn run_migrations(client: &Client) -> anyhow::Result<()> {
    // Migração 2: Inventários dos personagens
    let migration_inventories = include_str!("../migrations/0002_character_inventories.sql");
    client
        .batch_execute(migration_inventories)
        .await
        .context("falha ao executar migration de character_inventories")?;

    // Migração 3: Atributos e tabela de personagens
    let migration_attributes = include_str!("../migrations/0003_character_attributes.sql");
    client
        .batch_execute(migration_attributes)
        .await
        .context("falha ao executar migration de character_attributes")?;

    Ok(())
}