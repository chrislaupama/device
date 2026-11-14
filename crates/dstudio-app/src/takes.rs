//! Takes from a looped recording: every pass around the loop becomes one take of the same
//! clip. The engine writes a looped pass as one growing file, so the passes sit one after
//! another in it; the clip covers the loop and plays one pass, and the Takes submenu of the
//! clip menu picks another (`Command::SetClipTake`). Comping is splitting the clip and
//! choosing a take for each part.

use crate::controller::Controller;

/// How a recorded take that ran around the loop is cut, in timeline frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCut {
    /// Frames before the loop start (recording began before the loop): their own clip.
    pub lead: i64,
    /// The looped clip: where it starts on the timeline and its length (the loop, or the part
    /// of it after the recording began).
    pub start: i64,
    pub len: i64,
    /// Where each complete pass starts, in frames from the recording's first kept frame.
    pub passes: Vec<i64>,
}

/// The cut for a recording that kept `frames` timeline frames from `first`, with the loop
/// `(ls, le)` on the whole time. `None` when it never got round the loop (it stays one
/// linear clip). A pass the recording stopped in the middle of is left out.
pub fn loop_cut(first: i64, frames: i64, (ls, le): (i64, i64)) -> Option<LoopCut> {
    if le <= ls || first >= le || first + frames <= le {
        return None;
    }
    let start = first.max(ls);
    let len = le - start;
    let lead = start - first;
    let period = le - ls;
    let passes: Vec<i64> = (0..).map(|k| lead + k * period).take_while(|&off| off + len <= frames).collect();
    Some(LoopCut { lead, start, len, passes })
}

impl Controller {
    /// While recording: a change to the loop (or turning it off) means the passes are no longer
    /// one loop length apart, so that recording stays linear.
    pub(crate) fn takes_tick(&mut self) {
        if self.rec.recording && self.rec.rec_loop.is_some() && self.loop_frames() != self.rec.rec_loop {
            self.rec.rec_loop = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOP: (i64, i64) = (1000, 2000);

    #[test]
    fn a_recording_inside_the_loop_makes_one_take_per_pass() {
        // From the loop start: three full passes and half of a fourth.
        let c = loop_cut(1000, 3500, LOOP).unwrap();
        assert_eq!(c, LoopCut { lead: 0, start: 1000, len: 1000, passes: vec![0, 1000, 2000] });
    }

    #[test]
    fn a_recording_that_starts_in_the_loop_covers_the_rest_of_it() {
        // Punched in at 1400: the clip is 1400..2000, and each later pass starts 400 into the loop.
        let c = loop_cut(1400, 600 + 1000 + 1000, LOOP).unwrap();
        assert_eq!(c, LoopCut { lead: 0, start: 1400, len: 600, passes: vec![0, 1000, 2000] });
    }

    #[test]
    fn a_recording_from_before_the_loop_keeps_a_lead_in() {
        let c = loop_cut(600, 400 + 1000 + 1000, LOOP).unwrap();
        assert_eq!(c, LoopCut { lead: 400, start: 1000, len: 1000, passes: vec![400, 1400] });
    }

    #[test]
    fn a_recording_that_never_wrapped_stays_linear() {
        assert_eq!(loop_cut(1000, 1000, LOOP), None, "stopped right at the loop end");
        assert_eq!(loop_cut(2000, 5000, LOOP), None, "started after the loop");
        assert_eq!(loop_cut(1000, 5000, (2000, 2000)), None, "empty loop");
    }

    #[test]
    fn one_pass_and_a_bit_is_one_take() {
        let c = loop_cut(1000, 1500, LOOP).unwrap();
        assert_eq!(c.passes, vec![0]);
    }
}
