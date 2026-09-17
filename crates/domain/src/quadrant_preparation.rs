use std::{collections::HashMap, error::Error, fmt};

use aurenfall_core::QuadrantCoord;

use crate::{FrontierLifecycleError, FrontierLifecycleTransition, FrontierQuadrantState, FrontierState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreparationFingerprint([u8; 32]);

impl PreparationFingerprint {
    pub fn new(bytes: [u8; 32]) -> Result<Self, QuadrantPreparationError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(QuadrantPreparationError::ZeroFingerprint);
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuadrantPreparationIdentity {
    coord: QuadrantCoord,
    generator_version: u32,
    generation_inputs_fingerprint: PreparationFingerprint,
}

impl QuadrantPreparationIdentity {
    pub fn new(
        coord: QuadrantCoord,
        generator_version: u32,
        generation_inputs_fingerprint: PreparationFingerprint,
    ) -> Result<Self, QuadrantPreparationError> {
        if generator_version == 0 {
            return Err(QuadrantPreparationError::ZeroGeneratorVersion);
        }
        Ok(Self {
            coord,
            generator_version,
            generation_inputs_fingerprint,
        })
    }

    #[must_use]
    pub const fn coord(self) -> QuadrantCoord {
        self.coord
    }

    #[must_use]
    pub const fn generator_version(self) -> u32 {
        self.generator_version
    }

    #[must_use]
    pub const fn generation_inputs_fingerprint(self) -> PreparationFingerprint {
        self.generation_inputs_fingerprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedQuadrantArtifact {
    identity: QuadrantPreparationIdentity,
    artifact_fingerprint: PreparationFingerprint,
    artifact_bytes: u64,
}

impl PreparedQuadrantArtifact {
    pub fn new(
        identity: QuadrantPreparationIdentity,
        artifact_fingerprint: PreparationFingerprint,
        artifact_bytes: u64,
    ) -> Result<Self, QuadrantPreparationError> {
        if artifact_bytes == 0 {
            return Err(QuadrantPreparationError::EmptyArtifact);
        }
        Ok(Self {
            identity,
            artifact_fingerprint,
            artifact_bytes,
        })
    }

    #[must_use]
    pub const fn identity(self) -> QuadrantPreparationIdentity {
        self.identity
    }

    #[must_use]
    pub const fn artifact_fingerprint(self) -> PreparationFingerprint {
        self.artifact_fingerprint
    }

    #[must_use]
    pub const fn artifact_bytes(self) -> u64 {
        self.artifact_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadrantPreparationRequestOutcome {
    Started,
    AlreadyInFlight,
    AlreadyPrepared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadrantPreparationCompletionOutcome {
    Prepared { transition: FrontierLifecycleTransition },
    AlreadyPrepared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadrantPreparationAbandonOutcome {
    Removed,
    NotFound,
    AlreadyPrepared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuadrantPreparationSlot {
    InFlight(QuadrantPreparationIdentity),
    Prepared(PreparedQuadrantArtifact),
}

impl QuadrantPreparationSlot {
    const fn identity(self) -> QuadrantPreparationIdentity {
        match self {
            Self::InFlight(identity) => identity,
            Self::Prepared(artifact) => artifact.identity(),
        }
    }
}

#[derive(Debug, Default)]
pub struct QuadrantPreparationState {
    slots: HashMap<QuadrantCoord, QuadrantPreparationSlot>,
}

impl QuadrantPreparationState {
    #[must_use]
    pub fn in_flight_len(&self) -> usize {
        self.slots
            .values()
            .filter(|slot| matches!(**slot, QuadrantPreparationSlot::InFlight(_)))
            .count()
    }

    pub fn request(
        &mut self,
        frontier: &FrontierState,
        identity: QuadrantPreparationIdentity,
    ) -> Result<QuadrantPreparationRequestOutcome, QuadrantPreparationError> {
        let coord = identity.coord();
        if let Some(slot) = self.slots.get(&coord).copied() {
            if slot.identity() != identity {
                return Err(QuadrantPreparationError::ConflictingIdentity { coord });
            }
            return Ok(match slot {
                QuadrantPreparationSlot::InFlight(_) => QuadrantPreparationRequestOutcome::AlreadyInFlight,
                QuadrantPreparationSlot::Prepared(_) => QuadrantPreparationRequestOutcome::AlreadyPrepared,
            });
        }

        let actual = frontier.state(coord);
        if actual != FrontierQuadrantState::Candidate {
            return Err(QuadrantPreparationError::FrontierNotCandidate { coord, actual });
        }

        self.slots
            .insert(coord, QuadrantPreparationSlot::InFlight(identity));
        Ok(QuadrantPreparationRequestOutcome::Started)
    }

    pub fn complete(
        &mut self,
        frontier: &mut FrontierState,
        artifact: PreparedQuadrantArtifact,
    ) -> Result<QuadrantPreparationCompletionOutcome, QuadrantPreparationError> {
        let identity = artifact.identity();
        let coord = identity.coord();
        let Some(slot) = self.slots.get(&coord).copied() else {
            return Err(QuadrantPreparationError::CompletionWithoutRequest { coord });
        };
        if slot.identity() != identity {
            return Err(QuadrantPreparationError::ConflictingIdentity { coord });
        }

        if let QuadrantPreparationSlot::Prepared(existing) = slot {
            if existing == artifact {
                return Ok(QuadrantPreparationCompletionOutcome::AlreadyPrepared);
            }
            return Err(QuadrantPreparationError::ConflictingPreparedArtifact { coord });
        }

        let actual = frontier.state(coord);
        if actual != FrontierQuadrantState::Candidate {
            return Err(QuadrantPreparationError::FrontierNotCandidate { coord, actual });
        }

        let transition = frontier
            .mark_prepared(coord)?
            .ok_or(QuadrantPreparationError::MissingPreparedTransition { coord })?;
        self.slots
            .insert(coord, QuadrantPreparationSlot::Prepared(artifact));
        Ok(QuadrantPreparationCompletionOutcome::Prepared { transition })
    }

    pub fn cancel(
        &mut self,
        identity: QuadrantPreparationIdentity,
    ) -> Result<QuadrantPreparationAbandonOutcome, QuadrantPreparationError> {
        self.abandon(identity)
    }

    pub fn fail(
        &mut self,
        identity: QuadrantPreparationIdentity,
    ) -> Result<QuadrantPreparationAbandonOutcome, QuadrantPreparationError> {
        self.abandon(identity)
    }

    #[must_use]
    pub fn prepared_artifact(
        &self,
        identity: QuadrantPreparationIdentity,
    ) -> Option<PreparedQuadrantArtifact> {
        self.slots
            .get(&identity.coord())
            .copied()
            .and_then(|slot| match slot {
                QuadrantPreparationSlot::Prepared(artifact) if artifact.identity() == identity => {
                    Some(artifact)
                }
                _ => None,
            })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn abandon(
        &mut self,
        identity: QuadrantPreparationIdentity,
    ) -> Result<QuadrantPreparationAbandonOutcome, QuadrantPreparationError> {
        let coord = identity.coord();
        let Some(slot) = self.slots.get(&coord).copied() else {
            return Ok(QuadrantPreparationAbandonOutcome::NotFound);
        };
        if slot.identity() != identity {
            return Err(QuadrantPreparationError::ConflictingIdentity { coord });
        }
        if matches!(slot, QuadrantPreparationSlot::Prepared(_)) {
            return Ok(QuadrantPreparationAbandonOutcome::AlreadyPrepared);
        }
        self.slots.remove(&coord);
        Ok(QuadrantPreparationAbandonOutcome::Removed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadrantPreparationError {
    ZeroGeneratorVersion,
    ZeroFingerprint,
    EmptyArtifact,
    FrontierNotCandidate {
        coord: QuadrantCoord,
        actual: FrontierQuadrantState,
    },
    ConflictingIdentity {
        coord: QuadrantCoord,
    },
    CompletionWithoutRequest {
        coord: QuadrantCoord,
    },
    ConflictingPreparedArtifact {
        coord: QuadrantCoord,
    },
    MissingPreparedTransition {
        coord: QuadrantCoord,
    },
    Lifecycle(FrontierLifecycleError),
}

impl fmt::Display for QuadrantPreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroGeneratorVersion => {
                write!(
                    formatter,
                    "quadrant preparation generator_version must be non-zero"
                )
            }
            Self::ZeroFingerprint => {
                write!(
                    formatter,
                    "quadrant preparation fingerprint must not be all zeroes"
                )
            }
            Self::EmptyArtifact => {
                write!(
                    formatter,
                    "prepared quadrant artifact must contain at least one byte"
                )
            }
            Self::FrontierNotCandidate { coord, actual } => write!(
                formatter,
                "quadrant preparation requires Candidate at ({}, {}), found {actual}",
                coord.x(),
                coord.y()
            ),
            Self::ConflictingIdentity { coord, .. } => write!(
                formatter,
                "conflicting quadrant preparation identity at ({}, {})",
                coord.x(),
                coord.y()
            ),
            Self::CompletionWithoutRequest { coord } => write!(
                formatter,
                "quadrant preparation completion has no active request at ({}, {})",
                coord.x(),
                coord.y()
            ),
            Self::ConflictingPreparedArtifact { coord } => write!(
                formatter,
                "quadrant preparation produced conflicting artifacts at ({}, {})",
                coord.x(),
                coord.y()
            ),
            Self::MissingPreparedTransition { coord } => write!(
                formatter,
                "quadrant preparation did not produce Candidate -> Prepared at ({}, {})",
                coord.x(),
                coord.y()
            ),
            Self::Lifecycle(error) => write!(formatter, "quadrant preparation lifecycle error: {error}"),
        }
    }
}

impl Error for QuadrantPreparationError {}

impl From<FrontierLifecycleError> for QuadrantPreparationError {
    fn from(value: FrontierLifecycleError) -> Self {
        Self::Lifecycle(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InitialFrontier, derive_candidate_frontier};

    fn frontier_fixture() -> Result<FrontierState, Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        let mut frontier = FrontierState::from_initial(&initial, 1)?;
        derive_candidate_frontier(&mut frontier)?;
        Ok(frontier)
    }

    fn fingerprint(value: u8) -> Result<PreparationFingerprint, QuadrantPreparationError> {
        PreparationFingerprint::new([value; 32])
    }

    fn identity(
        coord: QuadrantCoord,
        input: u8,
    ) -> Result<QuadrantPreparationIdentity, QuadrantPreparationError> {
        QuadrantPreparationIdentity::new(coord, 7, fingerprint(input)?)
    }

    fn artifact(
        identity: QuadrantPreparationIdentity,
        output: u8,
    ) -> Result<PreparedQuadrantArtifact, QuadrantPreparationError> {
        PreparedQuadrantArtifact::new(identity, fingerprint(output)?, 128)
    }

    #[test]
    fn identity_and_artifact_validation_fail_closed() -> Result<(), Box<dyn Error>> {
        assert!(matches!(
            PreparationFingerprint::new([0; 32]),
            Err(QuadrantPreparationError::ZeroFingerprint)
        ));
        assert!(matches!(
            QuadrantPreparationIdentity::new(QuadrantCoord::new(3, 0), 0, fingerprint(1)?,),
            Err(QuadrantPreparationError::ZeroGeneratorVersion)
        ));
        let valid_identity = identity(QuadrantCoord::new(3, 0), 1)?;
        assert!(matches!(
            PreparedQuadrantArtifact::new(valid_identity, fingerprint(2)?, 0),
            Err(QuadrantPreparationError::EmptyArtifact)
        ));
        Ok(())
    }

    #[test]
    fn duplicate_request_is_idempotent_and_single_slot() -> Result<(), Box<dyn Error>> {
        let frontier = frontier_fixture()?;
        let mut preparations = QuadrantPreparationState::default();
        let identity = identity(QuadrantCoord::new(3, 0), 1)?;

        assert_eq!(
            preparations.request(&frontier, identity)?,
            QuadrantPreparationRequestOutcome::Started
        );
        assert_eq!(
            preparations.request(&frontier, identity)?,
            QuadrantPreparationRequestOutcome::AlreadyInFlight
        );
        assert_eq!(preparations.len(), 1);
        assert!(!preparations.is_empty());
        Ok(())
    }

    #[test]
    fn conflicting_identity_for_same_quadrant_is_rejected() -> Result<(), Box<dyn Error>> {
        let frontier = frontier_fixture()?;
        let mut preparations = QuadrantPreparationState::default();
        let first = identity(QuadrantCoord::new(3, 0), 1)?;
        let second = identity(QuadrantCoord::new(3, 0), 2)?;

        preparations.request(&frontier, first)?;
        assert!(matches!(
            preparations.request(&frontier, second),
            Err(QuadrantPreparationError::ConflictingIdentity { coord, .. })
                if coord == QuadrantCoord::new(3, 0)
        ));
        assert_eq!(preparations.len(), 1);
        Ok(())
    }

    #[test]
    fn validated_completion_is_the_only_registry_path_to_prepared() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let mut frontier = frontier_fixture()?;
        let mut preparations = QuadrantPreparationState::default();
        let identity = identity(coord, 1)?;
        let artifact = artifact(identity, 2)?;

        preparations.request(&frontier, identity)?;
        assert_eq!(frontier.state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(frontier.revision(), 1);

        let outcome = preparations.complete(&mut frontier, artifact)?;
        assert!(matches!(
            outcome,
            QuadrantPreparationCompletionOutcome::Prepared { transition }
                if transition.from() == FrontierQuadrantState::Candidate
                    && transition.to() == FrontierQuadrantState::Prepared
                    && !transition.advances_public_revision()
        ));
        assert_eq!(frontier.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(frontier.revision(), 1);
        assert_eq!(preparations.prepared_artifact(identity), Some(artifact));

        assert_eq!(
            preparations.complete(&mut frontier, artifact)?,
            QuadrantPreparationCompletionOutcome::AlreadyPrepared
        );
        assert_eq!(preparations.len(), 1);
        Ok(())
    }

    #[test]
    fn conflicting_duplicate_completion_is_rejected_without_mutation() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let mut frontier = frontier_fixture()?;
        let mut preparations = QuadrantPreparationState::default();
        let identity = identity(coord, 1)?;
        let first = artifact(identity, 2)?;
        let second = artifact(identity, 3)?;

        preparations.request(&frontier, identity)?;
        preparations.complete(&mut frontier, first)?;
        assert!(matches!(
            preparations.complete(&mut frontier, second),
            Err(QuadrantPreparationError::ConflictingPreparedArtifact { coord: error_coord })
                if error_coord == coord
        ));
        assert_eq!(frontier.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(frontier.revision(), 1);
        assert_eq!(preparations.prepared_artifact(identity), Some(first));
        Ok(())
    }

    #[test]
    fn failure_and_cancel_remove_only_in_flight_private_state() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let frontier = frontier_fixture()?;
        let mut preparations = QuadrantPreparationState::default();
        let identity = identity(coord, 1)?;

        preparations.request(&frontier, identity)?;
        assert_eq!(
            preparations.fail(identity)?,
            QuadrantPreparationAbandonOutcome::Removed
        );
        assert!(preparations.is_empty());
        assert_eq!(frontier.state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(frontier.revision(), 1);

        preparations.request(&frontier, identity)?;
        assert_eq!(
            preparations.cancel(identity)?,
            QuadrantPreparationAbandonOutcome::Removed
        );
        assert_eq!(
            preparations.cancel(identity)?,
            QuadrantPreparationAbandonOutcome::NotFound
        );
        assert_eq!(frontier.state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(frontier.revision(), 1);
        Ok(())
    }

    #[test]
    fn completion_without_request_is_non_mutating() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let mut frontier = frontier_fixture()?;
        let mut preparations = QuadrantPreparationState::default();
        let identity = identity(coord, 1)?;
        let artifact = artifact(identity, 2)?;

        assert!(matches!(
            preparations.complete(&mut frontier, artifact),
            Err(QuadrantPreparationError::CompletionWithoutRequest { coord: error_coord })
                if error_coord == coord
        ));
        assert_eq!(frontier.state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(frontier.revision(), 1);
        assert!(preparations.is_empty());
        Ok(())
    }
}
