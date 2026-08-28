//! Seq continuity tracking for one sender (per cell).
//!
//! Pure core: no alloc, no std. The u16 seq wraps; a gap is a reliability
//! observation, a backwards jump is a restart (new session) — mirroring the
//! torn-walk discipline of walks/2: never splice across a tear.

/// What a newly arrived seq says, relative to the last one seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqVerdict {
    /// First seq ever seen from this sender.
    Start,
    /// Exactly the expected next seq (contiguous, including u16 wrap).
    Contiguous,
    /// Forward jump: `missing` frames were skipped (loss, or the sender
    /// batching). The count is data, not error.
    Gap { missing: u16 },
    /// Same seq again (e.g. an ALARM redundantly fired on two links).
    Duplicate,
    /// seq went backwards (beyond a plausible forward jump): treat as a new
    /// sender session. The walk tears here — a fresh chain opens.
    Restart,
}

#[derive(Debug, Clone, Copy)]
pub struct SeqTracker {
    last: Option<u16>,
}

impl SeqTracker {
    pub fn new() -> Self {
        SeqTracker { last: None }
    }

    /// Classify `seq` and update state. `Restart` resets the tracker, so the
    /// next arrival is `Start` of the new session.
    pub fn observe(&mut self, seq: u16) -> SeqVerdict {
        let verdict = match self.last {
            None => SeqVerdict::Start,
            Some(last) => {
                // Forward distance modulo 2^16.
                let fwd = seq.wrapping_sub(last);
                if fwd == 0 {
                    SeqVerdict::Duplicate
                } else if fwd < 0x8000 {
                    if fwd == 1 {
                        SeqVerdict::Contiguous
                    } else {
                        SeqVerdict::Gap { missing: fwd - 1 }
                    }
                } else {
                    // Backwards (or >= 32768 lost — indistinguishable at v0
                    // and honestly classified as a tear).
                    SeqVerdict::Restart
                }
            }
        };
        if verdict != SeqVerdict::Restart {
            self.last = Some(seq);
        } else {
            self.last = None;
        }
        verdict
    }

    pub fn last(&self) -> Option<u16> {
        self.last
    }
}

impl Default for SeqTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuity_wrap_and_gaps() {
        let mut t = SeqTracker::new();
        assert_eq!(t.observe(0), SeqVerdict::Start);
        assert_eq!(t.observe(1), SeqVerdict::Contiguous);
        assert_eq!(t.observe(5), SeqVerdict::Gap { missing: 3 });
        assert_eq!(t.observe(5), SeqVerdict::Duplicate);
        assert_eq!(t.observe(6), SeqVerdict::Contiguous);
        // u16 wrap 65535 -> 0 is contiguous.
        let mut w = SeqTracker::new();
        assert_eq!(w.observe(65535), SeqVerdict::Start);
        assert_eq!(w.observe(0), SeqVerdict::Contiguous);
        assert_eq!(w.observe(1), SeqVerdict::Contiguous);
    }

    #[test]
    fn restart_resets() {
        let mut t = SeqTracker::new();
        assert_eq!(t.observe(100), SeqVerdict::Start);
        assert_eq!(t.observe(101), SeqVerdict::Contiguous);
        assert_eq!(t.observe(3), SeqVerdict::Restart);
        assert_eq!(t.observe(4), SeqVerdict::Start);
        assert_eq!(t.observe(5), SeqVerdict::Contiguous);
    }
}
