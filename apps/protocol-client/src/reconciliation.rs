use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

use aurenfall_contracts::{MoveIntent, SelfMovementSnapshot};
use crate::prediction::{ClientTimeUs, PredictionClockError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedPendingInput {
    pub intent: MoveIntent,
    pub sent_at: ClientTimeUs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedSnapshot {
    pub snapshot: SelfMovementSnapshot,
    pub acknowledged_inputs: usize,
    pub pending_inputs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotApplyOutcome {
    Applied(AppliedSnapshot),
    IgnoredStale { incoming_tick: u64, current_tick: u64 },
    IgnoredDuplicate { server_tick: u64 },
}

// Derivamos Copy para permitir a cópia sem erro de borrow/move
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaySegment {
    pub intent: MoveIntent,
    pub start_us: u64,
    pub end_us: u64,
    pub duration_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayPlan {
    pub generated_at_us: u64,
    pub total_duration_us: u64,
    pub segments: Vec<ReplaySegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationError {
    ZeroPendingCapacity,
    PendingHistoryFull { capacity: usize },
    NonMonotonicSentInput { previous: u64, incoming: u64 },
    NonMonotonicClientTime { previous_us: u64, incoming_us: u64 },
    ProcessedInputAckDisappeared { previous: u64 },
    ProcessedInputAckRegressed { previous: u64, incoming: u64 },
    ProcessedInputAckUnknown { incoming: u64 },
    ConflictingDuplicateSnapshot { server_tick: u64 },
    ReplayDurationOverflow,
    PredictionClock(PredictionClockError),
}

impl From<PredictionClockError> for ReconciliationError {
    fn from(error: PredictionClockError) -> Self {
        Self::PredictionClock(error)
    }
}

impl fmt::Display for ReconciliationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroPendingCapacity => write!(
                formatter,
                "pending input history capacity must be greater than zero"
            ),
            Self::PendingHistoryFull { capacity } => {
                write!(
                    formatter,
                    "pending input history reached bounded capacity {capacity}"
                )
            }
            Self::NonMonotonicSentInput { previous, incoming } => write!(
                formatter,
                "sent input sequence must increase monotonically: previous={previous}, incoming={incoming}"
            ),
            Self::NonMonotonicClientTime {
                previous_us,
                incoming_us,
            } => write!(
                formatter,
                "sent input client time must not regress: previous={previous_us}us, incoming={incoming_us}us"
            ),
            Self::ProcessedInputAckDisappeared { previous } => write!(
                formatter,
                "server processed-input acknowledgement disappeared after sequence {previous}"
            ),
            Self::ProcessedInputAckRegressed { previous, incoming } => write!(
                formatter,
                "server processed-input acknowledgement regressed: previous={previous}, incoming={incoming}"
            ),
            Self::ProcessedInputAckUnknown { incoming } => write!(
                formatter,
                "server acknowledged input sequence {incoming} that is not pending on this client"
            ),
            Self::ConflictingDuplicateSnapshot { server_tick } => write!(
                formatter,
                "server produced conflicting movement snapshots for tick {server_tick}"
            ),
            Self::ReplayDurationOverflow => write!(formatter, "client replay duration overflowed"),
            Self::PredictionClock(error) => write!(formatter, "client prediction clock error: {error}"),
        }
    }
}

impl Error for ReconciliationError {}

#[derive(Debug)]
pub struct ClientReconciler {
    pending_inputs: VecDeque<TimedPendingInput>,
    pending_capacity: usize,
    highest_sent_sequence: Option<u64>,
    last_sent_at: Option<ClientTimeUs>,
    last_processed_input_sequence: Option<u64>,
    last_snapshot: Option<SelfMovementSnapshot>,
}

impl ClientReconciler {
    pub fn new(pending_capacity: usize) -> Result<Self, ReconciliationError> {
        if pending_capacity == 0 {
            return Err(ReconciliationError::ZeroPendingCapacity);
        }
        Ok(Self {
            pending_inputs: VecDeque::with_capacity(pending_capacity),
            pending_capacity,
            highest_sent_sequence: None,
            last_sent_at: None,
            last_processed_input_sequence: None,
            last_snapshot: None,
        })
    }

    pub fn record_sent_input(
        &mut self,
        intent: MoveIntent,
        sent_at: ClientTimeUs,
    ) -> Result<(), ReconciliationError> {
        if self.pending_inputs.len() >= self.pending_capacity {
            return Err(ReconciliationError::PendingHistoryFull {
                capacity: self.pending_capacity,
            });
        }

        if let Some(previous) = self.highest_sent_sequence {
            if intent.sequence <= previous {
                return Err(ReconciliationError::NonMonotonicSentInput {
                    previous,
                    incoming: intent.sequence,
                });
            }
        }

        if let Some(previous_time) = self.last_sent_at {
            if sent_at.value() < previous_time.value() {
                return Err(ReconciliationError::NonMonotonicClientTime {
                    previous_us: previous_time.value(),
                    incoming_us: sent_at.value(),
                });
            }
        }

        self.highest_sent_sequence = Some(intent.sequence);
        self.last_sent_at = Some(sent_at);
        self.pending_inputs.push_back(TimedPendingInput { intent, sent_at });
        Ok(())
    }

    pub fn apply_snapshot(
        &mut self,
        snapshot: SelfMovementSnapshot,
    ) -> Result<SnapshotApplyOutcome, ReconciliationError> {
        if let Some(last) = &self.last_snapshot {
            if snapshot.server_tick < last.server_tick {
                return Ok(SnapshotApplyOutcome::IgnoredStale {
                    incoming_tick: snapshot.server_tick,
                    current_tick: last.server_tick,
                });
            }
            if snapshot.server_tick == last.server_tick {
                if snapshot != *last {
                    return Err(ReconciliationError::ConflictingDuplicateSnapshot {
                        server_tick: snapshot.server_tick,
                    });
                }
                return Ok(SnapshotApplyOutcome::IgnoredDuplicate {
                    server_tick: snapshot.server_tick,
                });
            }
        }

        // No contrato SelfMovementSnapshot, o campo com o sequence confirmado é `snapshot.sequence`
        let incoming_ack = snapshot.sequence;
        if let Some(previous_ack) = self.last_processed_input_sequence {
            match incoming_ack {
                None => {
                    return Err(ReconciliationError::ProcessedInputAckDisappeared {
                        previous: previous_ack,
                    });
                }
                Some(incoming) if incoming < previous_ack => {
                    return Err(ReconciliationError::ProcessedInputAckRegressed {
                        previous: previous_ack,
                        incoming,
                    });
                }
                _ => {}
            }
        }

        let initial_pending = self.pending_inputs.len();
        if let Some(ack) = incoming_ack {
            if let Some(highest) = self.highest_sent_sequence {
                if ack > highest {
                    return Err(ReconciliationError::ProcessedInputAckUnknown { incoming: ack });
                }
            }
            self.pending_inputs.retain(|timed| timed.intent.sequence > ack);
        }

        let acknowledged_inputs = initial_pending.saturating_sub(self.pending_inputs.len());
        self.last_processed_input_sequence = incoming_ack;
        self.last_snapshot = Some(snapshot);

        Ok(SnapshotApplyOutcome::Applied(AppliedSnapshot {
            snapshot,
            acknowledged_inputs,
            pending_inputs: self.pending_inputs.len(),
        }))
    }

    pub fn replay_plan(&self, now: ClientTimeUs) -> Result<ReplayPlan, ReconciliationError> {
        let mut segments = Vec::with_capacity(self.pending_inputs.len());
        let mut total_duration_us = 0_u64;

        for i in 0..self.pending_inputs.len() {
            let current = &self.pending_inputs[i];
            let start_us = current.sent_at.value();

            if start_us > now.value() {
                return Err(ReconciliationError::PredictionClock(
                    PredictionClockError::TimeRegressed {
                        earlier_us: start_us,
                        later_us: now.value(),
                    },
                ));
            }

            let end_us = if i + 1 < self.pending_inputs.len() {
                self.pending_inputs[i + 1].sent_at.value()
            } else {
                now.value()
            };

            let duration_us = end_us.saturating_sub(start_us);
            total_duration_us = total_duration_us
                .checked_add(duration_us)
                .ok_or(ReconciliationError::ReplayDurationOverflow)?;

            segments.push(ReplaySegment {
                intent: current.intent,
                start_us,
                end_us,
                duration_us,
            });
        }

        Ok(ReplayPlan {
            generated_at_us: now.value(),
            total_duration_us,
            segments,
        })
    }

    #[allow(dead_code)]
    pub fn pending_inputs(&self) -> &VecDeque<TimedPendingInput> {
        &self.pending_inputs
    }

    #[allow(dead_code)]
    pub fn last_snapshot(&self) -> Option<&SelfMovementSnapshot> {
        self.last_snapshot.as_ref()
    }
}