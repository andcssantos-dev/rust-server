use anyhow::{Context, Result};
use aurenfall_core::CharacterId;
use aurenfall_domain::containers::CharacterContainerTree;
use aurenfall_domain::items::CharacterInventoryState;
use tokio_postgres::Client;

// =========================================================================
// REPOSITÓRIO LEGADO (Inventário Plano 10x6)
// Mantido para compatibilidade com os testes e simulações já existentes.
// =========================================================================

#[derive(Debug)]
pub struct InventoryRepository;

impl InventoryRepository {
    /// Carrega o inventário plano persistido do personagem.
    /// Retorna `None` se o jogador ainda não possuir registro no banco.
    pub async fn load_inventory(
        client: &Client,
        character_id: CharacterId,
    ) -> Result<Option<(CharacterInventoryState, u64)>> {
        let stmt = "SELECT inventory_data, revision FROM character_inventories WHERE character_id = $1";
        let row = client
            .query_opt(stmt, &[&(character_id.0 as i64)])
            .await
            .context("falha ao consultar character_inventories no PostgreSQL")?;

        let Some(row) = row else {
            return Ok(None);
        };

        let raw_bytes: Vec<u8> = row.get(0);
        let revision: i64 = row.get(1);

        let state: CharacterInventoryState = bincode::deserialize(&raw_bytes)
            .context("falha ao desserializar CharacterInventoryState do banco")?;

        Ok(Some((state, revision as u64)))
    }

    /// Salva o estado do inventário plano com controle de concorrência otimista (UPSERT).
    pub async fn save_inventory(
        client: &Client,
        character_id: CharacterId,
        state: &CharacterInventoryState,
        expected_revision: u64,
    ) -> Result<u64> {
        let raw_bytes = bincode::serialize(state)
            .context("falha ao serializar CharacterInventoryState para bincode")?;

        let char_id_i64 = character_id.0 as i64;
        let expected_rev_i64 = expected_revision as i64;

        let query = r#"
            INSERT INTO character_inventories (character_id, inventory_data, revision, updated_at)
            VALUES ($1, $2, 1, NOW())
            ON CONFLICT (character_id) DO UPDATE
            SET inventory_data = EXCLUDED.inventory_data,
                revision = character_inventories.revision + 1,
                updated_at = NOW()
            WHERE character_inventories.revision = $3
            RETURNING revision;
        "#;

        let row = client
            .query_opt(query, &[&char_id_i64, &raw_bytes, &expected_rev_i64])
            .await
            .context("falha ao persistir inventário no PostgreSQL")?;

        let Some(row) = row else {
            anyhow::bail!(
                "conflito de concorrência ao salvar inventário do personagem {}",
                character_id.0
            );
        };

        let updated_rev: i64 = row.get(0);
        Ok(updated_rev as u64)
    }
}

// =========================================================================
// NOVO REPOSITÓRIO: Árvore de Containers e Mochilas Aninhadas
// Suporta mochilas, afixos mágicos, rotação 2D e agrupamento de itens.
// =========================================================================

/// Representa a árvore de mochilas lida do banco junto com a sua versão OCC.
#[derive(Debug, Clone)]
pub struct PersistedContainerTree {
    pub revision: u64,
    pub tree: CharacterContainerTree,
}

/// Repositório autoritativo para carregar e salvar árvores de containers no banco.
#[derive(Debug)]
pub struct ContainerTreeRepository;

impl ContainerTreeRepository {
    /// Carrega a árvore de containers do personagem a partir do PostgreSQL.
    /// Retorna `Ok(Some(PersistedContainerTree))` se existir, ou `Ok(None)` se for novo.
    pub async fn load_tree(
        client: &Client,
        character_id: CharacterId,
    ) -> Result<Option<PersistedContainerTree>> {
        let stmt = "
            SELECT inventory_data, revision 
            FROM character_inventories 
            WHERE character_id = $1;
        ";

        let char_id_raw = character_id.0 as i64;
        let row_opt = client
            .query_opt(stmt, &[&char_id_raw])
            .await
            .context("falha ao consultar containers do personagem no banco")?;

        match row_opt {
            Some(row) => {
                let raw_bytes: Vec<u8> = row.get(0);
                let revision_raw: i64 = row.get(1);

                // Reconverte os bytes em formato JSON para a nossa árvore CharacterContainerTree
                let tree: CharacterContainerTree = serde_json::from_slice(&raw_bytes)
                    .context("falha ao desserializar a árvore de containers do formato JSON")?;

                Ok(Some(PersistedContainerTree {
                    revision: revision_raw as u64,
                    tree,
                }))
            }
            None => Ok(None),
        }
    }

    /// Salva a árvore de containers no PostgreSQL usando Controle Otimista de Concorrência (OCC).
    /// Incrementa a revisão atômica e retorna a nova versão persistida.
    pub async fn save_tree(
        client: &Client,
        character_id: CharacterId,
        expected_revision: u64,
        tree: &CharacterContainerTree,
    ) -> Result<u64> {
        // Serializa a árvore inteira para uma sequência de bytes no padrão JSON
        let json_bytes = serde_json::to_vec(tree)
            .context("falha ao serializar CharacterContainerTree para JSON")?;

        let char_id_raw = character_id.0 as i64;
        let expected_rev_raw = expected_revision as i64;

        let query = r#"
            INSERT INTO character_inventories (character_id, inventory_data, revision, updated_at)
            VALUES ($1, $2, 1, NOW())
            ON CONFLICT (character_id) DO UPDATE
            SET inventory_data = EXCLUDED.inventory_data,
                revision = character_inventories.revision + 1,
                updated_at = NOW()
            WHERE character_inventories.revision = $3
            RETURNING revision;
        "#;

        let row_opt = client
            .query_opt(query, &[&char_id_raw, &json_bytes, &expected_rev_raw])
            .await
            .context("falha ao persistir árvore de containers no PostgreSQL")?;

        let Some(row) = row_opt else {
            anyhow::bail!(
                "conflito de concorrência ao salvar containers do personagem {}: revisão esperada {}",
                character_id.0,
                expected_revision
            );
        };

        let new_revision: i64 = row.get(0);
        Ok(new_revision as u64)
    }
}