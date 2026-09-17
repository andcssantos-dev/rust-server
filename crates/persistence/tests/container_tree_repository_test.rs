//! Teste de integração do repositório da árvore de containers com PostgreSQL.
//! 
//! Valida o ciclo completo de persistência:
//! 1. Gravação inicial (INSERT) com revisão 1.
//! 2. Leitura (SELECT) e reconstrução idêntica da árvore JSON.
//! 3. Proteção contra sobrescrita concorrente (Controle Otimista de Concorrência).

use aurenfall_core::CharacterId;
use aurenfall_domain::containers::{
    CharacterContainerTree, ContainerAffix, ContainerId, ContainerInstance, ItemCategory, StoredItem,
};
use aurenfall_persistence::inventory::ContainerTreeRepository;
use aurenfall_persistence::{connect, run_migrations, PostgresSettings};

/// Cria as configurações de conexão apontando para o PostgreSQL local de desenvolvimento.
fn test_postgres_settings() -> PostgresSettings {
    let connection_string = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "host=localhost port=5433 user=postgres password=postgres dbname=aurenfall_dev".to_string()
    });
    PostgresSettings { connection_string }
}

#[tokio::test]
async fn test_container_tree_persistence_lifecycle() -> anyhow::Result<()> {
    // 1. Conecta ao PostgreSQL local
    let settings = test_postgres_settings();
    let client = match connect(&settings).await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Aviso: PostgreSQL não disponível para o teste ({err}). Ignorando teste de banco.");
            return Ok(());
        }
    };

    // Aplica as migrações para assegurar que a tabela character_inventories existe
    run_migrations(&client).await?;

    // Identificador exclusivo para o teste não interferir em outros dados
    let test_char_id = CharacterId(888_777_666);

    // Limpeza preventiva de execuções anteriores
    let _ = client
        .execute(
            "DELETE FROM character_inventories WHERE character_id = $1;",
            &[&(test_char_id.0 as i64)],
        )
        .await;

    // 2. Monta uma árvore de mochilas em memória com itens e afixo mágico
    let mut original_tree = CharacterContainerTree::new();

    // Cria os bolsos do personagem (Container Raiz)
    let root = ContainerInstance::new_grid(ContainerId(1), 10, 6, 0);
    original_tree.set_root(root);

    // Cria uma mochila mágica com afixo de redução de peso de 25%
    let mut magic_bag = ContainerInstance::new_grid(ContainerId(2), 6, 6, 300);
    magic_bag.add_affix(ContainerAffix::WeightReduction(25));
    original_tree.add_container(magic_bag);
    original_tree.attach_to_parent(ContainerId(2), ContainerId(1), 2).unwrap();

    // Insere uma pilha de 15 pedras dentro da mochila mágica
    let stones = StoredItem::new(10, 500, ItemCategory::Ore, 200, 15, 99, 1, 1);
    original_tree.quick_insert(ContainerId(2), stones, false).unwrap();

    // =====================================================================
    // TESTE 1: Gravação inicial (esperando revisão 0 -> deve resultar na revisão 1)
    // =====================================================================
    let saved_rev = ContainerTreeRepository::save_tree(
        &client,
        test_char_id,
        0,
        &original_tree,
    )
    .await
    .expect("Deveria salvar a árvore inicial com sucesso");

    assert_eq!(saved_rev, 1, "A primeira revisão salva deve ser 1");

    // =====================================================================
    // TESTE 2: Leitura da base de dados e validação dos dados reconstruídos
    // =====================================================================
    let loaded = ContainerTreeRepository::load_tree(&client, test_char_id)
        .await
        .expect("Deveria carregar o inventário sem erros")
        .expect("O inventário do personagem deveria existir na base de dados");

    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.tree.root_id, Some(ContainerId(1)));
    assert_eq!(loaded.tree.containers.len(), 2, "Deveriam existir 2 containers salvos");

    // Valida se os itens guardados dentro da mochila mágica continuam íntegros
    let loaded_magic_bag = loaded.tree.containers.get(&ContainerId(2)).unwrap();
    assert_eq!(loaded_magic_bag.items.len(), 1);
    assert_eq!(loaded_magic_bag.items[0].quantity, 15);
    assert_eq!(loaded_magic_bag.items[0].definition_id, 500);

    // =====================================================================
    // TESTE 3: Proteção contra Conflito de Concorrência (OCC)
    // =====================================================================
    // Tentamos salvar passando a revisão 0, mas a base de dados já avançou para a revisão 1
    let conflict_result = ContainerTreeRepository::save_tree(
        &client,
        test_char_id,
        0, // Versão desatualizada propositadamente
        &original_tree,
    )
    .await;

    assert!(
        conflict_result.is_err(),
        "A base de dados deve recusar gravações com versão desatualizada!"
    );

    // Limpeza após validação bem-sucedida
    let _ = client
        .execute(
            "DELETE FROM character_inventories WHERE character_id = $1;",
            &[&(test_char_id.0 as i64)],
        )
        .await;

    Ok(())
}