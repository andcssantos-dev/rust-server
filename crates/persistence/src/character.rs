use anyhow::{Context, Result};
use aurenfall_core::{CharacterId, WorldPositionMm};
use tokio_postgres::Client;

#[derive(Debug, Clone)]
pub struct CharacterEntity {
    pub character_id: i64,
    pub name: String,
    pub level: i32,
    pub strength: i32,
    pub defense: i32,
    pub agility: i32,
    pub vitality: i32,
    pub intelligence: i32,
}

#[derive(Debug)]
pub struct CharacterRepository;

impl CharacterRepository {
    /// Carrega a última posição persistida do personagem no banco de dados.
    pub async fn load_position(
        client: &Client,
        character_id: CharacterId,
    ) -> Result<Option<WorldPositionMm>> {
        let row_opt = client
            .query_opt(
                "SELECT pos_x_mm, pos_y_mm, pos_z_mm FROM characters WHERE character_id = $1",
                &[&(character_id.0 as i64)],
            )
            .await
            .context("falha ao consultar posição persistida do personagem")?;

        if let Some(row) = row_opt {
            let x: i64 = row.get(0);
            let y: i64 = row.get(1);
            let z: i64 = row.get(2);
            Ok(Some(WorldPositionMm::new(x, y, z)))
        } else {
            Ok(None)
        }
    }

    /// Salva um novo personagem recém-criado com os atributos iniciais distribuídos.
    pub async fn create_character(
        client: &Client,
        account_id: i64,
        name: &str,
        str_val: i32,
        def_val: i32,
        agi_val: i32,
        vit_val: i32,
        int_val: i32,
    ) -> Result<i64> {
        let row = client
            .query_one(
                r#"
                INSERT INTO characters 
                (account_id, name, strength, defense, agility, vitality, intelligence)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING character_id
                "#,
                &[
                    &account_id,
                    &name,
                    &str_val,
                    &def_val,
                    &agi_val,
                    &vit_val,
                    &int_val,
                ],
            )
            .await
            .context("falha ao inserir novo personagem")?;

        let id: i64 = row.get(0);
        Ok(id)
    }

    /// Atualiza os atributos básicos e pontos disponíveis após o ganho de nível.
    pub async fn update_attributes(
        client: &Client,
        character_id: i64,
        str_val: i32,
        def_val: i32,
        agi_val: i32,
        vit_val: i32,
        int_val: i32,
        available_points: i32,
    ) -> Result<()> {
        client
            .execute(
                r#"
                UPDATE characters 
                SET strength = $1, defense = $2, agility = $3, vitality = $4, intelligence = $5, 
                    available_attribute_points = $6, updated_at = NOW()
                WHERE character_id = $7
                "#,
                &[
                    &str_val,
                    &def_val,
                    &agi_val,
                    &vit_val,
                    &int_val,
                    &available_points,
                    &character_id,
                ],
            )
            .await
            .context("falha ao atualizar atributos do personagem")?;

        Ok(())
    }
}