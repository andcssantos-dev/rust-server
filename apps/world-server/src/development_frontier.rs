use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use aurenfall_core::QuadrantCoord;
use aurenfall_domain::{
    FrontierRequirementId, FrontierRequirementResolver, FrontierUnlockRequirements, PreparationFingerprint,
    PreparedQuadrantArtifact,
};

use crate::frontier_runtime::FrontierRuntime;

const DEVELOPMENT_FRONTIER_REQUIREMENT_ID: u32 = 1;
const DEVELOPMENT_PREPARATION_INPUT_FINGERPRINT: [u8; 32] = [0xD1; 32];
const DEVELOPMENT_PREPARED_ARTIFACT_FINGERPRINT: [u8; 32] = [0xD2; 32];
const DEVELOPMENT_PREPARED_ARTIFACT_BYTES: u64 = 1;

#[derive(Debug, Clone, Copy)]
pub struct DevelopmentFrontierRevealConfig {
    coord: QuadrantCoord,
    delay: Duration,
}

impl DevelopmentFrontierRevealConfig {
    pub fn new(coord: QuadrantCoord, delay: Duration) -> anyhow::Result<Self> {
        if delay.is_zero() {
            bail!("development frontier reveal delay must be greater than zero");
        }
        Ok(Self { coord, delay })
    }

    #[must_use]
    pub const fn coord(self) -> QuadrantCoord {
        self.coord
    }

    #[must_use]
    pub const fn delay(self) -> Duration {
        self.delay
    }
}

#[derive(Debug)]
pub(crate) struct DevelopmentFrontierRevealGate {
    requirements: FrontierUnlockRequirements,
    requirement: FrontierRequirementId,
    delay: Duration,
    armed_at: Option<Instant>,
    complete: bool,
}

impl DevelopmentFrontierRevealGate {
    pub(crate) fn new(
        frontier: &mut FrontierRuntime,
        config: DevelopmentFrontierRevealConfig,
    ) -> anyhow::Result<Self> {
        let generation_inputs_fingerprint =
            PreparationFingerprint::new(DEVELOPMENT_PREPARATION_INPUT_FINGERPRINT)
                .context("development preparation input fingerprint must be valid")?;
        let identity = frontier
            .preparation_identity(config.coord(), generation_inputs_fingerprint)
            .context("failed to build development frontier preparation identity")?;
        frontier
            .request_preparation(identity)
            .context("failed to register development frontier preparation")?;

        let artifact_fingerprint = PreparationFingerprint::new(DEVELOPMENT_PREPARED_ARTIFACT_FINGERPRINT)
            .context("development prepared artifact fingerprint must be valid")?;
        let artifact = PreparedQuadrantArtifact::new(
            identity,
            artifact_fingerprint,
            DEVELOPMENT_PREPARED_ARTIFACT_BYTES,
        )
        .context("failed to build development frontier prepared artifact")?;
        frontier
            .complete_preparation(artifact)
            .context("failed to commit development frontier prepared artifact")?;

        let requirement = FrontierRequirementId::new(DEVELOPMENT_FRONTIER_REQUIREMENT_ID)
            .context("development frontier requirement id must be valid")?;
        let requirements = FrontierUnlockRequirements::new(config.coord(), vec![requirement])
            .context("failed to build development frontier unlock requirements")?;
        Ok(Self {
            requirements,
            requirement,
            delay: config.delay(),
            armed_at: None,
            complete: false,
        })
    }

    pub(crate) fn arm(&mut self, now: Instant) -> bool {
        if self.armed_at.is_some() || self.complete {
            return false;
        }
        self.armed_at = Some(now);
        true
    }

    #[must_use]
    pub(crate) fn is_ready(&self, now: Instant) -> bool {
        self.armed_at
            .is_some_and(|armed_at| now.saturating_duration_since(armed_at) >= self.delay)
    }

    #[must_use]
    pub(crate) const fn is_complete(&self) -> bool {
        self.complete
    }

    pub(crate) fn mark_complete(&mut self) {
        self.complete = true;
    }

    #[must_use]
    pub(crate) fn requirements(&self) -> &FrontierUnlockRequirements {
        &self.requirements
    }

    #[must_use]
    pub(crate) const fn coord(&self) -> QuadrantCoord {
        self.requirements.coord()
    }

    #[must_use]
    pub(crate) const fn delay(&self) -> Duration {
        self.delay
    }

    #[must_use]
    pub(crate) const fn resolver(&self, satisfied: bool) -> DevelopmentFrontierRequirementResolver {
        DevelopmentFrontierRequirementResolver {
            coord: self.requirements.coord(),
            requirement: self.requirement,
            satisfied,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DevelopmentFrontierRequirementResolver {
    coord: QuadrantCoord,
    requirement: FrontierRequirementId,
    satisfied: bool,
}

impl FrontierRequirementResolver for DevelopmentFrontierRequirementResolver {
    fn resolve_requirement(&self, coord: QuadrantCoord, requirement: FrontierRequirementId) -> Option<bool> {
        (coord == self.coord && requirement == self.requirement).then_some(self.satisfied)
    }
}

#[cfg(test)]
mod tests {
    use aurenfall_domain::{FrontierQuadrantState, InitialFrontier};

    use super::*;

    fn runtime_fixture() -> anyhow::Result<FrontierRuntime> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        FrontierRuntime::new(&initial, 1, 7, 512_000)
    }

    #[test]
    fn development_gate_prepares_candidate_without_public_revision() -> anyhow::Result<()> {
        let coord = QuadrantCoord::new(3, 0);
        let mut frontier = runtime_fixture()?;
        let gate = DevelopmentFrontierRevealGate::new(
            &mut frontier,
            DevelopmentFrontierRevealConfig::new(coord, Duration::from_secs(5))?,
        )?;

        assert_eq!(frontier.quadrant_state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(frontier.revision(), 1);
        assert_eq!(frontier.revealed_len(), 16);
        assert_eq!(gate.coord(), coord);
        assert!(!gate.is_complete());
        Ok(())
    }

    #[test]
    fn development_gate_is_server_time_driven_and_one_shot() -> anyhow::Result<()> {
        let coord = QuadrantCoord::new(3, 0);
        let mut frontier = runtime_fixture()?;
        let mut gate = DevelopmentFrontierRevealGate::new(
            &mut frontier,
            DevelopmentFrontierRevealConfig::new(coord, Duration::from_secs(5))?,
        )?;
        let started = Instant::now();

        assert!(gate.arm(started));
        assert!(!gate.arm(started));
        assert!(!gate.is_ready(started + Duration::from_secs(4)));
        assert!(gate.is_ready(started + Duration::from_secs(5)));
        gate.mark_complete();
        assert!(gate.is_complete());
        Ok(())
    }
}
