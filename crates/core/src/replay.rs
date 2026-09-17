const WINDOW_BITS: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDecision {
    AcceptedNewHighest,
    AcceptedOutOfOrder,
    RejectedDuplicate,
    RejectedTooOld,
}

impl ReplayDecision {
    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::AcceptedNewHighest | Self::AcceptedOutOfOrder)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReplayWindow {
    highest: Option<u64>,
    seen: u64,
}

impl ReplayWindow {
    #[must_use]
    pub fn observe(&mut self, sequence: u64) -> ReplayDecision {
        let Some(highest) = self.highest else {
            self.highest = Some(sequence);
            self.seen = 1;
            return ReplayDecision::AcceptedNewHighest;
        };

        if sequence > highest {
            let shift = sequence - highest;
            self.seen = if shift >= WINDOW_BITS {
                1
            } else {
                (self.seen << shift) | 1
            };
            self.highest = Some(sequence);
            return ReplayDecision::AcceptedNewHighest;
        }

        let distance = highest - sequence;
        if distance >= WINDOW_BITS {
            return ReplayDecision::RejectedTooOld;
        }

        let mask = 1_u64 << distance;
        if self.seen & mask != 0 {
            return ReplayDecision::RejectedDuplicate;
        }

        self.seen |= mask;
        ReplayDecision::AcceptedOutOfOrder
    }

    #[must_use]
    pub fn accept(&mut self, sequence: u64) -> bool {
        self.observe(sequence).is_accepted()
    }

    #[must_use]
    pub const fn highest(&self) -> Option<u64> {
        self.highest
    }
}

#[cfg(test)]
mod tests {
    use super::{ReplayDecision, ReplayWindow};

    #[test]
    fn classifies_duplicates_and_unseen_out_of_order_packets() {
        let mut window = ReplayWindow::default();
        assert_eq!(window.observe(10), ReplayDecision::AcceptedNewHighest);
        assert_eq!(window.observe(12), ReplayDecision::AcceptedNewHighest);
        assert_eq!(window.observe(11), ReplayDecision::AcceptedOutOfOrder);
        assert_eq!(window.observe(11), ReplayDecision::RejectedDuplicate);
        assert_eq!(window.observe(10), ReplayDecision::RejectedDuplicate);
        assert_eq!(window.highest(), Some(12));
    }

    #[test]
    fn rejects_packets_older_than_window() {
        let mut window = ReplayWindow::default();
        assert!(window.accept(1));
        assert!(window.accept(100));
        assert_eq!(window.observe(1), ReplayDecision::RejectedTooOld);
    }
}
