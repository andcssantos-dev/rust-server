use aurenfall_domain::items::DroppedBagManager;
use aurenfall_persistence::items::WorldBagsStateSnapshot;

#[derive(Debug)]
pub struct WorldBootstrapService;

impl WorldBootstrapService {
    /// Restaura o estado das mochilas do mundo a partir de um snapshot persistido.
    /// Descarta entidades cujo TTL expirou durante o downtime e recalcula os ticks restantes.
    pub fn restore_world_bags(
        manager: &mut DroppedBagManager,
        snapshot: WorldBagsStateSnapshot,
        server_boot_tick: u64,
    ) -> usize {
        let mut restored_count = 0;
        let elapsed_ticks_offline = server_boot_tick.saturating_sub(snapshot.saved_at_tick);

        for mut bag in snapshot.bags {
            // Tempo de vida já consumido antes do snapshot + tempo offline
            let total_elapsed = (snapshot.saved_at_tick.saturating_sub(bag.spawned_at_tick))
                .saturating_add(elapsed_ticks_offline);

            if total_elapsed < bag.decay_after_ticks {
                // Ajusta a referência temporal para o relógio do servidor recém-iniciado
                let remaining_ticks = bag.decay_after_ticks.saturating_sub(total_elapsed);
                bag.spawned_at_tick = server_boot_tick;
                bag.decay_after_ticks = remaining_ticks;

                manager.spawn_bag(bag);
                restored_count += 1;
            }
        }

        restored_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurenfall_core::CharacterId;
    use aurenfall_domain::items::{DroppedBagEntity, DroppedBagId};

    #[test]
    fn test_restore_world_bags_filtering_and_time_adjustment() {
        let mut manager = DroppedBagManager::new();

        // Mochila 1: Spawnou no tick 100, TTL 100 (expira em 200). Snapshot salvo em 150.
        let bag_valid = DroppedBagEntity::new(
            DroppedBagId(1),
            CharacterId(10),
            (0, 0),
            (10.0, 10.0),
            100,
            100,
            Vec::new(),
        );

        // Mochila 2: Spawnou no tick 100, TTL 40 (expiraria em 140).
        let bag_expired = DroppedBagEntity::new(
            DroppedBagId(2),
            CharacterId(20),
            (0, 0),
            (15.0, 15.0),
            100,
            40,
            Vec::new(),
        );

        let snapshot = WorldBagsStateSnapshot {
            saved_at_tick: 150,
            bags: vec![bag_valid, bag_expired],
        };

        // Servidor religou no tick 170 (20 ticks após o snapshot)
        let restored = WorldBootstrapService::restore_world_bags(&mut manager, snapshot, 170);

        assert_eq!(restored, 1);
        let active_bag = manager.get_bag(DroppedBagId(1)).expect("Deveria existir");

        // Tempo decorrido: (150 - 100) + (170 - 150) = 70 ticks.
        // TTL restante: 100 - 70 = 30 ticks a partir do boot (170).
        assert_eq!(active_bag.spawned_at_tick, 170);
        assert_eq!(active_bag.decay_after_ticks, 30);
        assert!(manager.get_bag(DroppedBagId(2)).is_none());
    }
}