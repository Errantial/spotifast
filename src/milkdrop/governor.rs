//! The picture's detail, given up while a crossfade cannot hold the frame
//! rate, and taken back once it can.
//!
//! A soft cut draws the old and the new preset every frame for
//! [`super::CROSSFADE_SECONDS`], so a GPU that keeps up with one preset can
//! still miss the beat for the whole fade. This watches what each frame of a
//! fade costs and asks the engine for a smaller picture when enough frames in
//! a row run over the budget.
//!
//! It is deliberately deaf outside a fade: a slow preset on its own is the
//! detail the user asked for, and the picture is never shrunk on their
//! behalf. Inside a fade the step is taken once and latched for the rest of
//! it, so the detail cannot pump while the fade runs, and the smaller picture
//! is held for a while after the fade ends so it does not flicker at the
//! seam.

use std::time::{Duration, Instant};

/// How many frames the window looks at, which is a seventh of a second at 60
/// frames a second: long enough that one stall is not a verdict.
const WINDOW: usize = 8;
/// How many frames of the window must run over the budget to step down.
const LATE_FRAMES: usize = 5;
/// What the engine divides the picture by while a fade is too heavy. Two is
/// a quarter of the pixels, which is the one lever big enough to matter and
/// cheap enough to explain.
const DOWN: u32 = 2;
/// How long full detail waits after a fade before coming back.
const HOLD: Duration = Duration::from_secs(2);

/// Decides whether the picture is drawn at full detail or at [`DOWN`] while a
/// preset fades in.
pub(super) struct Governor {
    /// How long the last frames took, oldest replaced as new ones arrive.
    window: [Duration; WINDOW],
    at: usize,
    filled: usize,
    /// How many frames in the window ran over the budget.
    late: usize,
    /// The divisor asked of the engine: one at full detail.
    scale: u32,
    /// When the fade that took the detail away ended, while it is held.
    held: Option<Instant>,
}

impl Governor {
    pub(super) fn new() -> Self {
        Self {
            window: [Duration::ZERO; WINDOW],
            at: 0,
            filled: 0,
            late: 0,
            scale: 1,
            held: None,
        }
    }

    /// The divisor to hand the engine, on top of the user's own setting.
    pub(super) fn scale(&self) -> u32 {
        self.scale
    }

    /// One frame's cost, and whether a preset is fading in right now.
    ///
    /// A frame over the budget in a fade means the two presets did not fit;
    /// [`LATE_FRAMES`] of them in a row means the whole fade will not, so the
    /// picture gives up detail until it ends. Outside a fade the window is
    /// kept up to date but nothing is decided.
    pub(super) fn observe(
        &mut self,
        now: Instant,
        frame: Duration,
        fading: bool,
        budget: Duration,
    ) {
        if budget.is_zero() {
            self.scale = 1;
            self.held = None;
            return;
        }
        if self.filled == WINDOW {
            self.late -= usize::from(self.window[self.at] > budget);
        } else {
            self.filled += 1;
        }
        self.window[self.at] = frame;
        self.late += usize::from(frame > budget);
        self.at = (self.at + 1) % WINDOW;

        if fading {
            // A fade decides for itself; a hold from an earlier one is over.
            self.held = None;
            if self.scale == 1 && self.filled == WINDOW && self.late >= LATE_FRAMES {
                self.scale = DOWN;
            }
        } else if self.scale > 1 {
            match self.held {
                // The smaller picture is held for a while after the fade,
                // then the detail comes back: keeping it would be a silent
                // change of the quality the user chose for one slow fade.
                Some(since) if now.saturating_duration_since(since) >= HOLD => {
                    self.scale = 1;
                    self.held = None;
                }
                Some(_) => {}
                None => self.held = Some(now),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: Duration = Duration::from_nanos(1_000_000_000 / 60);

    /// Frames of a fade, one per budget, each `cost`.
    fn observe_frame(
        governor: &mut Governor,
        base: Instant,
        index: u32,
        cost: Duration,
        fading: bool,
    ) {
        governor.observe(base + BUDGET * index, cost, fading, BUDGET);
    }

    /// A fade the machine keeps up with is left alone, frame after frame, so
    /// the smaller picture never shows up where the GPU is fast enough.
    #[test]
    fn a_fade_within_the_budget_keeps_full_detail() {
        let mut governor = Governor::new();
        let base = Instant::now();
        for index in 0..400 {
            observe_frame(&mut governor, base, index, BUDGET / 2, true);
            assert_eq!(governor.scale(), 1, "frame {index} asked for less detail");
        }
    }

    /// Five slow frames in one fade is enough: the rest of the fade is drawn
    /// at a quarter of the pixels, and it stays that way until the fade ends
    /// even if the machine catches up.
    #[test]
    fn a_fade_over_the_budget_gives_up_detail_for_its_rest() {
        let mut governor = Governor::new();
        let base = Instant::now();
        // One stall is not a verdict; the window has to fill first.
        for index in 0..WINDOW as u32 - 1 {
            observe_frame(&mut governor, base, index, BUDGET * 3, true);
            assert_eq!(governor.scale(), 1, "too early at frame {index}");
        }
        observe_frame(&mut governor, base, WINDOW as u32, BUDGET * 3, true);
        assert_eq!(governor.scale(), DOWN, "the fade is too heavy");

        // The rest of the fade keeps the smaller picture, and it is only
        // taken once: the same slow frames cannot take it further.
        for index in 0..200 {
            observe_frame(&mut governor, base, 100 + index, BUDGET * 3, true);
            assert_eq!(governor.scale(), DOWN);
        }

        // Even a fade that speeds up does not get its detail back inside the
        // fade: that would be the picture pumping.
        for index in 0..200 {
            observe_frame(&mut governor, base, 400 + index, BUDGET / 4, true);
            assert_eq!(governor.scale(), DOWN, "no pumping inside a fade");
        }
    }

    /// Full detail comes back only after the hold, never at the seam.
    #[test]
    fn detail_returns_after_the_hold() {
        let mut governor = Governor::new();
        let base = Instant::now();
        for index in 0..WINDOW as u32 {
            observe_frame(&mut governor, base, index, BUDGET * 3, true);
        }
        assert_eq!(governor.scale(), DOWN);

        // The fade ends here; the smaller picture is held for `HOLD`.
        let end = base + BUDGET * 20;
        let frames = (HOLD.as_nanos() / BUDGET.as_nanos()) as u32 - 1;
        for step in 0..frames {
            governor.observe(end + BUDGET * step, BUDGET / 2, false, BUDGET);
            assert_eq!(governor.scale(), DOWN, "restored after {step} frames");
        }
        let after = end + HOLD;
        governor.observe(after, BUDGET / 2, false, BUDGET);
        assert_eq!(governor.scale(), 1, "the hold is over");
    }

    /// The window is what decides, not the last frame: a fade that stumbles
    /// once and then keeps up is left at full detail.
    #[test]
    fn one_stall_in_a_healthy_fade_is_not_enough() {
        let mut governor = Governor::new();
        let base = Instant::now();
        for index in 0..WINDOW as u32 {
            let cost = if index == 3 { BUDGET * 10 } else { BUDGET / 2 };
            observe_frame(&mut governor, base, index, cost, true);
        }
        assert_eq!(governor.scale(), 1);
    }

    /// Frames outside a fade never take detail away, however slow they are.
    #[test]
    fn a_slow_preset_keeps_the_detail_the_user_chose() {
        let mut governor = Governor::new();
        let base = Instant::now();
        for index in 0..400 {
            observe_frame(&mut governor, base, index, BUDGET * 4, false);
            assert_eq!(governor.scale(), 1, "frame {index}");
        }
    }

    /// After the hold restores detail, the next heavy fade may take it again,
    /// and the governor does not get stuck small.
    #[test]
    fn the_next_fade_decides_afresh() {
        let mut governor = Governor::new();
        let base = Instant::now();
        for index in 0..WINDOW as u32 {
            observe_frame(&mut governor, base, index, BUDGET * 3, true);
        }
        assert_eq!(governor.scale(), DOWN);

        let end = base + BUDGET * 20;
        // The fade is over here, which starts the hold; the next frame past
        // it brings full detail back.
        governor.observe(end, BUDGET / 2, false, BUDGET);
        assert_eq!(governor.scale(), DOWN, "held at the seam");
        governor.observe(end + HOLD, BUDGET / 2, false, BUDGET);
        assert_eq!(governor.scale(), 1);

        // The machine has not changed, so the next heavy fade may take the
        // detail again; the hold is not a decision to stay small for good.
        let again = end + HOLD + BUDGET;
        for index in 0..WINDOW as u32 {
            observe_frame(&mut governor, again, index, BUDGET * 3, true);
        }
        assert_eq!(governor.scale(), DOWN);
    }

    /// A budget of nothing cannot judge anything, and is treated as no fade
    /// at all rather than dividing by zero.
    #[test]
    fn no_budget_decides_nothing() {
        let mut governor = Governor::new();
        let base = Instant::now();
        governor.observe(base, Duration::from_secs(1), true, Duration::ZERO);
        assert_eq!(governor.scale(), 1);
    }
}
