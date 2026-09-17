use anyhow::Context;
use aurenfall_core::WorldPositionMm;
use crate::components::{
    Dirty, Identity, MineIntent, MoveIntent, Position, Remainder, RockNode, SpatialUpdate,
};
use crate::movement::MovementRules;
use crate::traversal::TraversalWorld;

/// Evento disparado quando uma pedra é completamente minerada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RockDestroyedEvent {
    pub resource_id: u32,
    pub position: WorldPositionMm,
    pub b_active: bool,
}

/// Ponto de entrada para o tick de simulação do mundo.
pub(crate) fn simulation_tick(
    world: &mut hecs::World,
    traversal: &TraversalWorld,
    rules: &MovementRules,
) -> anyhow::Result<(Vec<SpatialUpdate>, Vec<RockDestroyedEvent>)> {
    movement_tick_system(world, traversal, rules)?;
    let destroyed_rocks = mining_tick_system(world, rules.character_radius_mm())?;

    let mut updates = Vec::new();
    let mut to_clean = Vec::new();

    for (entity, (identity, position, _dirty)) in world.query_mut::<(&Identity, &Position, &Dirty)>() {
        updates.push(SpatialUpdate {
            identity: *identity,
            position: *position,
        });
        to_clean.push(entity);
    }

    for entity in to_clean {
        let _ = world.remove_one::<Dirty>(entity);
    }

    Ok((updates, destroyed_rocks))
}

pub(crate) fn movement_tick_system(
    world: &mut hecs::World,
    traversal: &TraversalWorld,
    rules: &MovementRules,
) -> anyhow::Result<()> {
    let mut modified = Vec::new();

    for (entity, (pos, intent, remainder)) in world.query_mut::<(&mut Position, &MoveIntent, Option<&mut Remainder>)>() {
        let mut temp_remainder = remainder.as_deref().map(|r| r.0).unwrap_or_default();

        let delta = rules
            .integrate(intent.0, &mut temp_remainder)
            .context("falha ao integrar intenção de movimento")?;

        if let Some(r) = remainder {
            r.0 = temp_remainder;
        }

        if delta.x != 0 || delta.y != 0 {
            let resolution = traversal
                .resolve_horizontal(pos.0, delta, rules.character_radius_mm())
                .context("falha ao resolver travessia horizontal")?;

            if resolution.position != pos.0 {
                pos.0 = resolution.position;
                modified.push(entity);
            }
        }
    }

    for entity in modified {
        let _ = world.insert_one(entity, Dirty);
    }

    Ok(())
}

/// Processa intenções de mineração (tecla E).
pub(crate) fn mining_tick_system(
    world: &mut hecs::World,
    reach_distance_mm: i64,
) -> anyhow::Result<Vec<RockDestroyedEvent>> {
    let max_reach_sq = i128::from(reach_distance_mm.saturating_add(1_500)).pow(2);

    // 1. Coleta intenções de mineradores
    let mut intents = Vec::new();
    for (entity, (pos, intent)) in world.query_mut::<(&Position, &MineIntent)>() {
        intents.push((entity, pos.0, intent.power));
    }

    if intents.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Coleta nós de pedra existentes (incluindo o ID da entidade convertido de forma segura)
    let mut rocks = Vec::new();
    for (entity, (pos, rock)) in world.query_mut::<(&Position, &RockNode)>() {
        // Usamos .get() no NonZeroU64 para extrair o valor primitivo u64 e converter para u32
        let resource_id = entity.to_bits().get() as u32;
        rocks.push((entity, resource_id, pos.0, rock.health));
    }

    let mut destroyed = Vec::new();
    let mut rocks_to_despawn = Vec::new();
    let mut damages: Vec<(hecs::Entity, u32)> = Vec::new();

    // 3. Avalia distâncias e danos sem borrows ativos no World
    for (_miner_entity, miner_pos, power) in &intents {
        let mut closest_rock: Option<(hecs::Entity, u32, WorldPositionMm, i128, u32)> = None;

        for (rock_entity, resource_id, rock_pos, health) in &rocks {
            let dx = i128::from(rock_pos.x() - miner_pos.x());
            let dy = i128::from(rock_pos.y() - miner_pos.y());
            let dist_sq = dx * dx + dy * dy;

            if dist_sq <= max_reach_sq {
                match closest_rock {
                    Some((_, _, _, best_dist, _)) if dist_sq < best_dist => {
                        closest_rock = Some((*rock_entity, *resource_id, *rock_pos, dist_sq, *health));
                    }
                    None => {
                        closest_rock = Some((*rock_entity, *resource_id, *rock_pos, dist_sq, *health));
                    }
                    _ => {}
                }
            }
        }

        if let Some((target_entity, resource_id, target_pos, _, current_health)) = closest_rock {
            damages.push((target_entity, *power));
            if current_health <= *power {
                destroyed.push(RockDestroyedEvent { 
                    resource_id, 
                    position: target_pos,
                    b_active: false,
                });
                rocks_to_despawn.push(target_entity);
            }
        }
    }

    // 4. Aplica o dano no componente
    for (target_entity, damage) in damages {
        if let Ok(mut rock) = world.get::<&mut RockNode>(target_entity) {
            rock.health = rock.health.saturating_sub(damage);
        }
    }

    // 5. Limpa intenções (ações pontuais de tecla E)
    for (miner_entity, _, _) in intents {
        let _ = world.remove_one::<MineIntent>(miner_entity);
    }

    // 6. Remove pedras completamente destruídas
    for rock in rocks_to_despawn {
        let _ = world.despawn(rock);
    }

    Ok(destroyed)
}