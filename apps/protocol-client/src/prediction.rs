use std::{error::Error, fmt, time::Instant};

use aurenfall_contracts::{MOVE_AXIS_MAX, MovementPredictionProfile, SelfMovementSnapshot};

use crate::reconciliation::ReplayPlan;

const MICROS_PER_SECOND: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientTimeUs(u64);

impl ClientTimeUs {
    #[cfg(test)]
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    pub fn checked_duration_since(self, earlier: Self) -> Result<u64, PredictionClockError> {
        self.0
            .checked_sub(earlier.0)
            .ok_or(PredictionClockError::TimeRegressed {
                earlier_us: earlier.0,
                later_us: self.0,
            })
    }
}

#[derive(Debug)]
pub struct ClientPredictionClock {
    origin: Instant,
}

impl ClientPredictionClock {
    #[must_use]
    pub fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    pub fn now(&self) -> Result<ClientTimeUs, PredictionClockError> {
        let elapsed_us = self.origin.elapsed().as_micros();
        let elapsed_us = u64::try_from(elapsed_us)
            .map_err(|_| PredictionClockError::ElapsedTimeOverflow { elapsed_us })?;
        Ok(ClientTimeUs(elapsed_us))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictedPositionMm {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct PositionalPredictor {
    profile: MovementPredictionProfile,
    max_input_lifetime_us: u64,
}

impl PositionalPredictor {
    pub fn new(profile: MovementPredictionProfile) -> Result<Self, PredictionError> {
        let timeout_numerator = profile
            .movement_input_timeout_ticks
            .checked_mul(MICROS_PER_SECOND)
            .ok_or(PredictionError::TimeoutOverflow)?;
        let max_input_lifetime_us = timeout_numerator / u64::from(profile.simulation_tick_hz);
        if max_input_lifetime_us == 0 {
            return Err(PredictionError::ZeroPredictableLifetime);
        }
        Ok(Self {
            profile,
            max_input_lifetime_us,
        })
    }

    pub fn predict(
        self,
        baseline: SelfMovementSnapshot,
        replay: &ReplayPlan,
    ) -> Result<PredictedPositionMm, PredictionError> {
        let denominator = i128::from(MOVE_AXIS_MAX)
            .checked_mul(i128::from(MICROS_PER_SECOND))
            .ok_or(PredictionError::IntegrationOverflow)?;
        let speed = i128::from(self.profile.speed_mm_per_second);
        let mut x = i128::from(baseline.x_mm);
        let mut y = i128::from(baseline.y_mm);
        let mut remainder_x = 0_i128;
        let mut remainder_y = 0_i128;

        for segment in &replay.segments {
            let duration_us = segment.duration_us.min(self.max_input_lifetime_us);
            let (axis_x, axis_y) = normalized_axes(segment.intent.axis_x, segment.intent.axis_y);
            let numerator_x = remainder_x
                .checked_add(
                    speed
                        .checked_mul(i128::from(axis_x))
                        .and_then(|value| value.checked_mul(i128::from(duration_us)))
                        .ok_or(PredictionError::IntegrationOverflow)?,
                )
                .ok_or(PredictionError::IntegrationOverflow)?;
            let numerator_y = remainder_y
                .checked_add(
                    speed
                        .checked_mul(i128::from(axis_y))
                        .and_then(|value| value.checked_mul(i128::from(duration_us)))
                        .ok_or(PredictionError::IntegrationOverflow)?,
                )
                .ok_or(PredictionError::IntegrationOverflow)?;

            x = x
                .checked_add(numerator_x / denominator)
                .ok_or(PredictionError::PositionOverflow)?;
            y = y
                .checked_add(numerator_y / denominator)
                .ok_or(PredictionError::PositionOverflow)?;
            remainder_x = numerator_x % denominator;
            remainder_y = numerator_y % denominator;
        }

        Ok(PredictedPositionMm {
            x: i64::try_from(x).map_err(|_| PredictionError::PositionOverflow)?,
            y: i64::try_from(y).map_err(|_| PredictionError::PositionOverflow)?,
            z: baseline.z_mm,
        })
    }
}

fn normalized_axes(axis_x: i16, axis_y: i16) -> (i64, i64) {
    let x = i64::from(axis_x);
    let y = i64::from(axis_y);
    let scale = i64::from(MOVE_AXIS_MAX);
    let magnitude_squared = (x * x + y * y) as u64;
    let scale_squared = (scale * scale) as u64;

    if magnitude_squared <= scale_squared {
        return (x, y);
    }

    let magnitude = integer_sqrt_ceil(magnitude_squared) as i64;
    (x * scale / magnitude, y * scale / magnitude)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionClockError {
    ElapsedTimeOverflow { elapsed_us: u128 },
    TimeRegressed { earlier_us: u64, later_us: u64 },
}

impl fmt::Display for PredictionClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ElapsedTimeOverflow { elapsed_us } => {
                write!(
                    formatter,
                    "client prediction clock overflowed at {elapsed_us} microseconds"
                )
            }
            Self::TimeRegressed { earlier_us, later_us } => write!(
                formatter,
                "client prediction time regressed: earlier={earlier_us}us, later={later_us}us"
            ),
        }
    }
}

impl Error for PredictionClockError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionError {
    TimeoutOverflow,
    ZeroPredictableLifetime,
    IntegrationOverflow,
    PositionOverflow,
}

impl fmt::Display for PredictionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeoutOverflow => write!(formatter, "prediction timeout conversion overflowed"),
            Self::ZeroPredictableLifetime => {
                write!(formatter, "prediction profile produced zero input lifetime")
            }
            Self::IntegrationOverflow => {
                write!(formatter, "client positional prediction integration overflowed")
            }
            Self::PositionOverflow => write!(
                formatter,
                "client predicted position overflowed i64 millimeter space"
            ),
        }
    }
}

impl Error for PredictionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconciliation::{ReplayPlan, ReplaySegment};
    use aurenfall_contracts::{MoveIntent, PresentationCorrectionProfile};

    fn profile() -> Result<MovementPredictionProfile, Box<dyn Error>> {
        let correction = PresentationCorrectionProfile::new(25, 250, 1_000, 120, 50)?;
        Ok(MovementPredictionProfile::new(4_500, 20, 250, 5, correction)?)
    }

    #[test]
    fn client_time_computes_monotonic_duration() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            ClientTimeUs::new(230).checked_duration_since(ClientTimeUs::new(160))?,
            70
        );
        Ok(())
    }

    #[test]
    fn client_time_rejects_regression() {
        assert_eq!(
            ClientTimeUs::new(99).checked_duration_since(ClientTimeUs::new(100)),
            Err(PredictionClockError::TimeRegressed {
                earlier_us: 100,
                later_us: 99,
            })
        );
    }

    #[test]
    fn half_axis_prediction_integrates_elapsed_microtime() -> Result<(), Box<dyn Error>> {
        let predictor = PositionalPredictor::new(profile()?)?;
        let intent = MoveIntent::new(2, 16_384, 0)?;
        let replay = ReplayPlan {
            generated_at_us: 50_000,
            total_duration_us: 50_000,
            segments: vec![ReplaySegment {
                intent,
                start_us: 0,
                end_us: 50_000,
                duration_us: 50_000,
            }],
        };
        let baseline = SelfMovementSnapshot {
            server_tick: 10,
            x_mm: 112,
            y_mm: 0,
            z_mm: 0,
            last_processed_input_sequence: Some(1),
        };

        assert_eq!(
            predictor.predict(baseline, &replay)?,
            PredictedPositionMm { x: 224, y: 0, z: 0 }
        );
        Ok(())
    }

    #[test]
    fn prediction_respects_server_deadman_lifetime() -> Result<(), Box<dyn Error>> {
        let predictor = PositionalPredictor::new(profile()?)?;
        let intent = MoveIntent::new(2, i16::MAX, 0)?;
        let replay = ReplayPlan {
            generated_at_us: 1_000_000,
            total_duration_us: 1_000_000,
            segments: vec![ReplaySegment {
                intent,
                start_us: 0,
                end_us: 1_000_000,
                duration_us: 1_000_000,
            }],
        };
        let baseline = SelfMovementSnapshot {
            server_tick: 10,
            x_mm: 0,
            y_mm: 0,
            z_mm: 0,
            last_processed_input_sequence: Some(1),
        };

        let predicted = predictor.predict(baseline, &replay)?;
        assert_eq!(predicted.x, 1_125);
        Ok(())
    }
}
