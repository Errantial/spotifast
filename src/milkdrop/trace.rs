//! How long a preset takes to load, and what the frames after it cost.
//!
//! Turning a `.milk` file into running shaders happens between two frames,
//! on the thread that draws, so its cost is paid by one frame and shows as
//! a hitch. This reports each switch and a summary on stderr when the child
//! is started with `SPOTIFAST_MILKDROP_TRACE` set; with it unset the only
//! cost is the cached check that reads it once.
//!
//! The measurements are wall-clock times of the frame boundaries, which is
//! what a person sees. A GPU's own timings would say where inside a frame
//! the time went, which this does not try to answer.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The child's environment variable that turns the report on.
pub const ENV: &str = "SPOTIFAST_MILKDROP_TRACE";

/// How long after a switch the frames are watched: long enough to hold the
/// whole crossfade and to see a preset settle.
const WINDOW: Duration = Duration::from_secs(1);

/// How many frames after a switch the first-frames figure counts.
const FIRST: usize = 30;

/// Whether the child was asked to report its frames. Read once, so a frame
/// asks a cached `bool` rather than the environment.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os(ENV).is_some_and(|value| !value.is_empty()))
}

/// One preset load as the drawing thread saw it.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub path: PathBuf,
    pub smooth: bool,
    pub took: Duration,
}

/// A switch whose frames are still being watched.
struct Watching {
    name: String,
    smooth: bool,
    load: Duration,
    /// When the load returned, which the frames are counted from.
    at: Instant,
    /// Frame durations since the switch, in milliseconds.
    frames: Vec<f64>,
}

/// One finished switch, kept for the summary.
struct Switch {
    load_ms: f64,
    worst_ms: f64,
    first: Vec<f64>,
}

/// Watches preset switches and the frames after them.
pub struct Trace {
    on: bool,
    /// When the previous frame was drawn. One stamp is enough to measure a
    /// frame, and keeps the windowless child reporting the same numbers as
    /// the one with a window, whose `drawn` ring serves its frame count.
    last: Option<Instant>,
    watching: Option<Watching>,
    switches: Vec<Switch>,
}

impl Default for Trace {
    fn default() -> Self {
        Self {
            on: enabled(),
            last: None,
            watching: None,
            switches: Vec::new(),
        }
    }
}

impl Trace {
    /// Starts watching a preset that has just loaded.
    pub fn note(&mut self, loaded: Loaded) {
        self.note_at(loaded, Instant::now());
    }

    /// The same, told what the time is, so tests can move it.
    fn note_at(&mut self, loaded: Loaded, at: Instant) {
        if !self.on {
            return;
        }
        // A switch the user made before the last one's second was up is
        // still reported, with the frames it had.
        self.finish();
        self.watching = Some(Watching {
            name: loaded
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| loaded.path.to_string_lossy().into_owned()),
            smooth: loaded.smooth,
            load: loaded.took,
            at,
            frames: Vec::new(),
        });
    }

    /// The end of a drawn frame. Call it for every frame, with the time the
    /// frame finished.
    pub fn frame(&mut self, now: Instant) {
        if !self.on {
            return;
        }
        let Some(last) = self.last.replace(now) else {
            return;
        };
        let Some(watching) = &mut self.watching else {
            return;
        };
        // The first frame after the load carries the load: its duration is
        // measured from the frame before the file was read.
        let took = now.saturating_duration_since(last).as_secs_f64() * 1_000.0;
        watching.frames.push(took);
        if now.saturating_duration_since(watching.at) >= WINDOW {
            self.finish();
        }
    }

    /// Reports the switch being watched and files it for the summary.
    fn finish(&mut self) {
        let Some(watching) = self.watching.take() else {
            return;
        };
        let worst_ms = watching.frames.iter().copied().fold(0.0, f64::max);
        let first: Vec<f64> = watching.frames.iter().copied().take(FIRST).collect();
        eprintln!(
            "milkdrop trace: switch preset={:?} smooth={} load_ms={:.1} first_{}_p50_ms={:.1} first_{}_max_ms={:.1} worst_{}s_ms={:.1} frames={}",
            watching.name,
            u8::from(watching.smooth),
            ms(watching.load),
            FIRST,
            percentile(&sorted(&first), 0.5),
            FIRST,
            first.iter().copied().fold(0.0, f64::max),
            WINDOW.as_secs(),
            worst_ms,
            watching.frames.len(),
        );
        self.switches.push(Switch {
            load_ms: ms(watching.load),
            worst_ms,
            first,
        });
    }

    /// Writes what every switch added up to when the child leaves.
    fn summary(&self) {
        if !self.on || self.switches.is_empty() {
            return;
        }
        let loads = sorted(&self.switches.iter().map(|s| s.load_ms).collect::<Vec<_>>());
        let worst = sorted(&self.switches.iter().map(|s| s.worst_ms).collect::<Vec<_>>());
        let first_max = self
            .switches
            .iter()
            .map(|s| s.first.iter().copied().fold(0.0, f64::max))
            .fold(0.0, f64::max);
        let slowest = self
            .switches
            .iter()
            .max_by(|a, b| a.load_ms.total_cmp(&b.load_ms))
            .map(|s| s.load_ms)
            .unwrap_or(0.0);
        eprintln!(
            "milkdrop trace: summary switches={} load_ms[p50={:.1} p99={:.1} max={:.1}] worst_{}s_ms[p50={:.1} p99={:.1} max={:.1}] first_{}_max_ms={:.1} slowest_load_ms={:.1}",
            self.switches.len(),
            percentile(&loads, 0.5),
            percentile(&loads, 0.99),
            loads.last().copied().unwrap_or(0.0),
            WINDOW.as_secs(),
            percentile(&worst, 0.5),
            percentile(&worst, 0.99),
            worst.last().copied().unwrap_or(0.0),
            FIRST,
            first_max,
            slowest,
        );
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        // The last switch's second may not have finished when the child
        // was asked to leave.
        self.finish();
        self.summary();
    }
}

fn ms(took: Duration) -> f64 {
    took.as_secs_f64() * 1_000.0
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values
}

/// The value at `fraction` of the way through sorted `values`, nearest rank.
fn percentile(values: &[f64], fraction: f64) -> f64 {
    let Some(last) = values.len().checked_sub(1) else {
        return 0.0;
    };
    let at = (fraction * last as f64).round() as usize;
    values[at.min(last)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traced() -> Trace {
        // Built by hand: the `Drop` that reports a summary forbids the
        // struct-update shorthand.
        Trace {
            on: true,
            last: None,
            watching: None,
            switches: Vec::new(),
        }
    }

    fn loaded(name: &str, took: Duration) -> Loaded {
        Loaded {
            path: PathBuf::from(name),
            smooth: true,
            took,
        }
    }

    /// A load's cost lands in the first frame after it, the first thirty
    /// frames are kept, and the worst frame of the second is reported.
    #[test]
    fn a_switch_reports_the_load_and_the_frames_after_it() {
        let mut trace = traced();
        let start = Instant::now();
        trace.frame(start);
        trace.note_at(loaded("glow.milk", Duration::from_millis(120)), start);
        // The load's frame, then fifty-nine ordinary ones.
        for step in 0..60 {
            trace.frame(start + Duration::from_millis(120 + 16 * step));
        }
        assert_eq!(trace.switches.len(), 1);
        let switch = &trace.switches[0];
        assert!((switch.load_ms - 120.0).abs() < 0.01, "{}", switch.load_ms);
        assert_eq!(switch.first.len(), FIRST);
        assert!(
            (switch.worst_ms - 120.0).abs() < 0.01,
            "the load's frame is the worst: {}",
            switch.worst_ms
        );
    }

    /// A switch made before the last one's second ended still reports, so
    /// no load is lost when somebody taps next quickly.
    #[test]
    fn a_switch_cut_short_is_still_reported() {
        let mut trace = traced();
        let start = Instant::now();
        trace.frame(start);
        trace.note_at(loaded("a.milk", Duration::from_millis(5)), start);
        trace.frame(start + Duration::from_millis(16));
        trace.note_at(
            loaded("b.milk", Duration::from_millis(7)),
            start + Duration::from_millis(100),
        );
        assert_eq!(trace.switches.len(), 1, "the first switch was reported");
        assert_eq!(trace.switches[0].first.len(), 1);
        assert!((trace.switches[0].load_ms - 5.0).abs() < 0.01);
    }

    /// The percentiles read the sorted values without running off either end.
    #[test]
    fn percentiles_stay_inside_the_values() {
        assert_eq!(percentile(&[], 0.5), 0.0);
        assert_eq!(percentile(&[4.0], 0.99), 4.0);
        let values = sorted(&[1.0, 2.0, 3.0, 4.0, 100.0]);
        assert_eq!(percentile(&values, 0.0), 1.0);
        assert_eq!(percentile(&values, 0.5), 3.0);
        assert_eq!(percentile(&values, 1.0), 100.0);
    }
}
