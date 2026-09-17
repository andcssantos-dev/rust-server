use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
};

use aurenfall_core::QuadrantCoord;

pub const MAX_INITIAL_FRONTIER_AXIS: u16 = 64;
pub const MAX_REVEALED_FRONTIER_QUADRANTS: usize = 65_535;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialFrontier {
    min_coord: QuadrantCoord,
    max_coord: QuadrantCoord,
    width: u16,
    height: u16,
    revealed_quadrants: Vec<QuadrantCoord>,
}

impl InitialFrontier {
    pub fn rectangular(
        min_coord: QuadrantCoord,
        width: u16,
        height: u16,
    ) -> Result<Self, InitialFrontierError> {
        validate_dimension("width", width)?;
        validate_dimension("height", height)?;

        let max_x = min_coord
            .x()
            .checked_add(i64::from(width) - 1)
            .ok_or(InitialFrontierError::CoordinateOverflow { axis: "x" })?;
        let max_y = min_coord
            .y()
            .checked_add(i64::from(height) - 1)
            .ok_or(InitialFrontierError::CoordinateOverflow { axis: "y" })?;
        let max_coord = QuadrantCoord::new(max_x, max_y);

        let capacity = usize::from(width) * usize::from(height);
        let mut revealed_quadrants = Vec::with_capacity(capacity);
        for offset_y in 0..height {
            let y = min_coord.y() + i64::from(offset_y);
            for offset_x in 0..width {
                let x = min_coord.x() + i64::from(offset_x);
                revealed_quadrants.push(QuadrantCoord::new(x, y));
            }
        }

        Ok(Self {
            min_coord,
            max_coord,
            width,
            height,
            revealed_quadrants,
        })
    }

    #[must_use]
    pub const fn min_coord(&self) -> QuadrantCoord {
        self.min_coord
    }

    #[must_use]
    pub const fn max_coord(&self) -> QuadrantCoord {
        self.max_coord
    }

    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.revealed_quadrants.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.revealed_quadrants.is_empty()
    }

    #[must_use]
    pub fn quadrants(&self) -> &[QuadrantCoord] {
        &self.revealed_quadrants
    }

    #[must_use]
    pub fn is_revealed(&self, coord: QuadrantCoord) -> bool {
        coord.x() >= self.min_coord.x()
            && coord.x() <= self.max_coord.x()
            && coord.y() >= self.min_coord.y()
            && coord.y() <= self.max_coord.y()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierQuadrantState {
    Potential,
    Candidate,
    Prepared,
    Revealed,
}

impl FrontierQuadrantState {
    #[must_use]
    const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Potential, Self::Candidate)
                | (Self::Candidate, Self::Prepared)
                | (Self::Prepared, Self::Revealed)
        )
    }
}

impl fmt::Display for FrontierQuadrantState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Potential => "Potential",
            Self::Candidate => "Candidate",
            Self::Prepared => "Prepared",
            Self::Revealed => "Revealed",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone)]
pub struct FrontierState {
    revision: u64,
    revealed_quadrants: Vec<QuadrantCoord>,
    revealed_lookup: HashSet<QuadrantCoord>,
    lifecycle_states: HashMap<QuadrantCoord, FrontierQuadrantState>,
}

impl FrontierState {
    pub fn from_initial(initial: &InitialFrontier, revision: u64) -> Result<Self, FrontierLifecycleError> {
        if revision == 0 {
            return Err(FrontierLifecycleError::ZeroRevision);
        }
        let revealed_quadrants = initial.quadrants().to_vec();
        let revealed_lookup = revealed_quadrants.iter().copied().collect();
        Ok(Self {
            revision,
            revealed_quadrants,
            revealed_lookup,
            lifecycle_states: HashMap::new(),
        })
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.revealed_quadrants.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.revealed_quadrants.is_empty()
    }

    #[must_use]
    pub fn quadrants(&self) -> &[QuadrantCoord] {
        &self.revealed_quadrants
    }

    #[must_use]
    pub fn state(&self, coord: QuadrantCoord) -> FrontierQuadrantState {
        if self.revealed_lookup.contains(&coord) {
            FrontierQuadrantState::Revealed
        } else {
            self.lifecycle_states
                .get(&coord)
                .copied()
                .unwrap_or(FrontierQuadrantState::Potential)
        }
    }

    #[must_use]
    pub fn is_revealed(&self, coord: QuadrantCoord) -> bool {
        self.revealed_lookup.contains(&coord)
    }

    #[must_use]
    pub fn quadrants_in_state(&self, target: FrontierQuadrantState) -> Vec<QuadrantCoord> {
        if target == FrontierQuadrantState::Revealed {
            let mut quadrants = self.revealed_quadrants.clone();
            quadrants.sort_unstable_by_key(|coord| (coord.y(), coord.x()));
            return quadrants;
        }

        let mut quadrants = self
            .lifecycle_states
            .iter()
            .filter(|(_, state)| **state == target)
            .map(|(coord, _)| *coord)
            .collect::<Vec<_>>();
        quadrants.sort_unstable_by_key(|coord| (coord.y(), coord.x()));
        quadrants
    }

    pub fn mark_candidate(
        &mut self,
        coord: QuadrantCoord,
    ) -> Result<Option<FrontierLifecycleTransition>, FrontierLifecycleError> {
        self.transition(coord, FrontierQuadrantState::Candidate)
    }

    pub fn mark_prepared(
        &mut self,
        coord: QuadrantCoord,
    ) -> Result<Option<FrontierLifecycleTransition>, FrontierLifecycleError> {
        self.transition(coord, FrontierQuadrantState::Prepared)
    }

    pub fn reveal(
        &mut self,
        coord: QuadrantCoord,
    ) -> Result<Option<FrontierLifecycleTransition>, FrontierLifecycleError> {
        self.transition(coord, FrontierQuadrantState::Revealed)
    }

    fn transition(
        &mut self,
        coord: QuadrantCoord,
        target: FrontierQuadrantState,
    ) -> Result<Option<FrontierLifecycleTransition>, FrontierLifecycleError> {
        let current = self.state(coord);
        if current == target {
            return Ok(None);
        }
        if !current.can_transition_to(target) {
            return Err(FrontierLifecycleError::InvalidTransition {
                coord,
                from: current,
                to: target,
            });
        }

        let previous_revision = self.revision;
        let next_revision = if target == FrontierQuadrantState::Revealed {
            let next_len = self.revealed_quadrants.len().checked_add(1).ok_or(
                FrontierLifecycleError::FrontierCapacityExceeded {
                    attempted: usize::MAX,
                    maximum: MAX_REVEALED_FRONTIER_QUADRANTS,
                },
            )?;
            if next_len > MAX_REVEALED_FRONTIER_QUADRANTS {
                return Err(FrontierLifecycleError::FrontierCapacityExceeded {
                    attempted: next_len,
                    maximum: MAX_REVEALED_FRONTIER_QUADRANTS,
                });
            }
            previous_revision
                .checked_add(1)
                .ok_or(FrontierLifecycleError::RevisionOverflow)?
        } else {
            previous_revision
        };

        if target == FrontierQuadrantState::Revealed {
            self.lifecycle_states.remove(&coord);
            let inserted = self.revealed_lookup.insert(coord);
            debug_assert!(inserted);
            self.revealed_quadrants.push(coord);
            self.revision = next_revision;
        } else {
            self.lifecycle_states.insert(coord, target);
        }

        Ok(Some(FrontierLifecycleTransition {
            coord,
            from: current,
            to: target,
            previous_revision,
            revision: next_revision,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontierLifecycleTransition {
    coord: QuadrantCoord,
    from: FrontierQuadrantState,
    to: FrontierQuadrantState,
    previous_revision: u64,
    revision: u64,
}

impl FrontierLifecycleTransition {
    #[must_use]
    pub const fn coord(self) -> QuadrantCoord {
        self.coord
    }

    #[must_use]
    pub const fn from(self) -> FrontierQuadrantState {
        self.from
    }

    #[must_use]
    pub const fn to(self) -> FrontierQuadrantState {
        self.to
    }

    #[must_use]
    pub const fn previous_revision(self) -> u64 {
        self.previous_revision
    }

    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn advances_public_revision(self) -> bool {
        self.revision != self.previous_revision
    }
}

fn validate_dimension(axis: &'static str, value: u16) -> Result<(), InitialFrontierError> {
    if value == 0 {
        return Err(InitialFrontierError::EmptyDimension { axis });
    }
    if value > MAX_INITIAL_FRONTIER_AXIS {
        return Err(InitialFrontierError::DimensionTooLarge {
            axis,
            value,
            maximum: MAX_INITIAL_FRONTIER_AXIS,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialFrontierError {
    EmptyDimension {
        axis: &'static str,
    },
    DimensionTooLarge {
        axis: &'static str,
        value: u16,
        maximum: u16,
    },
    CoordinateOverflow {
        axis: &'static str,
    },
}

impl fmt::Display for InitialFrontierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDimension { axis } => {
                write!(formatter, "initial frontier {axis} must be greater than zero")
            }
            Self::DimensionTooLarge { axis, value, maximum } => write!(
                formatter,
                "initial frontier {axis} {value} exceeds maximum {maximum}"
            ),
            Self::CoordinateOverflow { axis } => {
                write!(formatter, "initial frontier overflows quadrant {axis} coordinate")
            }
        }
    }
}

impl Error for InitialFrontierError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierLifecycleError {
    ZeroRevision,
    InvalidTransition {
        coord: QuadrantCoord,
        from: FrontierQuadrantState,
        to: FrontierQuadrantState,
    },
    FrontierCapacityExceeded {
        attempted: usize,
        maximum: usize,
    },
    RevisionOverflow,
}

impl fmt::Display for FrontierLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRevision => write!(formatter, "frontier revision must be non-zero"),
            Self::InvalidTransition { coord, from, to } => write!(
                formatter,
                "invalid frontier lifecycle transition ({}, {}) {from} -> {to}",
                coord.x(),
                coord.y()
            ),
            Self::FrontierCapacityExceeded { attempted, maximum } => write!(
                formatter,
                "frontier reveal would contain {attempted} quadrants; maximum is {maximum}"
            ),
            Self::RevisionOverflow => write!(formatter, "frontier revision overflowed u64"),
        }
    }
}

impl Error for FrontierLifecycleError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn lifecycle_fixture(revision: u64) -> Result<FrontierState, Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        Ok(FrontierState::from_initial(&initial, revision)?)
    }

    #[test]
    fn four_by_four_frontier_is_deterministic_and_row_major() -> Result<(), InitialFrontierError> {
        let frontier = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;

        assert_eq!(frontier.len(), 16);
        assert!(!frontier.is_empty());
        assert_eq!(frontier.min_coord(), QuadrantCoord::new(-1, -1));
        assert_eq!(frontier.max_coord(), QuadrantCoord::new(2, 2));
        assert_eq!(frontier.quadrants()[0], QuadrantCoord::new(-1, -1));
        assert_eq!(frontier.quadrants()[3], QuadrantCoord::new(2, -1));
        assert_eq!(frontier.quadrants()[4], QuadrantCoord::new(-1, 0));
        assert_eq!(frontier.quadrants()[15], QuadrantCoord::new(2, 2));
        Ok(())
    }

    #[test]
    fn absent_quadrants_remain_implicit_potential() -> Result<(), InitialFrontierError> {
        let frontier = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;

        assert!(frontier.is_revealed(QuadrantCoord::new(0, 0)));
        assert!(frontier.is_revealed(QuadrantCoord::new(2, 2)));
        assert!(!frontier.is_revealed(QuadrantCoord::new(3, 0)));
        assert!(!frontier.is_revealed(QuadrantCoord::new(-2, 0)));
        assert_eq!(frontier.len(), 16);
        Ok(())
    }

    #[test]
    fn dimensions_are_bounded_before_allocation() {
        assert!(matches!(
            InitialFrontier::rectangular(QuadrantCoord::new(0, 0), 0, 4),
            Err(InitialFrontierError::EmptyDimension { axis: "width" })
        ));
        assert!(matches!(
            InitialFrontier::rectangular(QuadrantCoord::new(0, 0), 4, 65),
            Err(InitialFrontierError::DimensionTooLarge {
                axis: "height",
                value: 65,
                maximum: MAX_INITIAL_FRONTIER_AXIS,
            })
        ));
    }

    #[test]
    fn coordinate_overflow_is_rejected_before_materialization() {
        assert!(matches!(
            InitialFrontier::rectangular(QuadrantCoord::new(i64::MAX, 0), 2, 1),
            Err(InitialFrontierError::CoordinateOverflow { axis: "x" })
        ));
        assert!(matches!(
            InitialFrontier::rectangular(QuadrantCoord::new(0, i64::MAX), 1, 2),
            Err(InitialFrontierError::CoordinateOverflow { axis: "y" })
        ));
    }

    #[test]
    fn initial_revealed_and_implicit_potential_are_distinct() -> Result<(), Box<dyn Error>> {
        let state = lifecycle_fixture(1)?;

        assert_eq!(
            state.state(QuadrantCoord::new(0, 0)),
            FrontierQuadrantState::Revealed
        );
        assert_eq!(
            state.state(QuadrantCoord::new(3, 0)),
            FrontierQuadrantState::Potential
        );
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn private_states_do_not_advance_public_revision() -> Result<(), Box<dyn Error>> {
        let mut state = lifecycle_fixture(1)?;
        let coord = QuadrantCoord::new(3, 0);

        let candidate = state
            .mark_candidate(coord)?
            .ok_or_else(|| std::io::Error::other("expected Potential -> Candidate"))?;
        assert_eq!(candidate.from(), FrontierQuadrantState::Potential);
        assert_eq!(candidate.to(), FrontierQuadrantState::Candidate);
        assert!(!candidate.advances_public_revision());
        assert_eq!(state.state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);

        let prepared = state
            .mark_prepared(coord)?
            .ok_or_else(|| std::io::Error::other("expected Candidate -> Prepared"))?;
        assert_eq!(prepared.from(), FrontierQuadrantState::Candidate);
        assert_eq!(prepared.to(), FrontierQuadrantState::Prepared);
        assert!(!prepared.advances_public_revision());
        assert_eq!(state.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn prepared_to_revealed_advances_revision_once() -> Result<(), Box<dyn Error>> {
        let mut state = lifecycle_fixture(7)?;
        let coord = QuadrantCoord::new(3, 0);
        state.mark_candidate(coord)?;
        state.mark_prepared(coord)?;

        let revealed = state
            .reveal(coord)?
            .ok_or_else(|| std::io::Error::other("expected Prepared -> Revealed"))?;
        assert_eq!(revealed.coord(), coord);
        assert_eq!(revealed.from(), FrontierQuadrantState::Prepared);
        assert_eq!(revealed.to(), FrontierQuadrantState::Revealed);
        assert_eq!(revealed.previous_revision(), 7);
        assert_eq!(revealed.revision(), 8);
        assert!(revealed.advances_public_revision());
        assert_eq!(state.revision(), 8);
        assert_eq!(state.len(), 17);
        assert_eq!(state.quadrants().last(), Some(&coord));
        assert!(state.is_revealed(coord));

        assert_eq!(state.reveal(coord)?, None);
        assert_eq!(state.revision(), 8);
        assert_eq!(state.len(), 17);
        Ok(())
    }

    #[test]
    fn repeated_private_transition_is_idempotent() -> Result<(), Box<dyn Error>> {
        let mut state = lifecycle_fixture(1)?;
        let coord = QuadrantCoord::new(3, 0);

        assert!(state.mark_candidate(coord)?.is_some());
        assert_eq!(state.mark_candidate(coord)?, None);
        assert_eq!(state.state(coord), FrontierQuadrantState::Candidate);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn lifecycle_cannot_skip_required_states() -> Result<(), Box<dyn Error>> {
        let mut state = lifecycle_fixture(1)?;
        let coord = QuadrantCoord::new(3, 0);

        assert_eq!(
            state.mark_prepared(coord),
            Err(FrontierLifecycleError::InvalidTransition {
                coord,
                from: FrontierQuadrantState::Potential,
                to: FrontierQuadrantState::Prepared,
            })
        );
        assert_eq!(
            state.reveal(coord),
            Err(FrontierLifecycleError::InvalidTransition {
                coord,
                from: FrontierQuadrantState::Potential,
                to: FrontierQuadrantState::Revealed,
            })
        );
        assert_eq!(state.state(coord), FrontierQuadrantState::Potential);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn revealed_cannot_regress_to_private_state() -> Result<(), Box<dyn Error>> {
        let mut state = lifecycle_fixture(1)?;
        let coord = QuadrantCoord::new(0, 0);

        assert_eq!(
            state.mark_candidate(coord),
            Err(FrontierLifecycleError::InvalidTransition {
                coord,
                from: FrontierQuadrantState::Revealed,
                to: FrontierQuadrantState::Candidate,
            })
        );
        assert_eq!(state.state(coord), FrontierQuadrantState::Revealed);
        assert_eq!(state.revision(), 1);
        assert_eq!(state.len(), 16);
        Ok(())
    }

    #[test]
    fn revision_overflow_is_non_mutating() -> Result<(), Box<dyn Error>> {
        let mut state = lifecycle_fixture(u64::MAX)?;
        let coord = QuadrantCoord::new(3, 0);
        state.mark_candidate(coord)?;
        state.mark_prepared(coord)?;

        assert_eq!(state.reveal(coord), Err(FrontierLifecycleError::RevisionOverflow));
        assert_eq!(state.state(coord), FrontierQuadrantState::Prepared);
        assert_eq!(state.revision(), u64::MAX);
        assert_eq!(state.len(), 16);
        assert!(!state.is_revealed(coord));
        Ok(())
    }
}
