use std::{collections::HashSet, error::Error, fmt};

use aurenfall_core::QuadrantCoord;

use crate::{FrontierLifecycleError, FrontierQuadrantState, FrontierState};

const CARDINAL_NEIGHBORS: [(i64, i64); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateFrontierDerivation {
    newly_candidates: Vec<QuadrantCoord>,
}

impl CandidateFrontierDerivation {
    #[must_use]
    pub fn newly_candidates(&self) -> &[QuadrantCoord] {
        &self.newly_candidates
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.newly_candidates.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.newly_candidates.is_empty()
    }
}

pub fn derive_candidate_frontier(
    state: &mut FrontierState,
) -> Result<CandidateFrontierDerivation, CandidateFrontierError> {
    let revealed = state.quadrants().to_vec();
    let mut seen = HashSet::new();
    let mut newly_candidates = Vec::new();

    for coord in revealed {
        for (delta_x, delta_y) in CARDINAL_NEIGHBORS {
            let neighbor = checked_neighbor(coord, delta_x, delta_y)?;
            if state.state(neighbor) == FrontierQuadrantState::Potential && seen.insert(neighbor) {
                newly_candidates.push(neighbor);
            }
        }
    }

    newly_candidates.sort_unstable_by_key(|coord| (coord.y(), coord.x()));

    for coord in &newly_candidates {
        let transition = state.mark_candidate(*coord)?;
        debug_assert!(transition.is_some());
    }

    Ok(CandidateFrontierDerivation { newly_candidates })
}

fn checked_neighbor(
    coord: QuadrantCoord,
    delta_x: i64,
    delta_y: i64,
) -> Result<QuadrantCoord, CandidateFrontierError> {
    let x = coord
        .x()
        .checked_add(delta_x)
        .ok_or(CandidateFrontierError::QuadrantCoordinateOverflow {
            coord,
            axis: "x",
            delta: delta_x,
        })?;
    let y = coord
        .y()
        .checked_add(delta_y)
        .ok_or(CandidateFrontierError::QuadrantCoordinateOverflow {
            coord,
            axis: "y",
            delta: delta_y,
        })?;
    Ok(QuadrantCoord::new(x, y))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateFrontierError {
    QuadrantCoordinateOverflow {
        coord: QuadrantCoord,
        axis: &'static str,
        delta: i64,
    },
    Lifecycle(FrontierLifecycleError),
}

impl fmt::Display for CandidateFrontierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuadrantCoordinateOverflow { coord, axis, delta } => write!(
                formatter,
                "candidate frontier neighbor overflow at ({}, {}) on {axis} delta {delta}",
                coord.x(),
                coord.y()
            ),
            Self::Lifecycle(error) => write!(formatter, "candidate frontier lifecycle error: {error}"),
        }
    }
}

impl Error for CandidateFrontierError {}

impl From<FrontierLifecycleError> for CandidateFrontierError {
    fn from(value: FrontierLifecycleError) -> Self {
        Self::Lifecycle(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InitialFrontier;

    fn frontier_fixture() -> Result<FrontierState, Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        Ok(FrontierState::from_initial(&initial, 1)?)
    }

    #[test]
    fn initial_revealed_frontier_derives_cardinal_candidate_ring() -> Result<(), Box<dyn Error>> {
        let mut state = frontier_fixture()?;
        let derived = derive_candidate_frontier(&mut state)?;

        assert_eq!(derived.len(), 16);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);

        for x in -1..=2 {
            assert_eq!(
                state.state(QuadrantCoord::new(x, -2)),
                FrontierQuadrantState::Candidate
            );
            assert_eq!(
                state.state(QuadrantCoord::new(x, 3)),
                FrontierQuadrantState::Candidate
            );
        }
        for y in -1..=2 {
            assert_eq!(
                state.state(QuadrantCoord::new(-2, y)),
                FrontierQuadrantState::Candidate
            );
            assert_eq!(
                state.state(QuadrantCoord::new(3, y)),
                FrontierQuadrantState::Candidate
            );
        }

        for coord in [
            QuadrantCoord::new(-2, -2),
            QuadrantCoord::new(3, -2),
            QuadrantCoord::new(-2, 3),
            QuadrantCoord::new(3, 3),
        ] {
            assert_eq!(state.state(coord), FrontierQuadrantState::Potential);
        }
        Ok(())
    }

    #[test]
    fn derivation_is_deterministic_and_idempotent() -> Result<(), Box<dyn Error>> {
        let mut first = frontier_fixture()?;
        let mut second = frontier_fixture()?;

        let first_pass = derive_candidate_frontier(&mut first)?;
        let second_pass = derive_candidate_frontier(&mut second)?;
        assert_eq!(first_pass, second_pass);
        assert_eq!(first_pass.len(), 16);

        let repeated = derive_candidate_frontier(&mut first)?;
        assert!(repeated.is_empty());
        assert_eq!(first.revision(), 1);
        assert_eq!(first.len(), 16);
        Ok(())
    }

    #[test]
    fn derivation_order_does_not_depend_on_reveal_history() -> Result<(), Box<dyn Error>> {
        let east = QuadrantCoord::new(3, 0);
        let north = QuadrantCoord::new(0, 3);
        let mut east_then_north = frontier_fixture()?;
        let mut north_then_east = frontier_fixture()?;

        derive_candidate_frontier(&mut east_then_north)?;
        east_then_north.mark_prepared(east)?;
        east_then_north.reveal(east)?;
        east_then_north.mark_prepared(north)?;
        east_then_north.reveal(north)?;

        derive_candidate_frontier(&mut north_then_east)?;
        north_then_east.mark_prepared(north)?;
        north_then_east.reveal(north)?;
        north_then_east.mark_prepared(east)?;
        north_then_east.reveal(east)?;

        let first = derive_candidate_frontier(&mut east_then_north)?;
        let second = derive_candidate_frontier(&mut north_then_east)?;
        assert_eq!(first, second);
        assert_eq!(
            first.newly_candidates(),
            &[QuadrantCoord::new(4, 0), QuadrantCoord::new(0, 4)]
        );
        assert_eq!(east_then_north.revision(), 3);
        assert_eq!(north_then_east.revision(), 3);
        Ok(())
    }

    #[test]
    fn prepared_candidate_is_never_downgraded_by_refresh() -> Result<(), Box<dyn Error>> {
        let mut state = frontier_fixture()?;
        derive_candidate_frontier(&mut state)?;
        let coord = QuadrantCoord::new(3, 0);
        state.mark_prepared(coord)?;

        let refreshed = derive_candidate_frontier(&mut state)?;
        assert!(refreshed.is_empty());
        assert_eq!(state.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn reveal_exposes_only_new_potential_neighbors_as_candidates() -> Result<(), Box<dyn Error>> {
        let mut state = frontier_fixture()?;
        derive_candidate_frontier(&mut state)?;
        let coord = QuadrantCoord::new(3, 0);
        state.mark_prepared(coord)?;
        let reveal = state
            .reveal(coord)?
            .ok_or_else(|| std::io::Error::other("expected Prepared -> Revealed"))?;
        assert_eq!(reveal.revision(), 2);

        let derived = derive_candidate_frontier(&mut state)?;
        assert_eq!(derived.newly_candidates(), &[QuadrantCoord::new(4, 0)]);
        assert_eq!(
            state.state(QuadrantCoord::new(4, 0)),
            FrontierQuadrantState::Candidate
        );
        assert_eq!(
            state.state(QuadrantCoord::new(3, -1)),
            FrontierQuadrantState::Candidate
        );
        assert_eq!(
            state.state(QuadrantCoord::new(3, 1)),
            FrontierQuadrantState::Candidate
        );
        assert_eq!(state.revision(), 2);
        assert_eq!(state.len(), 17);
        Ok(())
    }

    #[test]
    fn coordinate_overflow_is_non_mutating() -> Result<(), Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(i64::MAX, 0), 1, 1)?;
        let mut state = FrontierState::from_initial(&initial, 9)?;

        assert!(matches!(
            derive_candidate_frontier(&mut state),
            Err(CandidateFrontierError::QuadrantCoordinateOverflow {
                coord,
                axis: "x",
                delta: 1,
            }) if coord == QuadrantCoord::new(i64::MAX, 0)
        ));
        assert_eq!(state.revision(), 9);
        assert_eq!(state.len(), 1);
        assert_eq!(
            state.state(QuadrantCoord::new(i64::MAX - 1, 0)),
            FrontierQuadrantState::Potential
        );
        assert_eq!(
            state.state(QuadrantCoord::new(i64::MAX, -1)),
            FrontierQuadrantState::Potential
        );
        assert_eq!(
            state.state(QuadrantCoord::new(i64::MAX, 1)),
            FrontierQuadrantState::Potential
        );
        Ok(())
    }
}
