use std::{error::Error, fmt};

use aurenfall_contracts::PresentationCorrectionProfile;

use crate::prediction::{ClientTimeUs, PredictedPositionMm, PredictionClockError};

const MICROS_PER_MILLISECOND: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionMode {
    Absorb,
    Smooth,
    Rapid,
    HardSnap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresentationOffsetMm {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

impl PresentationOffsetMm {
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionDecision {
    pub mode: CorrectionMode,
    pub distance_mm: u64,
    pub duration_us: u64,
    pub active_offset: PresentationOffsetMm,
}

#[derive(Debug, Clone, Copy)]
struct ActiveCorrection {
    initial_offset: PresentationOffsetMm,
    started_at: ClientTimeUs,
    duration_us: u64,
}

#[derive(Debug)]
pub struct PresentationReconciler {
    profile: PresentationCorrectionProfile,
    smooth_duration_us: u64,
    rapid_duration_us: u64,
    active: Option<ActiveCorrection>,
}

impl PresentationReconciler {
    pub fn new(profile: PresentationCorrectionProfile) -> Result<Self, PresentationError> {
        let smooth_duration_us = u64::from(profile.smooth_duration_ms)
            .checked_mul(MICROS_PER_MILLISECOND)
            .ok_or(PresentationError::DurationOverflow)?;
        let rapid_duration_us = u64::from(profile.rapid_duration_ms)
            .checked_mul(MICROS_PER_MILLISECOND)
            .ok_or(PresentationError::DurationOverflow)?;
        Ok(Self {
            profile,
            smooth_duration_us,
            rapid_duration_us,
            active: None,
        })
    }

    pub fn reconcile(
        &mut self,
        old_gameplay: PredictedPositionMm,
        new_gameplay: PredictedPositionMm,
        now: ClientTimeUs,
    ) -> Result<CorrectionDecision, PresentationError> {
        let current_offset = self.sample_offset(now)?;
        let old_rendered = add_offset(old_gameplay, current_offset)?;
        let requested_offset = position_delta(old_rendered, new_gameplay)?;
        let distance_mm = magnitude_mm(requested_offset)?;

        let (mode, duration_us) = if distance_mm <= u64::from(self.profile.absorb_max_mm) {
            (CorrectionMode::Absorb, 0)
        } else if distance_mm <= u64::from(self.profile.smooth_max_mm) {
            (CorrectionMode::Smooth, self.smooth_duration_us)
        } else if distance_mm < u64::from(self.profile.hard_snap_threshold_mm) {
            (CorrectionMode::Rapid, self.rapid_duration_us)
        } else {
            (CorrectionMode::HardSnap, 0)
        };

        let active_offset = match mode {
            CorrectionMode::Smooth | CorrectionMode::Rapid => {
                self.active = Some(ActiveCorrection {
                    initial_offset: requested_offset,
                    started_at: now,
                    duration_us,
                });
                requested_offset
            }
            CorrectionMode::Absorb | CorrectionMode::HardSnap => {
                self.active = None;
                PresentationOffsetMm::ZERO
            }
        };

        Ok(CorrectionDecision {
            mode,
            distance_mm,
            duration_us,
            active_offset,
        })
    }

    pub fn sample_offset(&mut self, now: ClientTimeUs) -> Result<PresentationOffsetMm, PresentationError> {
        let Some(active) = self.active else {
            return Ok(PresentationOffsetMm::ZERO);
        };
        let elapsed_us = now.checked_duration_since(active.started_at)?;
        if elapsed_us >= active.duration_us {
            self.active = None;
            return Ok(PresentationOffsetMm::ZERO);
        }

        let remaining_us = active.duration_us - elapsed_us;
        Ok(PresentationOffsetMm {
            x: decay_component(active.initial_offset.x, remaining_us, active.duration_us)?,
            y: decay_component(active.initial_offset.y, remaining_us, active.duration_us)?,
            z: decay_component(active.initial_offset.z, remaining_us, active.duration_us)?,
        })
    }

    pub fn rendered_position(
        &mut self,
        gameplay: PredictedPositionMm,
        now: ClientTimeUs,
    ) -> Result<PredictedPositionMm, PresentationError> {
        add_offset(gameplay, self.sample_offset(now)?)
    }
}

fn add_offset(
    position: PredictedPositionMm,
    offset: PresentationOffsetMm,
) -> Result<PredictedPositionMm, PresentationError> {
    Ok(PredictedPositionMm {
        x: position
            .x
            .checked_add(offset.x)
            .ok_or(PresentationError::PositionOverflow)?,
        y: position
            .y
            .checked_add(offset.y)
            .ok_or(PresentationError::PositionOverflow)?,
        z: position
            .z
            .checked_add(offset.z)
            .ok_or(PresentationError::PositionOverflow)?,
    })
}

fn position_delta(
    from: PredictedPositionMm,
    to: PredictedPositionMm,
) -> Result<PresentationOffsetMm, PresentationError> {
    Ok(PresentationOffsetMm {
        x: from
            .x
            .checked_sub(to.x)
            .ok_or(PresentationError::PositionOverflow)?,
        y: from
            .y
            .checked_sub(to.y)
            .ok_or(PresentationError::PositionOverflow)?,
        z: from
            .z
            .checked_sub(to.z)
            .ok_or(PresentationError::PositionOverflow)?,
    })
}

fn decay_component(value: i64, remaining_us: u64, duration_us: u64) -> Result<i64, PresentationError> {
    let scaled = i128::from(value)
        .checked_mul(i128::from(remaining_us))
        .ok_or(PresentationError::InterpolationOverflow)?
        / i128::from(duration_us);
    i64::try_from(scaled).map_err(|_| PresentationError::InterpolationOverflow)
}

fn magnitude_mm(offset: PresentationOffsetMm) -> Result<u64, PresentationError> {
    let x = u128::from(offset.x.unsigned_abs());
    let y = u128::from(offset.y.unsigned_abs());
    let z = u128::from(offset.z.unsigned_abs());
    let squared = x
        .checked_mul(x)
        .and_then(|value| y.checked_mul(y).and_then(|y2| value.checked_add(y2)))
        .and_then(|value| z.checked_mul(z).and_then(|z2| value.checked_add(z2)))
        .ok_or(PresentationError::DistanceOverflow)?;
    let magnitude = integer_sqrt_floor(squared);
    u64::try_from(magnitude).map_err(|_| PresentationError::DistanceOverflow)
}

fn integer_sqrt_floor(value: u128) -> u128 {
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
pub enum PresentationError {
    DurationOverflow,
    PositionOverflow,
    InterpolationOverflow,
    DistanceOverflow,
    PredictionClock(PredictionClockError),
}

impl From<PredictionClockError> for PresentationError {
    fn from(error: PredictionClockError) -> Self {
        Self::PredictionClock(error)
    }
}

impl fmt::Display for PresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DurationOverflow => write!(formatter, "presentation correction duration overflowed"),
            Self::PositionOverflow => write!(formatter, "presentation position arithmetic overflowed"),
            Self::InterpolationOverflow => {
                write!(formatter, "presentation correction interpolation overflowed")
            }
            Self::DistanceOverflow => write!(formatter, "presentation correction distance overflowed"),
            Self::PredictionClock(error) => write!(formatter, "presentation prediction clock error: {error}"),
        }
    }
}

impl Error for PresentationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Result<PresentationCorrectionProfile, Box<dyn Error>> {
        Ok(PresentationCorrectionProfile::new(25, 250, 1_000, 120, 50)?)
    }

    fn position(x: i64) -> PredictedPositionMm {
        PredictedPositionMm { x, y: 0, z: 0 }
    }

    #[test]
    fn tiny_correction_is_absorbed_without_visual_offset() -> Result<(), Box<dyn Error>> {
        let mut reconciler = PresentationReconciler::new(profile()?)?;
        let decision = reconciler.reconcile(position(20), position(0), ClientTimeUs::new(0))?;

        assert_eq!(decision.mode, CorrectionMode::Absorb);
        assert_eq!(decision.distance_mm, 20);
        assert_eq!(decision.active_offset, PresentationOffsetMm::ZERO);
        assert_eq!(
            reconciler.rendered_position(position(0), ClientTimeUs::new(0))?,
            position(0)
        );
        Ok(())
    }

    #[test]
    fn smooth_correction_preserves_visual_position_then_decays() -> Result<(), Box<dyn Error>> {
        let mut reconciler = PresentationReconciler::new(profile()?)?;
        let decision = reconciler.reconcile(position(120), position(0), ClientTimeUs::new(0))?;

        assert_eq!(decision.mode, CorrectionMode::Smooth);
        assert_eq!(decision.duration_us, 120_000);
        assert_eq!(
            reconciler.rendered_position(position(0), ClientTimeUs::new(0))?,
            position(120)
        );
        assert_eq!(
            reconciler.rendered_position(position(0), ClientTimeUs::new(60_000))?,
            position(60)
        );
        assert_eq!(
            reconciler.rendered_position(position(0), ClientTimeUs::new(120_000))?,
            position(0)
        );
        Ok(())
    }

    #[test]
    fn medium_large_correction_uses_rapid_window() -> Result<(), Box<dyn Error>> {
        let mut reconciler = PresentationReconciler::new(profile()?)?;
        let decision = reconciler.reconcile(position(500), position(0), ClientTimeUs::new(0))?;

        assert_eq!(decision.mode, CorrectionMode::Rapid);
        assert_eq!(decision.duration_us, 50_000);
        Ok(())
    }

    #[test]
    fn large_correction_hard_snaps_to_gameplay_position() -> Result<(), Box<dyn Error>> {
        let mut reconciler = PresentationReconciler::new(profile()?)?;
        let decision = reconciler.reconcile(position(1_000), position(0), ClientTimeUs::new(0))?;

        assert_eq!(decision.mode, CorrectionMode::HardSnap);
        assert_eq!(decision.active_offset, PresentationOffsetMm::ZERO);
        assert_eq!(
            reconciler.rendered_position(position(0), ClientTimeUs::new(0))?,
            position(0)
        );
        Ok(())
    }

    #[test]
    fn new_correction_preserves_rendered_continuity_while_previous_one_is_active()
    -> Result<(), Box<dyn Error>> {
        let mut reconciler = PresentationReconciler::new(profile()?)?;
        reconciler.reconcile(position(200), position(0), ClientTimeUs::new(0))?;

        let before = reconciler.rendered_position(position(50), ClientTimeUs::new(60_000))?;
        assert_eq!(before, position(150));

        let decision = reconciler.reconcile(position(50), position(20), ClientTimeUs::new(60_000))?;
        assert_eq!(decision.mode, CorrectionMode::Smooth);
        assert_eq!(decision.active_offset.x, 130);
        assert_eq!(
            reconciler.rendered_position(position(20), ClientTimeUs::new(60_000))?,
            before
        );
        Ok(())
    }
}
