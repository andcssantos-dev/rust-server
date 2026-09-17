use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use aurenfall_core::ConnectionId;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct FrontierDeliveryPolicy {
    retry_interval: Duration,
    ack_timeout: Duration,
    max_retries: u32,
}

impl FrontierDeliveryPolicy {
    pub const DEFAULT: Self = Self {
        retry_interval: Duration::from_millis(500),
        ack_timeout: Duration::from_secs(5),
        max_retries: 3,
    };

    pub fn new(
        retry_interval: Duration,
        ack_timeout: Duration,
        max_retries: u32,
    ) -> Result<Self, FrontierDeliveryPolicyError> {
        if retry_interval.is_zero() {
            return Err(FrontierDeliveryPolicyError::ZeroRetryInterval);
        }
        if ack_timeout <= retry_interval {
            return Err(FrontierDeliveryPolicyError::AckTimeoutNotGreaterThanRetry {
                retry_interval,
                ack_timeout,
            });
        }
        if max_retries == 0 {
            return Err(FrontierDeliveryPolicyError::ZeroMaxRetries);
        }
        Ok(Self {
            retry_interval,
            ack_timeout,
            max_retries,
        })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FrontierDeliveryPolicyError {
    #[error("frontier ACK retry interval must be greater than zero")]
    ZeroRetryInterval,
    #[error("frontier ACK timeout {ack_timeout:?} must be greater than retry interval {retry_interval:?}")]
    AckTimeoutNotGreaterThanRetry {
        retry_interval: Duration,
        ack_timeout: Duration,
    },
    #[error("frontier ACK max retries must be greater than zero")]
    ZeroMaxRetries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierDeliveryAction {
    Retry {
        connection_id: ConnectionId,
        revision: u64,
        attempt: u32,
    },
    Timeout {
        connection_id: ConnectionId,
        revision: u64,
        retries_sent: u32,
    },
}

#[derive(Debug, Clone, Copy)]
struct PendingFrontierDelivery {
    revision: u64,
    started_at: Instant,
    next_retry_at: Instant,
    retries_sent: u32,
}

#[derive(Debug)]
pub struct FrontierDeliveryTracker {
    policy: FrontierDeliveryPolicy,
    pending: HashMap<ConnectionId, PendingFrontierDelivery>,
}

impl FrontierDeliveryTracker {
    #[must_use]
    pub fn new(policy: FrontierDeliveryPolicy) -> Self {
        Self {
            policy,
            pending: HashMap::new(),
        }
    }

    pub fn track(&mut self, connection_id: ConnectionId, revision: u64, now: Instant) {
        debug_assert!(revision > 0);
        self.pending.insert(
            connection_id,
            PendingFrontierDelivery {
                revision,
                started_at: now,
                next_retry_at: now + self.policy.retry_interval,
                retries_sent: 0,
            },
        );
    }

    pub fn acknowledge(&mut self, connection_id: ConnectionId, revision: u64) -> bool {
        let Some(pending) = self.pending.get(&connection_id).copied() else {
            return false;
        };
        if pending.revision != revision {
            return false;
        }
        self.pending.remove(&connection_id);
        true
    }

    pub fn remove(&mut self, connection_id: ConnectionId) {
        self.pending.remove(&connection_id);
    }

    pub fn evaluate(&mut self, now: Instant) -> Vec<FrontierDeliveryAction> {
        let connection_ids = self.pending.keys().copied().collect::<Vec<_>>();
        let mut actions = Vec::new();

        for connection_id in connection_ids {
            let Some(pending) = self.pending.get(&connection_id).copied() else {
                continue;
            };

            if now >= pending.started_at + self.policy.ack_timeout {
                self.pending.remove(&connection_id);
                actions.push(FrontierDeliveryAction::Timeout {
                    connection_id,
                    revision: pending.revision,
                    retries_sent: pending.retries_sent,
                });
                continue;
            }

            if pending.retries_sent >= self.policy.max_retries || now < pending.next_retry_at {
                continue;
            }

            let Some(current) = self.pending.get_mut(&connection_id) else {
                continue;
            };
            current.retries_sent += 1;
            current.next_retry_at = now + self.policy.retry_interval;
            actions.push(FrontierDeliveryAction::Retry {
                connection_id,
                revision: current.revision,
                attempt: current.retries_sent,
            });
        }

        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_policy_is_rejected() {
        assert!(matches!(
            FrontierDeliveryPolicy::new(Duration::ZERO, Duration::from_secs(1), 1),
            Err(FrontierDeliveryPolicyError::ZeroRetryInterval)
        ));
        assert!(matches!(
            FrontierDeliveryPolicy::new(Duration::from_secs(1), Duration::from_secs(1), 1),
            Err(FrontierDeliveryPolicyError::AckTimeoutNotGreaterThanRetry { .. })
        ));
        assert!(matches!(
            FrontierDeliveryPolicy::new(Duration::from_millis(100), Duration::from_secs(1), 0),
            Err(FrontierDeliveryPolicyError::ZeroMaxRetries)
        ));
    }

    #[test]
    fn retries_are_bounded_then_timeout() -> Result<(), FrontierDeliveryPolicyError> {
        let policy = FrontierDeliveryPolicy::new(Duration::from_millis(100), Duration::from_secs(1), 2)?;
        let mut tracker = FrontierDeliveryTracker::new(policy);
        let connection_id = ConnectionId(7);
        let start = Instant::now();
        tracker.track(connection_id, 2, start);

        assert!(tracker.evaluate(start + Duration::from_millis(99)).is_empty());
        assert_eq!(
            tracker.evaluate(start + Duration::from_millis(100)),
            vec![FrontierDeliveryAction::Retry {
                connection_id,
                revision: 2,
                attempt: 1,
            }]
        );
        assert_eq!(
            tracker.evaluate(start + Duration::from_millis(200)),
            vec![FrontierDeliveryAction::Retry {
                connection_id,
                revision: 2,
                attempt: 2,
            }]
        );
        assert!(tracker.evaluate(start + Duration::from_millis(500)).is_empty());
        assert_eq!(
            tracker.evaluate(start + Duration::from_secs(1)),
            vec![FrontierDeliveryAction::Timeout {
                connection_id,
                revision: 2,
                retries_sent: 2,
            }]
        );
        assert!(tracker.evaluate(start + Duration::from_secs(2)).is_empty());
        Ok(())
    }

    #[test]
    fn exact_ack_cancels_tracking_but_stale_ack_does_not() {
        let mut tracker = FrontierDeliveryTracker::new(FrontierDeliveryPolicy::DEFAULT);
        let connection_id = ConnectionId(7);
        let start = Instant::now();
        tracker.track(connection_id, 2, start);

        assert!(!tracker.acknowledge(connection_id, 1));
        assert_eq!(
            tracker.evaluate(start + Duration::from_millis(500)),
            vec![FrontierDeliveryAction::Retry {
                connection_id,
                revision: 2,
                attempt: 1,
            }]
        );

        assert!(tracker.acknowledge(connection_id, 2));
        assert!(tracker.evaluate(start + Duration::from_secs(10)).is_empty());
    }

    #[test]
    fn newer_revision_resets_retry_budget_and_deadline() {
        let mut tracker = FrontierDeliveryTracker::new(FrontierDeliveryPolicy::DEFAULT);
        let connection_id = ConnectionId(7);
        let start = Instant::now();
        tracker.track(connection_id, 1, start);
        let _ = tracker.evaluate(start + Duration::from_millis(500));

        let newer = start + Duration::from_secs(1);
        tracker.track(connection_id, 2, newer);
        assert!(tracker.evaluate(newer + Duration::from_millis(499)).is_empty());
        assert_eq!(
            tracker.evaluate(newer + Duration::from_millis(500)),
            vec![FrontierDeliveryAction::Retry {
                connection_id,
                revision: 2,
                attempt: 1,
            }]
        );
    }
}
