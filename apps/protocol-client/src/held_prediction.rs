use std::{collections::VecDeque, error::Error, fmt};

use aurenfall_contracts::{MoveIntent, SelfMovementSnapshotV2};

use crate::{
    prediction::ClientTimeUs,
    reconciliation::{ReplayPlan, ReplaySegment},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SentInput {
    intent: MoveIntent,
}

#[derive(Debug)]
pub struct HeldInputExtrapolator {
    sent_inputs: VecDeque<SentInput>,
    capacity: usize,
    active_intent: Option<MoveIntent>,
    baseline_received_at: Option<ClientTimeUs>,
}

impl HeldInputExtrapolator {
    pub fn new(capacity: usize) -> Result<Self, HeldPredictionError> {
        if capacity == 0 {
            return Err(HeldPredictionError::ZeroCapacity);
        }
        Ok(Self {
            sent_inputs: VecDeque::with_capacity(capacity),
            capacity,
            active_intent: None,
            baseline_received_at: None,
        })
    }

    pub fn record_sent_input(&mut self, intent: MoveIntent) -> Result<(), HeldPredictionError> {
        if let Some(previous) = self.sent_inputs.back()
            && intent.sequence <= previous.intent.sequence
        {
            return Err(HeldPredictionError::NonMonotonicSentSequence {
                previous: previous.intent.sequence,
                incoming: intent.sequence,
            });
        }
        if self.sent_inputs.len() >= self.capacity {
            return Err(HeldPredictionError::HistoryFull {
                capacity: self.capacity,
            });
        }
        self.sent_inputs.push_back(SentInput { intent });
        Ok(())
    }

    pub fn apply_authoritative_snapshot(
        &mut self,
        snapshot: SelfMovementSnapshotV2,
        received_at: ClientTimeUs,
    ) -> Result<(), HeldPredictionError> {
        let active_intent = match snapshot.active_movement_sequence {
            Some(active_sequence) => {
                if snapshot.last_processed_input_sequence != Some(active_sequence) {
                    return Err(HeldPredictionError::ActiveSequenceDoesNotMatchProcessed {
                        active: active_sequence,
                        processed: snapshot.last_processed_input_sequence,
                    });
                }
                let intent = self
                    .sent_inputs
                    .iter()
                    .find(|sent| sent.intent.sequence == active_sequence)
                    .map(|sent| sent.intent)
                    .or_else(|| {
                        self.active_intent
                            .filter(|intent| intent.sequence == active_sequence)
                    })
                    .ok_or(HeldPredictionError::UnknownActiveSequence {
                        sequence: active_sequence,
                    })?;
                if intent.axis_x == 0 && intent.axis_y == 0 {
                    return Err(HeldPredictionError::ZeroInputMarkedActive {
                        sequence: active_sequence,
                    });
                }
                Some(intent)
            }
            None => None,
        };

        if let Some(processed) = snapshot.last_processed_input_sequence {
            while self
                .sent_inputs
                .front()
                .is_some_and(|sent| sent.intent.sequence < processed)
            {
                self.sent_inputs.pop_front();
            }
        }

        self.active_intent = active_intent;
        self.baseline_received_at = Some(received_at);
        Ok(())
    }

    pub fn augment_replay_plan(
        &self,
        pending: &ReplayPlan,
        now: ClientTimeUs,
    ) -> Result<ReplayPlan, HeldPredictionError> {
        let mut segments = Vec::with_capacity(pending.segments.len() + 1);
        let mut total_duration_us = 0_u64;

        if let (Some(active), Some(baseline_received_at)) = (self.active_intent, self.baseline_received_at) {
            let held_start_us = baseline_received_at.value();
            let first_pending_start = pending.segments.first().map(|segment| segment.start_us);
            let held_end_us = match first_pending_start {
                Some(start) if start > held_start_us => start.min(now.value()),
                Some(_) => held_start_us,
                None => now.value(),
            };
            if held_end_us > held_start_us {
                let duration_us = held_end_us
                    .checked_sub(held_start_us)
                    .ok_or(HeldPredictionError::DurationOverflow)?;
                total_duration_us = total_duration_us
                    .checked_add(duration_us)
                    .ok_or(HeldPredictionError::DurationOverflow)?;
                segments.push(ReplaySegment {
                    intent: active,
                    start_us: held_start_us,
                    end_us: held_end_us,
                    duration_us,
                });
            }
        }

        for segment in &pending.segments {
            total_duration_us = total_duration_us
                .checked_add(segment.duration_us)
                .ok_or(HeldPredictionError::DurationOverflow)?;
            segments.push(*segment);
        }

        Ok(ReplayPlan {
            generated_at_us: now.value(),
            total_duration_us,
            segments,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldPredictionError {
    ZeroCapacity,
    HistoryFull { capacity: usize },
    NonMonotonicSentSequence { previous: u64, incoming: u64 },
    ActiveSequenceDoesNotMatchProcessed { active: u64, processed: Option<u64> },
    UnknownActiveSequence { sequence: u64 },
    ZeroInputMarkedActive { sequence: u64 },
    DurationOverflow,
}

impl fmt::Display for HeldPredictionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => write!(formatter, "held-input history capacity must be greater than zero"),
            Self::HistoryFull { capacity } => {
                write!(
                    formatter,
                    "held-input history reached bounded capacity {capacity}"
                )
            }
            Self::NonMonotonicSentSequence { previous, incoming } => write!(
                formatter,
                "held-input sent sequence must increase: previous={previous}, incoming={incoming}"
            ),
            Self::ActiveSequenceDoesNotMatchProcessed { active, processed } => write!(
                formatter,
                "server active movement sequence {active} does not match processed boundary {processed:?}"
            ),
            Self::UnknownActiveSequence { sequence } => write!(
                formatter,
                "server marked unknown client movement sequence {sequence} as active"
            ),
            Self::ZeroInputMarkedActive { sequence } => write!(
                formatter,
                "server marked zero movement input sequence {sequence} as active"
            ),
            Self::DurationOverflow => write!(formatter, "held-input replay duration overflowed"),
        }
    }
}

impl Error for HeldPredictionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(sequence: u64, axis_x: i16) -> Result<MoveIntent, Box<dyn Error>> {
        Ok(MoveIntent::new(sequence, axis_x, 0)?)
    }

    fn snapshot(processed: Option<u64>, active: Option<u64>) -> SelfMovementSnapshotV2 {
        SelfMovementSnapshotV2 {
            server_tick: 10,
            x_mm: 100,
            y_mm: 0,
            z_mm: 0,
            last_processed_input_sequence: processed,
            active_movement_sequence: active,
        }
    }

    #[test]
    fn acknowledged_active_input_is_extrapolated_after_snapshot() -> Result<(), Box<dyn Error>> {
        let mut tracker = HeldInputExtrapolator::new(8)?;
        tracker.record_sent_input(intent(1, 16_384)?)?;
        tracker.apply_authoritative_snapshot(snapshot(Some(1), Some(1)), ClientTimeUs::new(100))?;

        let plan = tracker.augment_replay_plan(
            &ReplayPlan {
                generated_at_us: 150,
                total_duration_us: 0,
                segments: Vec::new(),
            },
            ClientTimeUs::new(150),
        )?;
        assert_eq!(plan.total_duration_us, 50);
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.segments[0].intent.sequence, 1);
        Ok(())
    }

    #[test]
    fn pending_input_supersedes_held_input_at_local_send_time() -> Result<(), Box<dyn Error>> {
        let mut tracker = HeldInputExtrapolator::new(8)?;
        tracker.record_sent_input(intent(1, 16_384)?)?;
        tracker.record_sent_input(intent(2, 8_000)?)?;
        tracker.apply_authoritative_snapshot(snapshot(Some(1), Some(1)), ClientTimeUs::new(100))?;

        let pending = ReplayPlan {
            generated_at_us: 180,
            total_duration_us: 50,
            segments: vec![ReplaySegment {
                intent: intent(2, 8_000)?,
                start_us: 130,
                end_us: 180,
                duration_us: 50,
            }],
        };
        let plan = tracker.augment_replay_plan(&pending, ClientTimeUs::new(180))?;
        assert_eq!(plan.total_duration_us, 80);
        assert_eq!(plan.segments.len(), 2);
        assert_eq!(plan.segments[0].intent.sequence, 1);
        assert_eq!(plan.segments[0].duration_us, 30);
        assert_eq!(plan.segments[1].intent.sequence, 2);
        assert_eq!(plan.segments[1].duration_us, 50);
        Ok(())
    }

    #[test]
    fn inactive_snapshot_stops_acknowledged_extrapolation() -> Result<(), Box<dyn Error>> {
        let mut tracker = HeldInputExtrapolator::new(8)?;
        tracker.record_sent_input(intent(1, 16_384)?)?;
        tracker.apply_authoritative_snapshot(snapshot(Some(1), Some(1)), ClientTimeUs::new(100))?;
        tracker.apply_authoritative_snapshot(snapshot(Some(1), None), ClientTimeUs::new(150))?;

        let plan = tracker.augment_replay_plan(
            &ReplayPlan {
                generated_at_us: 200,
                total_duration_us: 0,
                segments: Vec::new(),
            },
            ClientTimeUs::new(200),
        )?;
        assert!(plan.segments.is_empty());
        assert_eq!(plan.total_duration_us, 0);
        Ok(())
    }

    #[test]
    fn active_sequence_must_match_processed_boundary() -> Result<(), Box<dyn Error>> {
        let mut tracker = HeldInputExtrapolator::new(8)?;
        tracker.record_sent_input(intent(1, 16_384)?)?;
        tracker.record_sent_input(intent(2, 16_384)?)?;

        assert!(matches!(
            tracker.apply_authoritative_snapshot(snapshot(Some(2), Some(1)), ClientTimeUs::new(100)),
            Err(HeldPredictionError::ActiveSequenceDoesNotMatchProcessed { .. })
        ));
        Ok(())
    }
}
