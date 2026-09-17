use anyhow::{Context, bail};
use aurenfall_core::WorldPositionMm;

use crate::MovementDeltaMm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticTraversalBlocker {
    min_x_mm: i64,
    max_x_mm: i64,
    min_y_mm: i64,
    max_y_mm: i64,
}

impl StaticTraversalBlocker {
    pub fn new(min_x_mm: i64, max_x_mm: i64, min_y_mm: i64, max_y_mm: i64) -> anyhow::Result<Self> {
        if min_x_mm >= max_x_mm {
            bail!("traversal blocker X bounds must satisfy min < max");
        }
        if min_y_mm >= max_y_mm {
            bail!("traversal blocker Y bounds must satisfy min < max");
        }
        Ok(Self {
            min_x_mm,
            max_x_mm,
            min_y_mm,
            max_y_mm,
        })
    }

    #[must_use]
    pub const fn min_x_mm(self) -> i64 {
        self.min_x_mm
    }

    #[must_use]
    pub const fn max_x_mm(self) -> i64 {
        self.max_x_mm
    }

    #[must_use]
    pub const fn min_y_mm(self) -> i64 {
        self.min_y_mm
    }

    #[must_use]
    pub const fn max_y_mm(self) -> i64 {
        self.max_y_mm
    }
}

#[derive(Debug, Clone, Default)]
pub struct TraversalWorld {
    static_blockers: Vec<StaticTraversalBlocker>,
}

impl TraversalWorld {
    pub fn add_static_blocker(&mut self, blocker: StaticTraversalBlocker) {
        self.static_blockers.push(blocker);
    }

    #[must_use]
    pub fn static_blocker_count(&self) -> usize {
        self.static_blockers.len()
    }

    pub(crate) fn validate_placement(
        &self,
        position: WorldPositionMm,
        character_radius_mm: i64,
    ) -> anyhow::Result<()> {
        validate_radius(character_radius_mm)?;
        for blocker in &self.static_blockers {
            let expanded = ExpandedBlocker::new(*blocker, character_radius_mm)?;
            if expanded.contains_strict(position.x(), position.y()) {
                bail!(
                    "authoritative position ({}, {}) overlaps a traversal blocker",
                    position.x(),
                    position.y()
                );
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_horizontal(
        &self,
        position: WorldPositionMm,
        desired: MovementDeltaMm,
        character_radius_mm: i64,
    ) -> anyhow::Result<TraversalResolution> {
        self.validate_placement(position, character_radius_mm)?;

        let desired_x = position
            .x()
            .checked_add(desired.x)
            .context("candidate traversal X position overflow")?;
        let resolved_x = self.resolve_x(position.x(), desired_x, position.y(), character_radius_mm)?;

        let desired_y = position
            .y()
            .checked_add(desired.y)
            .context("candidate traversal Y position overflow")?;
        let resolved_y = self.resolve_y(position.y(), desired_y, resolved_x, character_radius_mm)?;

        let resolved = WorldPositionMm::new(resolved_x, resolved_y, position.z());
        self.validate_placement(resolved, character_radius_mm)?;

        let actual = MovementDeltaMm {
            x: resolved_x
                .checked_sub(position.x())
                .context("resolved traversal X delta overflow")?,
            y: resolved_y
                .checked_sub(position.y())
                .context("resolved traversal Y delta overflow")?,
        };
        Ok(TraversalResolution {
            position: resolved,
            desired,
            actual,
            constrained_x: actual.x != desired.x,
            constrained_y: actual.y != desired.y,
        })
    }

    fn resolve_x(&self, current_x: i64, desired_x: i64, y: i64, radius_mm: i64) -> anyhow::Result<i64> {
        let mut resolved = desired_x;
        for blocker in &self.static_blockers {
            let expanded = ExpandedBlocker::new(*blocker, radius_mm)?;
            if !expanded.contains_y_strict(y) {
                continue;
            }

            if desired_x > current_x && current_x <= expanded.min_x && desired_x > expanded.min_x {
                resolved = resolved.min(expanded.min_x);
            } else if desired_x < current_x && current_x >= expanded.max_x && desired_x < expanded.max_x {
                resolved = resolved.max(expanded.max_x);
            }
        }
        Ok(resolved)
    }

    fn resolve_y(&self, current_y: i64, desired_y: i64, x: i64, radius_mm: i64) -> anyhow::Result<i64> {
        let mut resolved = desired_y;
        for blocker in &self.static_blockers {
            let expanded = ExpandedBlocker::new(*blocker, radius_mm)?;
            if !expanded.contains_x_strict(x) {
                continue;
            }

            if desired_y > current_y && current_y <= expanded.min_y && desired_y > expanded.min_y {
                resolved = resolved.min(expanded.min_y);
            } else if desired_y < current_y && current_y >= expanded.max_y && desired_y < expanded.max_y {
                resolved = resolved.max(expanded.max_y);
            }
        }
        Ok(resolved)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TraversalResolution {
    pub(crate) position: WorldPositionMm,
    pub(crate) desired: MovementDeltaMm,
    pub(crate) actual: MovementDeltaMm,
    pub(crate) constrained_x: bool,
    pub(crate) constrained_y: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExpandedBlocker {
    min_x: i64,
    max_x: i64,
    min_y: i64,
    max_y: i64,
}

impl ExpandedBlocker {
    fn new(blocker: StaticTraversalBlocker, radius_mm: i64) -> anyhow::Result<Self> {
        validate_radius(radius_mm)?;
        Ok(Self {
            min_x: blocker
                .min_x_mm
                .checked_sub(radius_mm)
                .context("expanded traversal blocker min X overflow")?,
            max_x: blocker
                .max_x_mm
                .checked_add(radius_mm)
                .context("expanded traversal blocker max X overflow")?,
            min_y: blocker
                .min_y_mm
                .checked_sub(radius_mm)
                .context("expanded traversal blocker min Y overflow")?,
            max_y: blocker
                .max_y_mm
                .checked_add(radius_mm)
                .context("expanded traversal blocker max Y overflow")?,
        })
    }

    const fn contains_strict(self, x: i64, y: i64) -> bool {
        self.contains_x_strict(x) && self.contains_y_strict(y)
    }

    const fn contains_x_strict(self, x: i64) -> bool {
        x > self.min_x && x < self.max_x
    }

    const fn contains_y_strict(self, y: i64) -> bool {
        y > self.min_y && y < self.max_y
    }
}

fn validate_radius(radius_mm: i64) -> anyhow::Result<()> {
    if radius_mm <= 0 {
        bail!("character traversal radius must be > 0 mm");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> anyhow::Result<StaticTraversalBlocker> {
        StaticTraversalBlocker::new(650, 750, -2_000, 2_000)
    }

    #[test]
    fn free_space_preserves_requested_horizontal_delta() -> anyhow::Result<()> {
        let world = TraversalWorld::default();
        let desired = MovementDeltaMm { x: 50, y: -25 };
        let resolution = world.resolve_horizontal(WorldPositionMm::new(100, 200, 300), desired, 250)?;

        assert_eq!(resolution.position, WorldPositionMm::new(150, 175, 300));
        assert_eq!(resolution.desired, desired);
        assert_eq!(resolution.actual, desired);
        assert!(!resolution.constrained_x);
        assert!(!resolution.constrained_y);
        Ok(())
    }

    #[test]
    fn swept_positive_x_movement_stops_at_expanded_wall_boundary() -> anyhow::Result<()> {
        let mut world = TraversalWorld::default();
        world.add_static_blocker(wall()?);
        let desired = MovementDeltaMm { x: 113, y: 0 };

        let resolution = world.resolve_horizontal(WorldPositionMm::new(337, 0, 0), desired, 250)?;

        assert_eq!(resolution.position, WorldPositionMm::new(400, 0, 0));
        assert_eq!(resolution.desired, desired);
        assert_eq!(resolution.actual, MovementDeltaMm { x: 63, y: 0 });
        assert!(resolution.constrained_x);
        assert!(!resolution.constrained_y);
        Ok(())
    }

    #[test]
    fn large_axis_delta_cannot_tunnel_through_wall() -> anyhow::Result<()> {
        let mut world = TraversalWorld::default();
        world.add_static_blocker(wall()?);

        let resolution =
            world.resolve_horizontal(WorldPositionMm::ORIGIN, MovementDeltaMm { x: 5_000, y: 0 }, 250)?;

        assert_eq!(resolution.position.x(), 400);
        assert!(resolution.constrained_x);
        Ok(())
    }

    #[test]
    fn axis_separation_allows_sliding_along_wall() -> anyhow::Result<()> {
        let mut world = TraversalWorld::default();
        world.add_static_blocker(wall()?);

        let resolution = world.resolve_horizontal(
            WorldPositionMm::new(337, 0, 0),
            MovementDeltaMm { x: 113, y: 100 },
            250,
        )?;

        assert_eq!(resolution.position, WorldPositionMm::new(400, 100, 0));
        assert!(resolution.constrained_x);
        assert!(!resolution.constrained_y);
        Ok(())
    }

    #[test]
    fn spawn_inside_expanded_blocker_is_rejected() -> anyhow::Result<()> {
        let mut world = TraversalWorld::default();
        world.add_static_blocker(wall()?);

        assert!(
            world
                .validate_placement(WorldPositionMm::new(500, 0, 0), 250)
                .is_err()
        );
        assert!(
            world
                .validate_placement(WorldPositionMm::new(400, 0, 0), 250)
                .is_ok()
        );
        Ok(())
    }
}
