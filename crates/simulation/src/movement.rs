use anyhow::{Context, bail};
use aurenfall_gamedata::MovementConfig;

const MOVEMENT_AXIS_SCALE: i64 = i16::MAX as i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementInput {
    axis_x: i16,
    axis_y: i16,
}

impl MovementInput {
    pub const ZERO: Self = Self { axis_x: 0, axis_y: 0 };

    #[must_use]
    pub const fn from_axes(axis_x: i16, axis_y: i16) -> Option<Self> {
        if axis_x == i16::MIN || axis_y == i16::MIN {
            return None;
        }
        Some(Self { axis_x, axis_y })
    }

    #[must_use]
    pub const fn axis_x(self) -> i16 {
        self.axis_x
    }

    #[must_use]
    pub const fn axis_y(self) -> i16 {
        self.axis_y
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.axis_x == 0 && self.axis_y == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterMovementSettings {
    speed_mm_per_second: u32,
    input_timeout_ticks: u64,
    character_radius_mm: u32,
}

impl CharacterMovementSettings {
    /// Construtor base original: valida e inicializa os campos
    pub fn new(
        speed_mm_per_second: u32,
        input_timeout_ticks: u64,
        character_radius_mm: u32,
    ) -> anyhow::Result<Self> {
        if speed_mm_per_second == 0 {
            bail!("character movement speed must be > 0 mm/s");
        }
        if input_timeout_ticks == 0 {
            bail!("movement input timeout must be > 0 ticks");
        }
        if character_radius_mm == 0 {
            bail!("character traversal radius must be > 0 mm");
        }
        Ok(Self {
            speed_mm_per_second,
            input_timeout_ticks,
            character_radius_mm,
        })
    }

    /// Cria as configurações lendo do GameData
    pub fn from_gamedata(config: &MovementConfig) -> anyhow::Result<Self> {
        Self::new(
            config.speeds.speed_mm_per_second,
            config.physics.input_timeout_ticks,
            config.physics.character_radius_mm,
        )
    }

    /// Cria a versão de corrida rápida (sprint) do GameData
    pub fn sprint_from_gamedata(config: &MovementConfig) -> anyhow::Result<Self> {
        Self::new(
            config.speeds.sprint_speed_mm_per_second,
            config.physics.input_timeout_ticks,
            config.physics.character_radius_mm,
        )
    }

    #[must_use]
    pub const fn speed_mm_per_second(self) -> u32 {
        self.speed_mm_per_second
    }

    #[must_use]
    pub const fn input_timeout_ticks(self) -> u64 {
        self.input_timeout_ticks
    }

    #[must_use]
    pub const fn character_radius_mm(self) -> u32 {
        self.character_radius_mm
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MovementRemainder {
    x: i64,
    y: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MovementDeltaMm {
    pub(crate) x: i64,
    pub(crate) y: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MovementRules {
    speed_mm_per_second: i64,
    denominator: i64,
    input_timeout_ticks: u64,
    character_radius_mm: i64,
}

impl MovementRules {
    pub(crate) fn new(settings: CharacterMovementSettings, tick_rate_hz: u32) -> anyhow::Result<Self> {
        if tick_rate_hz == 0 {
            bail!("movement tick rate must be > 0");
        }

        let denominator = MOVEMENT_AXIS_SCALE
            .checked_mul(i64::from(tick_rate_hz))
            .context("movement denominator overflow")?;
        Ok(Self {
            speed_mm_per_second: i64::from(settings.speed_mm_per_second),
            denominator,
            input_timeout_ticks: settings.input_timeout_ticks,
            character_radius_mm: i64::from(settings.character_radius_mm),
        })
    }

    #[must_use]
    pub(crate) const fn input_timeout_ticks(self) -> u64 {
        self.input_timeout_ticks
    }

    #[must_use]
    pub(crate) const fn character_radius_mm(self) -> i64 {
        self.character_radius_mm
    }

    pub(crate) fn integrate(
        self,
        input: MovementInput,
        remainder: &mut MovementRemainder,
    ) -> anyhow::Result<MovementDeltaMm> {
        let (axis_x, axis_y) = normalized_axes(input);
        let numerator_x = remainder
            .x
            .checked_add(
                self.speed_mm_per_second
                    .checked_mul(axis_x)
                    .context("movement X numerator overflow")?,
            )
            .context("movement X remainder overflow")?;
        let numerator_y = remainder
            .y
            .checked_add(
                self.speed_mm_per_second
                    .checked_mul(axis_y)
                    .context("movement Y numerator overflow")?,
            )
            .context("movement Y remainder overflow")?;

        let delta = MovementDeltaMm {
            x: numerator_x / self.denominator,
            y: numerator_y / self.denominator,
        };
        remainder.x = numerator_x % self.denominator;
        remainder.y = numerator_y % self.denominator;
        Ok(delta)
    }
}

fn normalized_axes(input: MovementInput) -> (i64, i64) {
    let x = i64::from(input.axis_x);
    let y = i64::from(input.axis_y);
    let magnitude_squared = (x * x + y * y) as u64;
    let scale_squared = (MOVEMENT_AXIS_SCALE * MOVEMENT_AXIS_SCALE) as u64;

    if magnitude_squared <= scale_squared {
        return (x, y);
    }

    let magnitude = integer_sqrt_ceil(magnitude_squared) as i64;
    (
        x * MOVEMENT_AXIS_SCALE / magnitude,
        y * MOVEMENT_AXIS_SCALE / magnitude,
    )
}

fn integer_sqrt_ceil(value: u64) -> u64 {
    let floor = integer_sqrt_floor(value);
    if floor * floor == value { floor } else { floor + 1 }
}

fn integer_sqrt_floor(value: u64) -> u64 {
    if value < 2 {
        return value;
    }

    let mut current = value;
    let mut next = current.div_ceil(2);
    while next < current {
        current = next;
        next = (current + value / current) / 2;
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> anyhow::Result<CharacterMovementSettings> {
        CharacterMovementSettings::new(4_000, 5, 250)
    }

    #[test]
    fn movement_input_rejects_asymmetric_i16_minimum() {
        assert_eq!(MovementInput::from_axes(i16::MIN, 0), None);
        assert_eq!(MovementInput::from_axes(0, i16::MIN), None);
        assert_eq!(
            MovementInput::from_axes(i16::MIN + 1, i16::MAX),
            Some(MovementInput {
                axis_x: i16::MIN + 1,
                axis_y: i16::MAX,
            })
        );
    }

    #[test]
    fn movement_settings_reject_invalid_authoritative_values() {
        assert!(CharacterMovementSettings::new(0, 5, 250).is_err());
        assert!(CharacterMovementSettings::new(4_000, 0, 250).is_err());
        assert!(CharacterMovementSettings::new(4_000, 5, 0).is_err());
    }

    #[test]
    fn full_diagonal_is_normalized_without_floating_point() {
        let input = MovementInput::from_axes(i16::MAX, i16::MAX).unwrap_or(MovementInput::ZERO);
        let (x, y) = normalized_axes(input);
        let magnitude_squared = x * x + y * y;
        assert!(magnitude_squared <= MOVEMENT_AXIS_SCALE * MOVEMENT_AXIS_SCALE);
        assert_eq!(x, y);
        assert!(x > 23_000 && x < 23_200);
    }

    #[test]
    fn half_cardinal_input_preserves_analog_magnitude() -> anyhow::Result<()> {
        let rules = MovementRules::new(settings()?, 20)?;
        let input = MovementInput::from_axes(16_384, 0).context("test input must be valid")?;
        let mut remainder = MovementRemainder::default();
        let first = rules.integrate(input, &mut remainder)?;
        let second = rules.integrate(input, &mut remainder)?;

        assert_eq!(first.y, 0);
        assert_eq!(second.y, 0);
        assert_eq!(first.x + second.x, 200);
        Ok(())
    }

    #[test]
    fn fractional_remainder_prevents_long_term_tick_truncation_drift() -> anyhow::Result<()> {
        let rules = MovementRules::new(CharacterMovementSettings::new(1_000, 5, 250)?, 60)?;
        let input = MovementInput::from_axes(i16::MAX, 0).context("test input must be valid")?;
        let mut remainder = MovementRemainder::default();
        let mut distance = 0_i64;

        for _ in 0..60 {
            distance += rules.integrate(input, &mut remainder)?.x;
        }

        assert_eq!(distance, 1_000);
        Ok(())
    }
}
