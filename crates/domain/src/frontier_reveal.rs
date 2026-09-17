use std::{error::Error, fmt};

use aurenfall_core::QuadrantCoord;

use crate::{
    CandidateFrontierDerivation, CandidateFrontierError, FrontierEligibilityError,
    FrontierEligibilityEvaluation, FrontierLifecycleError, FrontierLifecycleTransition,
    FrontierQuadrantState, FrontierRequirementResolver, FrontierState, FrontierUnlockRequirements,
    derive_candidate_frontier, evaluate_frontier_eligibility,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoritativeRevealOutcome {
    Blocked {
        evaluation: FrontierEligibilityEvaluation,
    },
    AlreadyRevealed {
        coord: QuadrantCoord,
        revision: u64,
    },
    Revealed {
        transition: FrontierLifecycleTransition,
        candidate_derivation: CandidateFrontierDerivation,
    },
}

pub fn commit_authoritative_reveal(
    state: &mut FrontierState,
    requirements: &FrontierUnlockRequirements,
    resolver: &impl FrontierRequirementResolver,
) -> Result<AuthoritativeRevealOutcome, AuthoritativeRevealError> {
    let coord = requirements.coord();
    match state.state(coord) {
        FrontierQuadrantState::Revealed => {
            return Ok(AuthoritativeRevealOutcome::AlreadyRevealed {
                coord,
                revision: state.revision(),
            });
        }
        FrontierQuadrantState::Prepared => {}
        actual => {
            return Err(AuthoritativeRevealError::NotPrepared { coord, actual });
        }
    }

    let evaluation = evaluate_frontier_eligibility(requirements, resolver)?;
    if !evaluation.is_eligible() {
        return Ok(AuthoritativeRevealOutcome::Blocked { evaluation });
    }

    let mut staged = state.clone();
    let transition = staged
        .reveal(coord)?
        .ok_or(AuthoritativeRevealError::MissingRevealTransition { coord })?;
    let candidate_derivation = derive_candidate_frontier(&mut staged)?;

    *state = staged;
    Ok(AuthoritativeRevealOutcome::Revealed {
        transition,
        candidate_derivation,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoritativeRevealError {
    NotPrepared {
        coord: QuadrantCoord,
        actual: FrontierQuadrantState,
    },
    MissingRevealTransition {
        coord: QuadrantCoord,
    },
    Eligibility(FrontierEligibilityError),
    Lifecycle(FrontierLifecycleError),
    CandidateFrontier(CandidateFrontierError),
}

impl fmt::Display for AuthoritativeRevealError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrepared { coord, actual } => write!(
                formatter,
                "frontier reveal requires Prepared state at ({}, {}), found {actual}",
                coord.x(),
                coord.y()
            ),
            Self::MissingRevealTransition { coord } => write!(
                formatter,
                "frontier reveal at ({}, {}) produced no lifecycle transition after Prepared precondition",
                coord.x(),
                coord.y()
            ),
            Self::Eligibility(error) => write!(formatter, "frontier reveal eligibility failed: {error}"),
            Self::Lifecycle(error) => write!(formatter, "frontier reveal lifecycle failed: {error}"),
            Self::CandidateFrontier(error) => {
                write!(formatter, "frontier reveal candidate derivation failed: {error}")
            }
        }
    }
}

impl Error for AuthoritativeRevealError {}

impl From<FrontierEligibilityError> for AuthoritativeRevealError {
    fn from(value: FrontierEligibilityError) -> Self {
        Self::Eligibility(value)
    }
}

impl From<FrontierLifecycleError> for AuthoritativeRevealError {
    fn from(value: FrontierLifecycleError) -> Self {
        Self::Lifecycle(value)
    }
}

impl From<CandidateFrontierError> for AuthoritativeRevealError {
    fn from(value: CandidateFrontierError) -> Self {
        Self::CandidateFrontier(value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{FrontierRequirementId, InitialFrontier};

    #[derive(Default)]
    struct TestResolver {
        facts: HashMap<(QuadrantCoord, FrontierRequirementId), bool>,
    }

    impl TestResolver {
        fn with_fact(
            mut self,
            coord: QuadrantCoord,
            requirement: FrontierRequirementId,
            satisfied: bool,
        ) -> Self {
            self.facts.insert((coord, requirement), satisfied);
            self
        }
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

    fn requirement(value: u32) -> Result<FrontierRequirementId, FrontierEligibilityError> {
        FrontierRequirementId::new(value)
    }

    fn prepared_fixture(coord: QuadrantCoord, revision: u64) -> Result<FrontierState, Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        let mut state = FrontierState::from_initial(&initial, revision)?;
        derive_candidate_frontier(&mut state)?;
        state.mark_prepared(coord)?;
        Ok(state)
    }

    #[test]
    fn reveal_requires_prepared_state_even_when_requirement_is_satisfied() -> Result<(), Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        let mut state = FrontierState::from_initial(&initial, 1)?;
        derive_candidate_frontier(&mut state)?;
        let coord = QuadrantCoord::new(3, 0);
        let requirement = requirement(1)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![requirement])?;
        let resolver = TestResolver::default().with_fact(coord, requirement, true);

        assert_eq!(
            commit_authoritative_reveal(&mut state, &requirements, &resolver),
            Err(AuthoritativeRevealError::NotPrepared {
                coord,
                actual: FrontierQuadrantState::Candidate,
            })
        );
        assert_eq!(state.state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn blocked_eligibility_does_not_mutate_prepared_frontier() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let mut state = prepared_fixture(coord, 1)?;
        let requirement = requirement(1)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![requirement])?;
        let resolver = TestResolver::default().with_fact(coord, requirement, false);

        let outcome = commit_authoritative_reveal(&mut state, &requirements, &resolver)?;
        let AuthoritativeRevealOutcome::Blocked { evaluation } = outcome else {
            return Err("expected blocked authoritative reveal".into());
        };
        assert_eq!(evaluation.unsatisfied_requirements(), &[requirement]);
        assert_eq!(state.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn unknown_requirement_fails_closed_without_mutation() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let mut state = prepared_fixture(coord, 1)?;
        let requirement = requirement(9)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![requirement])?;

        assert_eq!(
            commit_authoritative_reveal(&mut state, &requirements, &TestResolver::default()),
            Err(AuthoritativeRevealError::Eligibility(
                FrontierEligibilityError::UnknownRequirement { coord, requirement }
            ))
        );
        assert_eq!(state.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn eligible_prepared_reveal_commits_revision_and_candidate_refresh_atomically()
    -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let mut state = prepared_fixture(coord, 1)?;
        let requirement = requirement(7)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![requirement])?;
        let resolver = TestResolver::default().with_fact(coord, requirement, true);

        let outcome = commit_authoritative_reveal(&mut state, &requirements, &resolver)?;
        let AuthoritativeRevealOutcome::Revealed {
            transition,
            candidate_derivation,
        } = outcome
        else {
            return Err("expected successful authoritative reveal".into());
        };

        assert_eq!(transition.coord(), coord);
        assert_eq!(transition.from(), FrontierQuadrantState::Prepared);
        assert_eq!(transition.to(), FrontierQuadrantState::Revealed);
        assert_eq!(transition.previous_revision(), 1);
        assert_eq!(transition.revision(), 2);
        assert_eq!(
            candidate_derivation.newly_candidates(),
            &[QuadrantCoord::new(4, 0)]
        );
        assert_eq!(state.state(coord), FrontierQuadrantState::Revealed);
        assert_eq!(
            state.state(QuadrantCoord::new(4, 0)),
            FrontierQuadrantState::Candidate
        );
        assert_eq!(state.revision(), 2);
        assert_eq!(state.len(), 17);
        Ok(())
    }

    #[test]
    fn repeated_authoritative_reveal_is_idempotent_without_resolving_requirements_again()
    -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let mut state = prepared_fixture(coord, 1)?;
        let requirement = requirement(5)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![requirement])?;
        let resolver = TestResolver::default().with_fact(coord, requirement, true);
        let _first = commit_authoritative_reveal(&mut state, &requirements, &resolver)?;

        let repeated = commit_authoritative_reveal(&mut state, &requirements, &TestResolver::default())?;
        assert_eq!(
            repeated,
            AuthoritativeRevealOutcome::AlreadyRevealed { coord, revision: 2 }
        );
        assert_eq!(state.revision(), 2);
        assert_eq!(state.len(), 17);
        Ok(())
    }

    #[test]
    fn candidate_refresh_failure_rolls_back_entire_public_reveal() -> Result<(), Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(i64::MAX - 1, 0), 1, 1)?;
        let mut state = FrontierState::from_initial(&initial, 9)?;
        derive_candidate_frontier(&mut state)?;
        let coord = QuadrantCoord::new(i64::MAX, 0);
        state.mark_prepared(coord)?;
        let requirement = requirement(1)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![requirement])?;
        let resolver = TestResolver::default().with_fact(coord, requirement, true);

        assert!(matches!(
            commit_authoritative_reveal(&mut state, &requirements, &resolver),
            Err(AuthoritativeRevealError::CandidateFrontier(
                CandidateFrontierError::QuadrantCoordinateOverflow {
                    coord: overflow_coord,
                    axis: "x",
                    delta: 1,
                }
            )) if overflow_coord == coord
        ));
        assert_eq!(state.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(state.revision(), 9);
        assert_eq!(state.len(), 1);
        Ok(())
    }
}
