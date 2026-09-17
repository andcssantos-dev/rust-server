use anyhow::{Context, bail};
use aurenfall_core::{CharacterId, IntentSequence, ServerTick, WorldPositionMm};

use crate::encumbrance::{CharacterContainersComponent, MovementEncumbranceEffect};
use crate::{MovementDeltaMm, MovementInput, MovementRemainder, MovementRules, TraversalResolution};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CharacterStepOutcome {
    pub(crate) traversal: TraversalResolution,
    pub(crate) expired_sequence: Option<IntentSequence>,
}

/// Estado autoritativo do personagem na simulação ECS.
#[derive(Debug, Clone)]
pub(crate) struct CharacterState {
    character_id: CharacterId,
    position: WorldPositionMm,
    movement_input: MovementInput,
    last_input_sequence: Option<IntentSequence>,
    last_input_tick: Option<ServerTick>,
    movement_remainder: MovementRemainder,
    last_simulated_tick: ServerTick,
    /// Árvore viva de recipientes, mochilas e controle de peso corporal
    containers: CharacterContainersComponent,
}

impl CharacterState {
    /// Inicializa e posiciona um novo personagem na simulação com o inventário padrão.
    pub(crate) fn spawn(
        character_id: CharacterId,
        position: WorldPositionMm,
        spawn_tick: ServerTick,
    ) -> anyhow::Result<Self> {
        if character_id.0 == 0 {
            bail!("character_id must be server-assigned and non-zero");
        }
        Ok(Self {
            character_id,
            position,
            movement_input: MovementInput::ZERO,
            last_input_sequence: None,
            last_input_tick: None,
            movement_remainder: MovementRemainder::default(),
            last_simulated_tick: spawn_tick,
            // Inicializa com os bolsos básicos (10x6) e 30 kg de capacidade máxima
            containers: CharacterContainersComponent::default_starting(30_000),
        })
    }

    #[must_use]
    pub(crate) const fn character_id(&self) -> CharacterId {
        self.character_id
    }

    #[must_use]
    pub(crate) const fn position(&self) -> WorldPositionMm {
        self.position
    }

    /// Retorna uma referência imutável para consultar os containers e o peso do personagem.
    #[must_use]
    pub(crate) fn containers(&self) -> &CharacterContainersComponent {
        &self.containers
    }

    /// Retorna uma referência mutável para adicionar ou remover itens das mochilas.
    pub(crate) fn containers_mut(&mut self) -> &mut CharacterContainersComponent {
        &mut self.containers
    }

    /// Retorna os efeitos de sobrecarga atuais (penalidade de velocidade, corrida e esquiva).
    #[must_use]
    pub(crate) fn encumbrance_effects(&self) -> MovementEncumbranceEffect {
        self.containers.evaluate_movement_effects()
    }

    #[must_use]
    pub(crate) const fn active_movement_sequence(&self) -> Option<IntentSequence> {
        if self.movement_input.is_zero() {
            None
        } else {
            self.last_input_sequence
        }
    }

    pub(crate) fn apply_movement_input(
        &mut self,
        sequence: IntentSequence,
        input: MovementInput,
        received_tick: ServerTick,
    ) -> anyhow::Result<()> {
        if let Some(previous) = self.last_input_sequence
            && sequence.0 <= previous.0
        {
            bail!(
                "character {} received non-monotonic movement sequence {} after {}",
                self.character_id.0,
                sequence.0,
                previous.0
            );
        }
        if received_tick.0 < self.last_simulated_tick.0 {
            bail!(
                "character {} received movement input at stale tick {} behind simulated tick {}",
                self.character_id.0,
                received_tick.0,
                self.last_simulated_tick.0
            );
        }

        self.last_input_sequence = Some(sequence);
        self.last_input_tick = Some(received_tick);
        self.movement_input = input;
        if input.is_zero() {
            self.movement_remainder = MovementRemainder::default();
        }
        Ok(())
    }

    pub(crate) fn simulate_tick<F>(
        &mut self,
        tick: ServerTick,
        rules: MovementRules,
        resolve_traversal: F,
    ) -> anyhow::Result<CharacterStepOutcome>
    where
        F: FnOnce(WorldPositionMm, MovementDeltaMm) -> anyhow::Result<TraversalResolution>,
    {
        if tick.0 <= self.last_simulated_tick.0 {
            bail!(
                "character {} cannot simulate tick {} after tick {}",
                self.character_id.0,
                tick.0,
                self.last_simulated_tick.0
            );
        }

        let expired_sequence = self.expire_stale_input(tick, rules)?;
        let desired_delta = rules.integrate(self.movement_input, &mut self.movement_remainder)?;

        // Aplica a penalidade física do peso das mochilas na velocidade do passo.
        // Convertido para i64 para corresponder exatamente ao tipo esperado por MovementDeltaMm.
        let speed_multiplier = self.encumbrance_effects().speed_multiplier;
        let weighted_delta = MovementDeltaMm {
            x: ((desired_delta.x as f32) * speed_multiplier).round() as i64,
            y: ((desired_delta.y as f32) * speed_multiplier).round() as i64,
        };

        let traversal = resolve_traversal(self.position, weighted_delta)?;
        self.position = traversal.position;
        self.last_simulated_tick = tick;
        Ok(CharacterStepOutcome {
            traversal,
            expired_sequence,
        })
    }

    fn expire_stale_input(
        &mut self,
        tick: ServerTick,
        rules: MovementRules,
    ) -> anyhow::Result<Option<IntentSequence>> {
        if self.movement_input.is_zero() {
            return Ok(None);
        }

        let received_tick = self
            .last_input_tick
            .context("non-zero movement input has no authoritative receive tick")?;
        let age_ticks = tick
            .0
            .checked_sub(received_tick.0)
            .context("movement input receive tick is ahead of simulation tick")?;
        if age_ticks < rules.input_timeout_ticks() {
            return Ok(None);
        }

        let sequence = self
            .last_input_sequence
            .context("non-zero movement input has no authoritative sequence")?;
        self.movement_input = MovementInput::ZERO;
        self.movement_remainder = MovementRemainder::default();
        Ok(Some(sequence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CharacterMovementSettings;
    use aurenfall_domain::containers::{EncumbranceTier, ItemCategory, StoredItem};

    fn rules(speed_mm_per_second: u32, timeout_ticks: u64) -> anyhow::Result<MovementRules> {
        MovementRules::new(
            CharacterMovementSettings::new(speed_mm_per_second, timeout_ticks, 250)?,
            20,
        )
    }

    fn free_traversal(
        position: WorldPositionMm,
        desired: MovementDeltaMm,
    ) -> anyhow::Result<TraversalResolution> {
        let resolved = position
            .checked_translate_horizontal(desired.x, desired.y)
            .context("test traversal position overflow")?;
        Ok(TraversalResolution {
            position: resolved,
            desired,
            actual: desired,
            constrained_x: false,
            constrained_y: false,
        })
    }

    #[test]
    fn character_spawns_with_default_containers_and_tracks_weight() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        
        assert_eq!(character.containers().current_tier(), EncumbranceTier::Light);
        assert_eq!(character.encumbrance_effects().speed_multiplier, 1.0);

        let heavy_ore = StoredItem::new(1, 100, ItemCategory::Ore, 26_000, 1, 1, 1, 1);
        character.containers_mut().pickup_item(heavy_ore).unwrap();

        assert_eq!(character.containers().current_tier(), EncumbranceTier::Heavy);
        assert_eq!(character.encumbrance_effects().speed_multiplier, 0.6);
        assert!(!character.encumbrance_effects().can_sprint);

        Ok(())
    }

    #[test]
    fn encumbrance_slows_down_movement_integration_in_tick() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        let rules = rules(4_000, 5)?;
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;
        character.apply_movement_input(IntentSequence(1), input, ServerTick(0))?;

        // 1. Passo normal (Leve = 100% da velocidade -> 200 mm no tick)
        let outcome_light = character.simulate_tick(ServerTick(1), rules, free_traversal)?;
        assert_eq!(outcome_light.traversal.actual, MovementDeltaMm { x: 200, y: 0 });

        // 2. Coloca 26 kg de carga (fica Pesado -> 60% da velocidade)
        let heavy_ore = StoredItem::new(1, 100, ItemCategory::Ore, 26_000, 1, 1, 1, 1);
        character.containers_mut().pickup_item(heavy_ore).unwrap();

        // 3. Próximo passo: 200 mm * 0.6 = 120 mm no tick!
        let outcome_heavy = character.simulate_tick(ServerTick(2), rules, free_traversal)?;
        assert_eq!(outcome_heavy.traversal.actual, MovementDeltaMm { x: 120, y: 0 });
        assert_eq!(character.position(), WorldPositionMm::new(320, 0, 0)); // 200 + 120 = 320 mm

        Ok(())
    }

    #[test]
    fn movement_is_owned_and_integrated_by_character_state() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        let rules = rules(4_000, 5)?;
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;
        character.apply_movement_input(IntentSequence(1), input, ServerTick(0))?;
        assert_eq!(character.active_movement_sequence(), Some(IntentSequence(1)));

        let outcome = character.simulate_tick(ServerTick(1), rules, free_traversal)?;
        assert_eq!(outcome.traversal.actual, MovementDeltaMm { x: 200, y: 0 });
        assert_eq!(outcome.expired_sequence, None);
        assert_eq!(character.position(), WorldPositionMm::new(200, 0, 0));
        assert_eq!(character.last_simulated_tick, ServerTick(1));
        Ok(())
    }

    #[test]
    fn traversal_resolution_controls_committed_authoritative_position() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        let rules = rules(4_000, 5)?;
        character.apply_movement_input(
            IntentSequence(1),
            MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?,
            ServerTick(0),
        )?;

        let outcome = character.simulate_tick(ServerTick(1), rules, |position, desired| {
            Ok(TraversalResolution {
                position: WorldPositionMm::new(75, position.y(), position.z()),
                desired,
                actual: MovementDeltaMm { x: 75, y: 0 },
                constrained_x: true,
                constrained_y: desired.y != 0,
            })
        })?;

        assert!(outcome.traversal.constrained_x);
        assert_eq!(character.position(), WorldPositionMm::new(75, 0, 0));
        Ok(())
    }

    #[test]
    fn zero_input_stops_immediately_and_clears_fractional_remainder() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        let rules = rules(4_500, 5)?;
        character.apply_movement_input(
            IntentSequence(1),
            MovementInput::from_axes(16_384, 0).context("test input must be valid")?,
            ServerTick(0),
        )?;
        character.simulate_tick(ServerTick(1), rules, free_traversal)?;
        assert_ne!(character.movement_remainder, MovementRemainder::default());

        character.apply_movement_input(IntentSequence(2), MovementInput::ZERO, ServerTick(1))?;
        assert_eq!(character.active_movement_sequence(), None);
        assert_eq!(character.movement_remainder, MovementRemainder::default());
        let stopped_at = character.position();

        let outcome = character.simulate_tick(ServerTick(2), rules, free_traversal)?;
        assert_eq!(outcome.traversal.actual, MovementDeltaMm { x: 0, y: 0 });
        assert_eq!(outcome.expired_sequence, None);
        assert_eq!(character.position(), stopped_at);
        Ok(())
    }

    #[test]
    fn stale_nonzero_input_expires_before_timeout_tick_is_integrated() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        let rules = rules(4_000, 5)?;
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;
        character.apply_movement_input(IntentSequence(1), input, ServerTick(0))?;

        for tick in 1..5 {
            let outcome = character.simulate_tick(ServerTick(tick), rules, free_traversal)?;
            assert_eq!(outcome.traversal.actual, MovementDeltaMm { x: 200, y: 0 });
            assert_eq!(outcome.expired_sequence, None);
        }
        assert_eq!(character.position(), WorldPositionMm::new(800, 0, 0));

        let expired = character.simulate_tick(ServerTick(5), rules, free_traversal)?;
        assert_eq!(expired.traversal.actual, MovementDeltaMm { x: 0, y: 0 });
        assert_eq!(expired.expired_sequence, Some(IntentSequence(1)));
        assert_eq!(character.active_movement_sequence(), None);
        assert_eq!(character.position(), WorldPositionMm::new(800, 0, 0));
        assert_eq!(character.movement_remainder, MovementRemainder::default());

        let later = character.simulate_tick(ServerTick(6), rules, free_traversal)?;
        assert_eq!(later.traversal.actual, MovementDeltaMm { x: 0, y: 0 });
        assert_eq!(later.expired_sequence, None);
        assert_eq!(character.position(), WorldPositionMm::new(800, 0, 0));
        Ok(())
    }

    #[test]
    fn fresh_input_after_deadman_timeout_resumes_movement() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        let rules = rules(4_000, 2)?;
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;
        character.apply_movement_input(IntentSequence(1), input, ServerTick(0))?;
        character.simulate_tick(ServerTick(1), rules, free_traversal)?;
        let expired = character.simulate_tick(ServerTick(2), rules, free_traversal)?;
        assert_eq!(expired.expired_sequence, Some(IntentSequence(1)));
        assert_eq!(character.position(), WorldPositionMm::new(200, 0, 0));

        character.apply_movement_input(IntentSequence(2), input, ServerTick(2))?;
        assert_eq!(character.active_movement_sequence(), Some(IntentSequence(2)));
        let resumed = character.simulate_tick(ServerTick(3), rules, free_traversal)?;
        assert_eq!(resumed.traversal.actual, MovementDeltaMm { x: 200, y: 0 });
        assert_eq!(resumed.expired_sequence, None);
        assert_eq!(character.position(), WorldPositionMm::new(400, 0, 0));
        Ok(())
    }

    #[test]
    fn character_state_rejects_non_monotonic_zone_sequences() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        character.apply_movement_input(IntentSequence(5), MovementInput::ZERO, ServerTick(0))?;

        assert!(
            character
                .apply_movement_input(IntentSequence(5), MovementInput::ZERO, ServerTick(0))
                .is_err()
        );
        assert!(
            character
                .apply_movement_input(IntentSequence(4), MovementInput::ZERO, ServerTick(0))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn character_state_rejects_stale_receive_tick() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(0))?;
        let rules = rules(4_000, 5)?;
        character.simulate_tick(ServerTick(1), rules, free_traversal)?;

        assert!(
            character
                .apply_movement_input(IntentSequence(1), MovementInput::ZERO, ServerTick(0))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn character_state_rejects_duplicate_or_reversed_ticks() -> anyhow::Result<()> {
        let mut character = CharacterState::spawn(CharacterId(7), WorldPositionMm::ORIGIN, ServerTick(10))?;
        let rules = rules(4_000, 5)?;

        assert!(
            character
                .simulate_tick(ServerTick(10), rules, free_traversal)
                .is_err()
        );
        assert!(
            character
                .simulate_tick(ServerTick(9), rules, free_traversal)
                .is_err()
        );
        Ok(())
    }
}