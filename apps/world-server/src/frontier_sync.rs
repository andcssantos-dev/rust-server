use std::collections::{HashMap, hash_map::Entry};

use aurenfall_core::ConnectionId;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierSyncState {
    Pending { revision: u64 },
    Synced { revision: u64 },
}

impl FrontierSyncState {
    #[must_use]
    pub const fn revision(self) -> u64 {
        match self {
            Self::Pending { revision } | Self::Synced { revision } => revision,
        }
    }

    #[must_use]
    pub const fn is_synced(self) -> bool {
        matches!(self, Self::Synced { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierAckOutcome {
    Synchronized { revision: u64 },
    Duplicate { revision: u64 },
    Stale { expected: u64, received: u64 },
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FrontierSyncError {
    #[error("frontier sync revision must be non-zero")]
    ZeroRevision,
    #[error("connection {connection_id:?} already has frontier sync state")]
    DuplicateConnection { connection_id: ConnectionId },
    #[error("connection {connection_id:?} has no frontier sync state")]
    UnknownConnection { connection_id: ConnectionId },
    #[error(
        "frontier ack revision {received} is ahead of expected revision {expected} for connection {connection_id:?}"
    )]
    FutureAck {
        connection_id: ConnectionId,
        expected: u64,
        received: u64,
    },
}

#[derive(Debug, Default)]
pub struct FrontierSyncRegistry {
    by_connection: HashMap<ConnectionId, FrontierSyncState>,
}

impl FrontierSyncRegistry {
    pub fn register_pending(
        &mut self,
        connection_id: ConnectionId,
        revision: u64,
    ) -> Result<(), FrontierSyncError> {
        validate_revision(revision)?;
        match self.by_connection.entry(connection_id) {
            Entry::Vacant(entry) => {
                entry.insert(FrontierSyncState::Pending { revision });
                Ok(())
            }
            Entry::Occupied(_) => Err(FrontierSyncError::DuplicateConnection { connection_id }),
        }
    }

    pub fn mark_all_pending(&mut self, revision: u64) -> Result<usize, FrontierSyncError> {
        validate_revision(revision)?;
        for state in self.by_connection.values_mut() {
            *state = FrontierSyncState::Pending { revision };
        }
        Ok(self.by_connection.len())
    }

    pub fn apply_ack(
        &mut self,
        connection_id: ConnectionId,
        received: u64,
    ) -> Result<FrontierAckOutcome, FrontierSyncError> {
        validate_revision(received)?;
        let state = self
            .by_connection
            .get_mut(&connection_id)
            .ok_or(FrontierSyncError::UnknownConnection { connection_id })?;
        let expected = state.revision();
        if received < expected {
            return Ok(FrontierAckOutcome::Stale { expected, received });
        }
        if received > expected {
            return Err(FrontierSyncError::FutureAck {
                connection_id,
                expected,
                received,
            });
        }

        match *state {
            FrontierSyncState::Pending { revision } => {
                *state = FrontierSyncState::Synced { revision };
                Ok(FrontierAckOutcome::Synchronized { revision })
            }
            FrontierSyncState::Synced { revision } => Ok(FrontierAckOutcome::Duplicate { revision }),
        }
    }

    #[must_use]
    pub fn is_synced(&self, connection_id: ConnectionId) -> bool {
        self.by_connection
            .get(&connection_id)
            .copied()
            .is_some_and(FrontierSyncState::is_synced)
    }

    #[must_use]
    pub fn state(&self, connection_id: ConnectionId) -> Option<FrontierSyncState> {
        self.by_connection.get(&connection_id).copied()
    }

    pub fn remove(&mut self, connection_id: ConnectionId) -> Option<FrontierSyncState> {
        self.by_connection.remove(&connection_id)
    }
}

fn validate_revision(revision: u64) -> Result<(), FrontierSyncError> {
    if revision == 0 {
        return Err(FrontierSyncError::ZeroRevision);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_requires_exact_ack_before_synced() -> Result<(), FrontierSyncError> {
        let connection_id = ConnectionId(7);
        let mut registry = FrontierSyncRegistry::default();
        registry.register_pending(connection_id, 2)?;

        assert!(!registry.is_synced(connection_id));
        assert_eq!(
            registry.apply_ack(connection_id, 1)?,
            FrontierAckOutcome::Stale {
                expected: 2,
                received: 1,
            }
        );
        assert!(!registry.is_synced(connection_id));
        assert_eq!(
            registry.apply_ack(connection_id, 2)?,
            FrontierAckOutcome::Synchronized { revision: 2 }
        );
        assert!(registry.is_synced(connection_id));
        assert_eq!(
            registry.apply_ack(connection_id, 2)?,
            FrontierAckOutcome::Duplicate { revision: 2 }
        );
        Ok(())
    }

    #[test]
    fn future_ack_is_protocol_error() -> Result<(), FrontierSyncError> {
        let connection_id = ConnectionId(7);
        let mut registry = FrontierSyncRegistry::default();
        registry.register_pending(connection_id, 2)?;
        assert_eq!(
            registry.apply_ack(connection_id, 3),
            Err(FrontierSyncError::FutureAck {
                connection_id,
                expected: 2,
                received: 3,
            })
        );
        assert!(!registry.is_synced(connection_id));
        Ok(())
    }

    #[test]
    fn newer_revision_returns_all_connections_to_pending() -> Result<(), FrontierSyncError> {
        let connection_id = ConnectionId(7);
        let mut registry = FrontierSyncRegistry::default();
        registry.register_pending(connection_id, 1)?;
        let _ = registry.apply_ack(connection_id, 1)?;
        assert!(registry.is_synced(connection_id));

        assert_eq!(registry.mark_all_pending(2)?, 1);
        assert_eq!(
            registry.state(connection_id),
            Some(FrontierSyncState::Pending { revision: 2 })
        );
        Ok(())
    }
}
