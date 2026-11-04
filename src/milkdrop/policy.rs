//! What a frame may cost, and how much detail a fade may give up for it.
//!
//! The window child and the measuring harness decide a frame by the same
//! rules: a frame that loads a preset is not a fade frame, a switch is only
//! recorded at the detail the user asked for, and a fade that runs over the
//! frame budget long enough is drawn smaller for the rest of the fade. One
//! place for them is what lets a harness number be a statement about the app.

use std::time::{Duration, Instant};

use super::Presets;
use super::governor::Governor;
use super::trace;

/// The smallest picture the user's setting and the governor may ask for
/// together: a quarter of the width and height, which is already a mosaic.
pub const MAX_SCALE: u32 = 4;

/// What the rules were for the frame about to draw.
pub struct Frame {
    /// Whether a fade was running when the frame began.
    pub fading: bool,
    /// The divisor to draw the picture at.
    pub scale: u32,
    /// When the frame began, which its cost is measured from.
    pub start: Instant,
}

/// The rules one frame follows: how big the picture is drawn, what a switch
/// cost, and whether a fade is giving up detail.
pub struct Policy {
    governor: Governor,
    /// The user's own detail divisor, before a fade asks for more.
    user_scale: u32,
    /// The frame-rate limit the fade is held to, or zero when uncapped.
    fps: u32,
    /// When the previous frame began, so a frame's cost is the gap between
    /// frames and not just the work inside it: a GPU can return from the
    /// draw before the picture is ready.
    last_start: Option<Instant>,
    /// Whether the previous frame loaded a preset. The gap this frame
    /// measures is that frame's cost, so it is the switch's, not the fade's.
    previous_switched: bool,
}

impl Policy {
    pub fn new(user_scale: u32, fps: u32) -> Self {
        Self {
            governor: Governor::new(),
            user_scale: user_scale.clamp(1, MAX_SCALE),
            fps,
            last_start: None,
            previous_switched: false,
        }
    }

    /// The user moved the detail setting.
    pub fn set_scale(&mut self, user_scale: u32) {
        self.user_scale = user_scale.clamp(1, MAX_SCALE);
    }

    /// The frame-rate limit changed, which moves the fade's budget with it.
    pub fn set_fps(&mut self, fps: u32) {
        self.fps = fps;
    }

    /// What a frame may cost before a fade is drawn with less detail: the
    /// limit it was asked to keep, or the 60 frames a second projectM is
    /// told about when the limit is uncapped.
    fn budget(&self) -> Duration {
        let fps = if self.fps == 0 { 60 } else { self.fps };
        Duration::from_secs_f32(1.0 / fps as f32)
    }

    /// The rules for the frame about to draw. `now` is when it began; it is
    /// passed in so tests can move it.
    pub fn begin(&self, presets: &Presets, now: Instant) -> Frame {
        Frame {
            fading: presets.fading(now),
            scale: self
                .user_scale
                .saturating_mul(self.governor.scale())
                .min(MAX_SCALE),
            start: now,
        }
    }

    /// Applies what the frame did once it has drawn. A switch is recorded for
    /// the slow-preset list when it ran at the detail the user asked for; the
    /// governor judges the frame as a fade frame unless it paid for a switch.
    pub fn ended(
        &mut self,
        presets: &mut Presets,
        frame: &Frame,
        loaded: Option<&trace::Loaded>,
        first_draw: Duration,
        now: Instant,
    ) {
        if let Some(loaded) = loaded
            && records_switch(self.user_scale, frame.scale)
        {
            presets.record_switch(&loaded.path, loaded.took + first_draw);
        }
        // The gap between this frame's start and the last one's, which a
        // person sees; the work inside the frame is the fallback for the
        // first frame there is nothing to compare with.
        let interval = self
            .last_start
            .map(|last| frame.start.saturating_duration_since(last))
            .unwrap_or_else(|| now.saturating_duration_since(frame.start));
        self.last_start = Some(frame.start);
        // The gap from the previous start is the previous frame's cost, so
        // the frame after a switch reports the switch's long frame. Neither
        // it nor the switch frame itself says anything about the fade.
        let switched = loaded.is_some();
        let pays_for_switch = switched || self.previous_switched;
        self.previous_switched = switched;
        self.governor
            .observe(now, interval, frame.fading, pays_for_switch, self.budget());
    }

    /// The divisor the governor is asking for, apart from the user's setting.
    #[cfg(test)]
    fn governor_scale(&self) -> u32 {
        self.governor.scale()
    }
}

/// Whether a switch drawn at `drawn` says what switching to that preset costs
/// the user. `user` is the detail their own setting asks for; when the
/// governor's step down cannot shrink the picture any further (they already
/// chose the smallest one), the switch is drawn exactly as they would see it
/// and its cost is worth recording.
pub fn records_switch(user: u32, drawn: u32) -> bool {
    drawn == user
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const BUDGET: Duration = Duration::from_nanos(1_000_000_000 / 60);

    fn loaded(name: &str, took: Duration) -> trace::Loaded {
        trace::Loaded {
            path: PathBuf::from(name),
            smooth: true,
            took,
            first_draw: Duration::ZERO,
        }
    }

    /// A fresh list, with a fade running from `start`.
    fn fading_presets(start: Instant) -> Presets {
        let mut presets = Presets::new();
        presets.fading_since = Some(start);
        presets
    }

    #[test]
    fn a_switch_is_recorded_unless_the_picture_was_smaller() {
        assert!(records_switch(1, 1), "the setting's own detail");
        assert!(
            records_switch(4, 4),
            "the user already chose the smallest picture"
        );
        assert!(!records_switch(1, 2), "a fade drew it at half size");
        assert!(!records_switch(3, 4), "the step went past the user's scale");
    }

    /// A slow switch at the user's own detail reaches the slow-preset list,
    /// which automatic play consults: the wiring the recording bug broke.
    #[test]
    fn a_slow_switch_teaches_the_slow_list() {
        let start = Instant::now();
        let mut presets = fading_presets(start);
        let mut policy = Policy::new(1, 60);
        let frame = policy.begin(&presets, start);
        let heavy = loaded("heavy.milk", Duration::from_millis(200));
        policy.ended(
            &mut presets,
            &frame,
            Some(&heavy),
            Duration::from_millis(5),
            start + BUDGET,
        );
        assert!(
            !presets.slow.allows(Path::new("heavy.milk")),
            "a 205 ms switch was not remembered"
        );

        // A cheap one is not recorded, and stays available to automatic play.
        let quick = loaded("quick.milk", Duration::from_millis(2));
        policy.ended(
            &mut presets,
            &frame,
            Some(&quick),
            Duration::from_millis(1),
            start + BUDGET * 2,
        );
        assert!(presets.slow.allows(Path::new("quick.milk")));
    }

    /// A switch drawn while a fade holds the step down is cheaper than the
    /// switch a person would see, so the slow list must not learn from it.
    #[test]
    fn a_switch_drawn_smaller_is_not_recorded() {
        let start = Instant::now();
        let mut presets = fading_presets(start);
        let mut policy = Policy::new(1, 60);

        // Eight slow fade frames: the governor gives up detail for the rest
        // of the fade, and the next switch is drawn at half size.
        for step in 0..8u32 {
            let frame = policy.begin(&presets, start + BUDGET * 3 * step);
            policy.ended(
                &mut presets,
                &frame,
                None,
                Duration::ZERO,
                start + BUDGET * 3 * (step + 1),
            );
        }
        assert_eq!(policy.governor_scale(), 2, "the fade gave up detail");

        let frame = policy.begin(&presets, start + BUDGET * 30);
        assert_eq!(frame.scale, 2, "the switch is drawn at half size");
        let heavy = loaded("heavy.milk", Duration::from_millis(200));
        policy.ended(
            &mut presets,
            &frame,
            Some(&heavy),
            Duration::from_millis(5),
            start + BUDGET * 31,
        );
        assert!(
            presets.slow.allows(Path::new("heavy.milk")),
            "a quarter-size switch was taken at face value"
        );
    }

    /// The frame that loads a preset is not a fade frame: a slow switch does
    /// not shrink the picture for a fade that is keeping up.
    #[test]
    fn a_switch_frame_does_not_judge_a_fade() {
        let start = Instant::now();
        let mut presets = fading_presets(start);
        let mut policy = Policy::new(1, 60);
        for step in 0..20u32 {
            let frame = policy.begin(&presets, start + BUDGET * 3 * step);
            let heavy = loaded("heavy.milk", Duration::from_millis(400));
            policy.ended(
                &mut presets,
                &frame,
                Some(&heavy),
                Duration::from_millis(5),
                start + BUDGET * 3 * (step + 1),
            );
        }
        assert_eq!(policy.governor_scale(), 1, "a switch judged the fade");
    }

    /// Drives frames that begin `gaps` apart, the frame at `switch` (an
    /// index into `gaps`, counted from the second frame) loading a preset,
    /// and returns the governor's divisor afterwards.
    fn run_frames(gaps: &[Duration], switch: Option<usize>) -> u32 {
        let start = Instant::now();
        let mut presets = fading_presets(start);
        let mut policy = Policy::new(1, 60);
        let heavy = loaded("heavy.milk", Duration::from_millis(110));
        let mut at = start;
        let frame = policy.begin(&presets, at);
        policy.ended(&mut presets, &frame, None, Duration::ZERO, at + BUDGET);
        for (index, gap) in gaps.iter().enumerate() {
            at += *gap;
            let frame = policy.begin(&presets, at);
            let loads = (switch == Some(index)).then_some(&heavy);
            policy.ended(
                &mut presets,
                &frame,
                loads,
                Duration::from_millis(5),
                at + BUDGET,
            );
        }
        policy.governor_scale()
    }

    /// The gap between two frame starts is the earlier frame's cost, so the
    /// frame after a switch carries the switch's long frame. It is not a
    /// fade frame either: a fade four frames late in eight must not be
    /// tipped over the five-frame threshold by one press of Next.
    #[test]
    fn the_frame_after_a_switch_does_not_judge_a_fade() {
        let late = BUDGET * 3;
        let mut gaps = vec![late, late, late, late, BUDGET, BUDGET, BUDGET];
        // The switch frame begins on time and lasts 115 ms, so the frame
        // after it begins 115 ms later.
        gaps.push(BUDGET);
        gaps.push(Duration::from_millis(115));
        gaps.extend([BUDGET; 8]);
        let switch = Some(7);
        assert_eq!(
            run_frames(&gaps, switch),
            1,
            "the switch's own frame counted as a late fade frame"
        );

        // The same frames without the switch frame: the long gap is then a
        // late fade frame, the fifth, and the fade steps down. The exclusion
        // is the switch, not every long gap.
        gaps.remove(7);
        assert_eq!(run_frames(&gaps, None), 2);
    }

    /// Only the switch's own frame is left out: a fade that stays heavy
    /// after the switch still steps down.
    #[test]
    fn a_heavy_fade_after_a_switch_still_steps_down() {
        let late = BUDGET * 3;
        let mut gaps = vec![BUDGET, Duration::from_millis(115)];
        gaps.extend([late; 8]);
        assert_eq!(run_frames(&gaps, Some(0)), 2);
    }

    /// The picture is only ever shrunk by a fade, never by the user's scale
    /// being applied twice.
    #[test]
    fn the_step_down_never_draws_a_bigger_picture() {
        let start = Instant::now();
        let mut presets = fading_presets(start);
        let mut policy = Policy::new(3, 60);
        for step in 0..8u32 {
            let frame = policy.begin(&presets, start + BUDGET * 3 * step);
            policy.ended(
                &mut presets,
                &frame,
                None,
                Duration::ZERO,
                start + BUDGET * 3 * (step + 1),
            );
        }
        assert_eq!(policy.governor_scale(), 2);
        let frame = policy.begin(&presets, start + BUDGET * 30);
        assert_eq!(frame.scale, 4, "a step down at scale three goes smaller");
    }
}
