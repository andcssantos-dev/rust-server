use anyhow::{Context, bail};
use aurenfall_contracts::{FrontierManifest, FrontierQuadrantCoord};
use aurenfall_core::QuadrantCoord;
use aurenfall_domain::{
    AuthoritativeRevealOutcome, FrontierQuadrantState, FrontierRequirementResolver, FrontierState,
    FrontierUnlockRequirements, InitialFrontier, PreparationFingerprint, PreparedQuadrantArtifact,
    QuadrantPreparationAbandonOutcome, QuadrantPreparationCompletionOutcome, QuadrantPreparationIdentity,
    QuadrantPreparationRequestOutcome, QuadrantPreparationState, commit_authoritative_reveal,
    derive_candidate_frontier,
};

#[derive(Debug)]
pub struct FrontierRuntime {
    state: FrontierState,
    preparations: QuadrantPreparationState,
    generator_version: u32,
    quadrant_size_mm: i64,
}

impl FrontierRuntime {
    pub fn new(
        initial: &InitialFrontier,
        revision: u64,
        generator_version: u32,
        quadrant_size_mm: i64,
    ) -> anyhow::Result<Self> {
        let mut state = FrontierState::from_initial(initial, revision)
            .context("failed to initialize authoritative frontier state")?;
        derive_candidate_frontier(&mut state).context("failed to derive initial candidate frontier")?;

        let runtime = Self {
            state,
            preparations: QuadrantPreparationState::default(),
            generator_version,
            quadrant_size_mm,
        };
        runtime
            .manifest()
            .context("initial frontier manifest is invalid")?;
        Ok(runtime)
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.state.revision()
    }

    #[must_use]
    pub fn revealed_len(&self) -> usize {
        self.state.len()
    }

    #[cfg(test)]
    #[must_use]
    pub const fn generator_version(&self) -> u32 {
        self.generator_version
    }

    #[cfg(test)]
    #[must_use]
    pub fn quadrant_state(&self, coord: QuadrantCoord) -> FrontierQuadrantState {
        self.state.state(coord)
    }

    #[must_use]
    pub fn candidate_quadrants(&self) -> Vec<QuadrantCoord> {
        self.state.quadrants_in_state(FrontierQuadrantState::Candidate)
    }

    #[must_use]
    pub fn prepared_len(&self) -> usize {
        self.state
            .quadrants_in_state(FrontierQuadrantState::Prepared)
            .len()
    }

    #[must_use]
    pub fn preparation_in_flight_len(&self) -> usize {
        self.preparations.in_flight_len()
    }

    pub fn preparation_identity(
        &self,
        coord: QuadrantCoord,
        generation_inputs_fingerprint: PreparationFingerprint,
    ) -> anyhow::Result<QuadrantPreparationIdentity> {
        QuadrantPreparationIdentity::new(coord, self.generator_version, generation_inputs_fingerprint)
            .context("failed to build authoritative quadrant preparation identity")
    }

    pub fn request_preparation(
        &mut self,
        identity: QuadrantPreparationIdentity,
    ) -> anyhow::Result<QuadrantPreparationRequestOutcome> {
        self.validate_preparation_generator(identity)?;
        self.preparations
            .request(&self.state, identity)
            .context("failed to register quadrant preparation request")
    }

    pub fn complete_preparation(
        &mut self,
        artifact: PreparedQuadrantArtifact,
    ) -> anyhow::Result<QuadrantPreparationCompletionOutcome> {
        self.validate_preparation_generator(artifact.identity())?;
        self.preparations
            .complete(&mut self.state, artifact)
            .context("failed to commit validated quadrant preparation artifact")
    }

    pub fn cancel_preparation(
        &mut self,
        identity: QuadrantPreparationIdentity,
    ) -> anyhow::Result<QuadrantPreparationAbandonOutcome> {
        self.validate_preparation_generator(identity)?;
        self.preparations
            .cancel(identity)
            .context("failed to cancel quadrant preparation")
    }

    pub fn fail_preparation(
        &mut self,
        identity: QuadrantPreparationIdentity,
    ) -> anyhow::Result<QuadrantPreparationAbandonOutcome> {
        self.validate_preparation_generator(identity)?;
        self.preparations
            .fail(identity)
            .context("failed to fail quadrant preparation")
    }

    pub fn commit_reveal(
        &mut self,
        requirements: &FrontierUnlockRequirements,
        resolver: &impl FrontierRequirementResolver,
    ) -> anyhow::Result<AuthoritativeRevealOutcome> {
        commit_authoritative_reveal(&mut self.state, requirements, resolver)
            .context("authoritative frontier reveal transaction failed")
    }

    pub fn manifest(&self) -> anyhow::Result<FrontierManifest> {
        build_manifest(&self.state, self.generator_version, self.quadrant_size_mm)
    }

    fn validate_preparation_generator(&self, identity: QuadrantPreparationIdentity) -> anyhow::Result<()> {
        if identity.generator_version() != self.generator_version {
            bail!(
                "quadrant preparation generator version mismatch at ({}, {}): identity={} runtime={}",
                identity.coord().x(),
                identity.coord().y(),
                identity.generator_version(),
                self.generator_version
            );
        }
        Ok(())
    }

    #[cfg(test)]
    fn state(&self) -> &FrontierState {
        &self.state
    }

    #[cfg(test)]
    pub(crate) fn prepared_artifact(
        &self,
        identity: QuadrantPreparationIdentity,
    ) -> Option<PreparedQuadrantArtifact> {
        self.preparations.prepared_artifact(identity)
    }
}

fn build_manifest(
    state: &FrontierState,
    generator_version: u32,
    quadrant_size_mm: i64,
) -> anyhow::Result<FrontierManifest> {
    let mut quadrants: Vec<_> = state
        .quadrants()
        .iter()
        .map(|coord| FrontierQuadrantCoord::new(coord.x(), coord.y()))
        .collect();
    quadrants.sort_unstable_by_key(|coord| (coord.y, coord.x));

    FrontierManifest::new(generator_version, quadrant_size_mm, state.revision(), quadrants)
        .context("authoritative frontier manifest is invalid")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use aurenfall_domain::{FrontierEligibilityError, FrontierRequirementId, FrontierUnlockRequirements};

    use super::*;

    #[derive(Default)]
    struct TestResolver {
        facts: HashMap<(QuadrantCoord, FrontierRequirementId), bool>,
    }

    impl FrontierRequirementResolver for TestResolver {
        fn resolve_requirement(
            &self,
            coord: QuadrantCoord,
            requirement: FrontierRequirementId,
        ) -> Option<bool> {
            self.facts.get(&(coord, requirement)).copied()
        }
    }

    fn runtime_fixture() -> anyhow::Result<FrontierRuntime> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        FrontierRuntime::new(&initial, 1, 7, 512_000)
    }

    fn requirement(value: u32) -> Result<FrontierRequirementId, FrontierEligibilityError> {
        FrontierRequirementId::new(value)
    }

    fn fingerprint(value: u8) -> anyhow::Result<PreparationFingerprint> {
        PreparationFingerprint::new([value; 32]).context("test preparation fingerprint must be valid")
    }

    fn prepare(
        runtime: &mut FrontierRuntime,
        coord: QuadrantCoord,
        input: u8,
        output: u8,
    ) -> anyhow::Result<QuadrantPreparationIdentity> {
        let identity = runtime.preparation_identity(coord, fingerprint(input)?)?;
        let requested = runtime.request_preparation(identity)?;
        assert!(matches!(
            requested,
            QuadrantPreparationRequestOutcome::Started | QuadrantPreparationRequestOutcome::AlreadyInFlight
        ));
        let artifact = PreparedQuadrantArtifact::new(identity, fingerprint(output)?, 128)?;
        let completed = runtime.complete_preparation(artifact)?;
        assert!(matches!(
            completed,
            QuadrantPreparationCompletionOutcome::Prepared { .. }
                | QuadrantPreparationCompletionOutcome::AlreadyPrepared
        ));
        Ok(identity)
    }

    #[test]
    fn initial_candidates_stay_private() -> anyhow::Result<()> {
        let runtime = runtime_fixture()?;
        let manifest = runtime.manifest()?;

        assert_eq!(runtime.revision(), 1);
        assert_eq!(runtime.revealed_len(), 16);
        assert_eq!(runtime.generator_version(), 7);
        assert_eq!(manifest.revision, 1);
        assert_eq!(manifest.quadrants.len(), 16);
        assert_eq!(
            runtime.state().state(QuadrantCoord::new(3, 0)),
            FrontierQuadrantState::Candidate
        );
        assert!(!manifest.quadrants.contains(&FrontierQuadrantCoord::new(3, 0)));
        assert_eq!(
            runtime.state().state(QuadrantCoord::new(3, 3)),
            FrontierQuadrantState::Potential
        );
        Ok(())
    }

    #[test]
    fn prepared_state_stays_private_and_requires_validated_artifact() -> anyhow::Result<()> {
        let mut runtime = runtime_fixture()?;
        let coord = QuadrantCoord::new(3, 0);
        let public_coord = FrontierQuadrantCoord::new(coord.x(), coord.y());
        let identity = prepare(&mut runtime, coord, 1, 2)?;

        let manifest = runtime.manifest()?;
        assert_eq!(runtime.state().state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(runtime.revision(), 1);
        assert_eq!(manifest.revision, 1);
        assert_eq!(manifest.quadrants.len(), 16);
        assert!(!manifest.quadrants.contains(&public_coord));
        assert!(runtime.prepared_artifact(identity).is_some());
        Ok(())
    }

    #[test]
    fn request_and_completion_are_idempotent() -> anyhow::Result<()> {
        let mut runtime = runtime_fixture()?;
        let coord = QuadrantCoord::new(3, 0);
        let identity = runtime.preparation_identity(coord, fingerprint(1)?)?;

        assert_eq!(
            runtime.request_preparation(identity)?,
            QuadrantPreparationRequestOutcome::Started
        );
        assert_eq!(
            runtime.request_preparation(identity)?,
            QuadrantPreparationRequestOutcome::AlreadyInFlight
        );
        let artifact = PreparedQuadrantArtifact::new(identity, fingerprint(2)?, 64)?;
        assert!(matches!(
            runtime.complete_preparation(artifact)?,
            QuadrantPreparationCompletionOutcome::Prepared { .. }
        ));
        assert_eq!(
            runtime.complete_preparation(artifact)?,
            QuadrantPreparationCompletionOutcome::AlreadyPrepared
        );
        assert_eq!(runtime.revision(), 1);
        assert_eq!(runtime.quadrant_state(coord), FrontierQuadrantState::Prepared);
        Ok(())
    }

    #[test]
    fn cancel_and_failure_leave_candidate_and_public_revision_unchanged() -> anyhow::Result<()> {
        let mut runtime = runtime_fixture()?;
        let coord = QuadrantCoord::new(3, 0);
        let identity = runtime.preparation_identity(coord, fingerprint(1)?)?;

        runtime.request_preparation(identity)?;
        assert_eq!(
            runtime.fail_preparation(identity)?,
            QuadrantPreparationAbandonOutcome::Removed
        );
        assert_eq!(runtime.quadrant_state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(runtime.revision(), 1);

        runtime.request_preparation(identity)?;
        assert_eq!(
            runtime.cancel_preparation(identity)?,
            QuadrantPreparationAbandonOutcome::Removed
        );
        assert_eq!(runtime.quadrant_state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(runtime.revision(), 1);
        Ok(())
    }

    #[test]
    fn runtime_commit_reveal_uses_authoritative_transaction() -> anyhow::Result<()> {
        let coord = QuadrantCoord::new(3, 0);
        let mut runtime = runtime_fixture()?;
        prepare(&mut runtime, coord, 1, 2)?;
        let requirement = requirement(7)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![requirement])?;
        let mut resolver = TestResolver::default();
        resolver.facts.insert((coord, requirement), true);

        let outcome = runtime.commit_reveal(&requirements, &resolver)?;
        assert!(matches!(outcome, AuthoritativeRevealOutcome::Revealed { .. }));
        assert_eq!(runtime.quadrant_state(coord), FrontierQuadrantState::Revealed);
        assert_eq!(runtime.revision(), 2);
        assert_eq!(runtime.revealed_len(), 17);
        Ok(())
    }

    #[test]
    fn manifest_order_ignores_reveal_history() -> anyhow::Result<()> {
        let east = QuadrantCoord::new(3, 0);
        let north = QuadrantCoord::new(0, 3);
        let mut east_then_north = runtime_fixture()?;
        let mut north_then_east = runtime_fixture()?;

        for coord in [east, north] {
            prepare(&mut east_then_north, coord, 1, 2)?;
            east_then_north.state.reveal(coord)?;
        }
        for coord in [north, east] {
            prepare(&mut north_then_east, coord, 1, 2)?;
            north_then_east.state.reveal(coord)?;
        }

        assert_eq!(east_then_north.revision(), 3);
        assert_eq!(north_then_east.revision(), 3);
        assert_eq!(east_then_north.manifest()?, north_then_east.manifest()?);
        Ok(())
    }
}
