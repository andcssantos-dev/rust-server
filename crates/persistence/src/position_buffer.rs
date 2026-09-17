use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tokio_postgres::Client;
use dashmap::DashMap;
use anyhow::Result;

#[derive(Clone, Debug)]
pub struct CharacterPositionUpdate {
    pub character_id: i64,
    pub x_mm: i64,
    pub y_mm: i64,
    pub z_mm: i64,
    pub quadrant_x: i64,
    pub quadrant_y: i64,
}

pub struct PositionPersistenceBuffer {
    client: Arc<Client>,
    buffer: Arc<DashMap<i64, CharacterPositionUpdate>>,
}

// Implementação manual de Debug para evitar o aviso de missing-debug-implementations
impl std::fmt::Debug for PositionPersistenceBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PositionPersistenceBuffer")
            .field("buffer_size", &self.buffer.len())
            .finish()
    }
}

impl PositionPersistenceBuffer {
    pub fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            buffer: Arc::new(DashMap::new()),
        }
    }

    /// Atualiza o cache em memória instantaneamente a cada tick do servidor
    pub fn track_position(&self, update: CharacterPositionUpdate) {
        self.buffer.insert(update.character_id, update);
    }

    /// Inicia a tarefa em segundo plano que descarrega o buffer no PostgreSQL periodicamente
    pub fn start_background_flusher(self: Arc<Self>, flush_interval: Duration) {
        tokio::spawn(async move {
            let mut ticker = interval(flush_interval);
            loop {
                ticker.tick().await;
                if let Err(e) = self.flush_to_db().await {
                    tracing::error!(error = ?e, "[PERSISTENCE] Erro ao descarregar buffer de posições em lote");
                }
            }
        });
    }

    /// Executa as atualizações em lote diretamente na base de dados
    async fn flush_to_db(&self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let updates: Vec<CharacterPositionUpdate> = self.buffer
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        self.buffer.clear();

        for update in updates {
            let rows_affected = self.client.execute(
                r#"
                UPDATE characters 
                SET pos_x_mm = $1, pos_y_mm = $2, pos_z_mm = $3,
                    current_quadrant_x = $4, current_quadrant_y = $5,
                    updated_at = NOW()
                WHERE character_id = $6
                "#,
                &[
                    &update.x_mm,
                    &update.y_mm,
                    &update.z_mm,
                    &update.quadrant_x,
                    &update.quadrant_y,
                    &update.character_id,
                ],
            )
            .await?;

            tracing::info!(
                character_id = update.character_id,
                x_mm = update.x_mm,
                y_mm = update.y_mm,
                z_mm = update.z_mm,
                rows_affected,
                "[PERSISTENCE] Posição do personagem atualizada no PostgreSQL"
            );
        }

        Ok(())
    }
}