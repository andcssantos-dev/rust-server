use thiserror::Error;

pub const MOVEMENT_PREDICTION_PROFILE_BYTES: usize = 44;
pub const MOVEMENT_PREDICTION_PROFILE_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationCorrectionProfile {
    pub absorb_max_mm: u32,
    pub smooth_max_mm: u32,
    pub hard_snap_threshold_mm: u32,
    pub smooth_duration_ms: u32,
    pub rapid_duration_ms: u32,
}

impl PresentationCorrectionProfile {
    pub fn new(
        absorb_max_mm: u32,
        smooth_max_mm: u32,
        hard_snap_threshold_mm: u32,
        smooth_duration_ms: u32,
        rapid_duration_ms: u32,
    ) -> Result<Self, PredictionProfileCodecError> {
        let profile = Self {
            absorb_max_mm,
            smooth_max_mm,
            hard_snap_threshold_mm,
            smooth_duration_ms,
            rapid_duration_ms,
        };
        profile.validate()?;
        Ok(profile)
    }

    fn validate(self) -> Result<(), PredictionProfileCodecError> {
        if self.absorb_max_mm >= self.smooth_max_mm {
            return Err(PredictionProfileCodecError::InvalidCorrectionThresholds {
                absorb_max_mm: self.absorb_max_mm,
                smooth_max_mm: self.smooth_max_mm,
                hard_snap_threshold_mm: self.hard_snap_threshold_mm,
            });
        }
        if self.smooth_max_mm >= self.hard_snap_threshold_mm {
            return Err(PredictionProfileCodecError::InvalidCorrectionThresholds {
                absorb_max_mm: self.absorb_max_mm,
                smooth_max_mm: self.smooth_max_mm,
                hard_snap_threshold_mm: self.hard_snap_threshold_mm,
            });
        }
        if self.smooth_duration_ms == 0 {
            return Err(PredictionProfileCodecError::ZeroValue("smooth_duration_ms"));
        }
        if self.rapid_duration_ms == 0 {
            return Err(PredictionProfileCodecError::ZeroValue("rapid_duration_ms"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementPredictionProfile {
    pub profile_version: u32,
    pub speed_mm_per_second: u32,
    pub simulation_tick_hz: u32,
    pub character_radius_mm: u32,
    pub movement_input_timeout_ticks: u64,
    pub presentation_correction: PresentationCorrectionProfile,
}

impl MovementPredictionProfile {
    pub fn new(
        speed_mm_per_second: u32,
        simulation_tick_hz: u32,
        character_radius_mm: u32,
        movement_input_timeout_ticks: u64,
        presentation_correction: PresentationCorrectionProfile,
    ) -> Result<Self, PredictionProfileCodecError> {
        let profile = Self {
            profile_version: MOVEMENT_PREDICTION_PROFILE_VERSION,
            speed_mm_per_second,
            simulation_tick_hz,
            character_radius_mm,
            movement_input_timeout_ticks,
            presentation_correction,
        };
        profile.validate()?;
        Ok(profile)
    }

    #[must_use]
    pub fn encode(self) -> [u8; MOVEMENT_PREDICTION_PROFILE_BYTES] {
        let mut bytes = [0_u8; MOVEMENT_PREDICTION_PROFILE_BYTES];
        bytes[0..4].copy_from_slice(&self.profile_version.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.speed_mm_per_second.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.simulation_tick_hz.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.character_radius_mm.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.movement_input_timeout_ticks.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.presentation_correction.absorb_max_mm.to_le_bytes());
        bytes[28..32].copy_from_slice(&self.presentation_correction.smooth_max_mm.to_le_bytes());
        bytes[32..36].copy_from_slice(&self.presentation_correction.hard_snap_threshold_mm.to_le_bytes());
        bytes[36..40].copy_from_slice(&self.presentation_correction.smooth_duration_ms.to_le_bytes());
        bytes[40..44].copy_from_slice(&self.presentation_correction.rapid_duration_ms.to_le_bytes());
        bytes
    }

    pub fn decode(payload: &[u8]) -> Result<Self, PredictionProfileCodecError> {
        if payload.len() != MOVEMENT_PREDICTION_PROFILE_BYTES {
            return Err(PredictionProfileCodecError::InvalidLength {
                expected: MOVEMENT_PREDICTION_PROFILE_BYTES,
                actual: payload.len(),
            });
        }

        let presentation_correction = PresentationCorrectionProfile {
            absorb_max_mm: decode_u32(payload, 24)?,
            smooth_max_mm: decode_u32(payload, 28)?,
            hard_snap_threshold_mm: decode_u32(payload, 32)?,
            smooth_duration_ms: decode_u32(payload, 36)?,
            rapid_duration_ms: decode_u32(payload, 40)?,
        };
        let profile = Self {
            profile_version: decode_u32(payload, 0)?,
            speed_mm_per_second: decode_u32(payload, 4)?,
            simulation_tick_hz: decode_u32(payload, 8)?,
            character_radius_mm: decode_u32(payload, 12)?,
            movement_input_timeout_ticks: decode_u64(payload, 16)?,
            presentation_correction,
        };
        profile.validate()?;
        Ok(profile)
    }

    fn validate(self) -> Result<(), PredictionProfileCodecError> {
        if self.profile_version != MOVEMENT_PREDICTION_PROFILE_VERSION {
            return Err(PredictionProfileCodecError::UnsupportedVersion {
                received: self.profile_version,
                supported: MOVEMENT_PREDICTION_PROFILE_VERSION,
            });
        }
        if self.speed_mm_per_second == 0 {
            return Err(PredictionProfileCodecError::ZeroValue("speed_mm_per_second"));
        }
        if self.simulation_tick_hz == 0 {
            return Err(PredictionProfileCodecError::ZeroValue("simulation_tick_hz"));
        }
        if self.character_radius_mm == 0 {
            return Err(PredictionProfileCodecError::ZeroValue("character_radius_mm"));
        }
        if self.movement_input_timeout_ticks == 0 {
            return Err(PredictionProfileCodecError::ZeroValue(
                "movement_input_timeout_ticks",
            ));
        }
        self.presentation_correction.validate()
    }
}

fn decode_u32(payload: &[u8], start: usize) -> Result<u32, PredictionProfileCodecError> {
    Ok(u32::from_le_bytes(payload[start..start + 4].try_into().map_err(
        |_| PredictionProfileCodecError::InvalidLength {
            expected: MOVEMENT_PREDICTION_PROFILE_BYTES,
            actual: payload.len(),
        },
    )?))
}

fn decode_u64(payload: &[u8], start: usize) -> Result<u64, PredictionProfileCodecError> {
    Ok(u64::from_le_bytes(payload[start..start + 8].try_into().map_err(
        |_| PredictionProfileCodecError::InvalidLength {
            expected: MOVEMENT_PREDICTION_PROFILE_BYTES,
            actual: payload.len(),
        },
    )?))
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PredictionProfileCodecError {
    #[error("movement prediction profile payload length {actual} does not equal {expected}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("movement prediction profile version {received} is unsupported; expected {supported}")]
    UnsupportedVersion { received: u32, supported: u32 },
    #[error("movement prediction profile field {0} must be non-zero")]
    ZeroValue(&'static str),
    #[error(
        "presentation correction thresholds must satisfy absorb < smooth < hard snap; got {absorb_max_mm}, {smooth_max_mm}, {hard_snap_threshold_mm} mm"
    )]
    InvalidCorrectionThresholds {
        absorb_max_mm: u32,
        smooth_max_mm: u32,
        hard_snap_threshold_mm: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correction_profile() -> Result<PresentationCorrectionProfile, PredictionProfileCodecError> {
        PresentationCorrectionProfile::new(25, 250, 1_000, 120, 50)
    }

    #[test]
    fn movement_prediction_profile_roundtrips_server_owned_parameters()
    -> Result<(), PredictionProfileCodecError> {
        let profile = MovementPredictionProfile::new(4_500, 20, 250, 5, correction_profile()?)?;
        assert_eq!(MovementPredictionProfile::decode(&profile.encode())?, profile);
        Ok(())
    }

    #[test]
    fn movement_prediction_profile_rejects_invalid_values() -> Result<(), PredictionProfileCodecError> {
        let correction = correction_profile()?;
        assert!(MovementPredictionProfile::new(0, 20, 250, 5, correction).is_err());
        assert!(MovementPredictionProfile::new(4_500, 0, 250, 5, correction).is_err());
        assert!(MovementPredictionProfile::new(4_500, 20, 0, 5, correction).is_err());
        assert!(MovementPredictionProfile::new(4_500, 20, 250, 0, correction).is_err());
        Ok(())
    }

    #[test]
    fn presentation_correction_profile_rejects_invalid_threshold_order() {
        assert!(PresentationCorrectionProfile::new(250, 25, 1_000, 120, 50).is_err());
        assert!(PresentationCorrectionProfile::new(25, 1_000, 250, 120, 50).is_err());
        assert!(PresentationCorrectionProfile::new(25, 250, 1_000, 0, 50).is_err());
        assert!(PresentationCorrectionProfile::new(25, 250, 1_000, 120, 0).is_err());
    }
}
