use std::{error::Error, fmt};

use aurenfall_core::QuadrantCoord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrontierRequirementId(u32);

impl FrontierRequirementId {
    pub fn new(value: u32) -> Result<Self, FrontierEligibilityError> {
        if value == 0 {
            return Err(FrontierEligibilityError::ZeroRequirementId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierUnlockRequirements {
    coord: QuadrantCoord,
    requirements: Vec<FrontierRequirementId>,
}

impl FrontierUnlockRequirements {
    pub fn new(
        coord: QuadrantCoord,
        mut requirements: Vec<FrontierRequirementId>,
    ) -> Result<Self, FrontierEligibilityError> {
        if requirements.is_empty() {
            return Err(FrontierEligibilityError::EmptyRequirementSet { coord });
        }

        requirements.sort_unstable();
        if let Some(pair) = requirements.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(FrontierEligibilityError::DuplicateRequirement {
                coord,
                requirement: pair[0],
            });
        }

        Ok(Self { coord, requirements })
    }

    #[must_use]
    pub const fn coord(&self) -> QuadrantCoord {
        self.coord
    }

    #[must_use]
    pub fn requirements(&self) -> &[FrontierRequirementId] {
        &self.requirements
    }
}

pub trait FrontierRequirementResolver {
    fn resolve_requirement(&self, coord: QuadrantCoord, requirement: FrontierRequirementId) -> Option<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierEligibilityEvaluation {
    coord: QuadrantCoord,
    unsatisfied_requirements: Vec<FrontierRequirementId>,
}

impl FrontierEligibilityEvaluation {
    #[must_use]
    pub const fn coord(&self) -> QuadrantCoord {
        self.coord
    }

    #[must_use]
    pub fn is_eligible(&self) -> bool {
        self.unsatisfied_requirements.is_empty()
    }

    #[must_use]
    pub fn unsatisfied_requirements(&self) -> &[FrontierRequirementId] {
        &self.unsatisfied_requirements
    }
}

pub fn evaluate_frontier_eligibility(
    requirements: &FrontierUnlockRequirements,
    resolver: &impl FrontierRequirementResolver,
) -> Result<FrontierEligibilityEvaluation, FrontierEligibilityError> {
    let mut unsatisfied_requirements = Vec::new();

    for requirement in requirements.requirements() {
        match resolver.resolve_requirement(requirements.coord(), *requirement) {
            Some(true) => {}
            Some(false) => unsatisfied_requirements.push(*requirement),
            None => {
                return Err(FrontierEligibilityError::UnknownRequirement {
                    coord: requirements.coord(),
                    requirement: *requirement,
                });
            }
        }
    }

    Ok(FrontierEligibilityEvaluation {
        coord: requirements.coord(),
        unsatisfied_requirements,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierEligibilityError {
    ZeroRequirementId,
    EmptyRequirementSet {
        coord: QuadrantCoord,
    },
    DuplicateRequirement {
        coord: QuadrantCoord,
        requirement: FrontierRequirementId,
    },
    UnknownRequirement {
        coord: QuadrantCoord,
        requirement: FrontierRequirementId,
    },
}

impl fmt::Display for FrontierEligibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRequirementId => write!(formatter, "frontier requirement id must be non-zero"),
            Self::EmptyRequirementSet { coord } => write!(
                formatter,
                "frontier unlock requirements for ({}, {}) cannot be empty",
                coord.x(),
                coord.y()
            ),
            Self::DuplicateRequirement { coord, requirement } => write!(
                formatter,
                "frontier unlock requirements for ({}, {}) contain duplicate requirement {}",
                coord.x(),
                coord.y(),
                requirement.value()
            ),
            Self::UnknownRequirement { coord, requirement } => write!(
                formatter,
                "frontier requirement {} for ({}, {}) is unknown to the authoritative resolver",
                requirement.value(),
                coord.x(),
                coord.y()
            ),
        }
    }
}

impl Error for FrontierEligibilityError {}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

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

    #[test]
    fn requirement_ids_and_sets_fail_closed_on_invalid_configuration() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        assert_eq!(
            FrontierRequirementId::new(0),
            Err(FrontierEligibilityError::ZeroRequirementId)
        );
        assert_eq!(
            FrontierUnlockRequirements::new(coord, Vec::new()),
            Err(FrontierEligibilityError::EmptyRequirementSet { coord })
        );

        let requirement = requirement(7)?;
        assert_eq!(
            FrontierUnlockRequirements::new(coord, vec![requirement, requirement]),
            Err(FrontierEligibilityError::DuplicateRequirement { coord, requirement })
        );
        Ok(())
    }

    #[test]
    fn all_declared_requirements_must_be_satisfied() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(3, 0);
        let first = requirement(10)?;
        let second = requirement(20)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![second, first])?;
        let resolver = TestResolver::default()
            .with_fact(coord, first, true)
            .with_fact(coord, second, true);

        let evaluation = evaluate_frontier_eligibility(&requirements, &resolver)?;
        assert!(evaluation.is_eligible());
        assert!(evaluation.unsatisfied_requirements().is_empty());
        assert_eq!(evaluation.coord(), coord);
        assert_eq!(requirements.requirements(), &[first, second]);
        Ok(())
    }

    #[test]
    fn blocked_evaluation_reports_unsatisfied_requirements_deterministically() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(-8, 11);
        let first = requirement(1)?;
        let second = requirement(2)?;
        let third = requirement(3)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![third, first, second])?;
        let resolver = TestResolver::default()
            .with_fact(coord, first, false)
            .with_fact(coord, second, true)
            .with_fact(coord, third, false);

        let evaluation = evaluate_frontier_eligibility(&requirements, &resolver)?;
        assert!(!evaluation.is_eligible());
        assert_eq!(evaluation.unsatisfied_requirements(), &[first, third]);
        Ok(())
    }

    #[test]
    fn unknown_requirement_is_an_error_instead_of_implicit_success() -> Result<(), Box<dyn Error>> {
        let coord = QuadrantCoord::new(4, 4);
        let known = requirement(1)?;
        let unknown = requirement(2)?;
        let requirements = FrontierUnlockRequirements::new(coord, vec![known, unknown])?;
        let resolver = TestResolver::default().with_fact(coord, known, true);

        assert_eq!(
            evaluate_frontier_eligibility(&requirements, &resolver),
            Err(FrontierEligibilityError::UnknownRequirement {
                coord,
                requirement: unknown,
            })
        );
        Ok(())
    }

    #[test]
    fn requirement_resolution_is_scoped_to_the_target_quadrant() -> Result<(), Box<dyn Error>> {
        let target = QuadrantCoord::new(3, 0);
        let other = QuadrantCoord::new(0, 3);
        let requirement = requirement(9)?;
        let requirements = FrontierUnlockRequirements::new(target, vec![requirement])?;
        let resolver = TestResolver::default()
            .with_fact(other, requirement, true)
            .with_fact(target, requirement, false);

        let evaluation = evaluate_frontier_eligibility(&requirements, &resolver)?;
        assert!(!evaluation.is_eligible());
        assert_eq!(evaluation.unsatisfied_requirements(), &[requirement]);
        Ok(())
    }
}
