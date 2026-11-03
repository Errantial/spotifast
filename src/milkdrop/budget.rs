//! Presets whose switch costs too much to be chosen automatically.
//!
//! A switch is the load (read, parse, translate) and the first draw of the
//! new preset, where its shaders compile and link. Both happen on the thread
//! that draws, so their cost is one long frame. The engine cannot make that
//! cost disappear without a second context (see
//! `DESIGN-offthread-compile.md`), so a preset that costs too much is kept
//! out of automatic play instead. It stays in the folder, in history, and
//! one key press away: only the choices the app makes by itself consult
//! this list.
//!
//! The list lives for as long as the window does. It is a measurement of
//! this machine and this graphics driver, and both are cheap to measure
//! again; nothing is written to the config folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a switch may cost before its preset stops being chosen
/// automatically: three frame periods at 60 Hz.
///
/// `PLAN-milkdrop-transitions.md` asks for a worst frame of 25 ms at 60 fps.
/// A switch cost is only part of its frame (audio, overlay and present add
/// a millisecond or two), so anything above 25 ms already breaks that
/// number. The budget is deliberately higher than the criterion, at 50 ms:
/// only a switch that cannot possibly hide in one dropped frame, and that
/// has dropped at least two, is excluded. Blacklisting at the exact
/// criterion would turn ordinary scheduling noise on a loaded machine into
/// a preset that never plays by itself.
pub const SWITCH_BUDGET: Duration = Duration::from_millis(50);

/// The presets seen to switch too slowly, with the worst cost each was
/// measured at.
#[derive(Debug, Default)]
pub(super) struct SlowPresets {
    costs: HashMap<PathBuf, Duration>,
}

impl SlowPresets {
    /// Notes what a switch cost. Returns whether the preset is now kept out
    /// of automatic play. A preset already known slow keeps its worst cost;
    /// a later cheap switch does not make it trusted again, because the
    /// switch that was slow is the one a person would have seen.
    pub(super) fn record(&mut self, path: &Path, switch: Duration) -> bool {
        if switch <= SWITCH_BUDGET {
            return self.costs.contains_key(path);
        }
        let worst = self.costs.entry(path.to_path_buf()).or_default();
        if switch > *worst {
            *worst = switch;
        }
        true
    }

    /// Whether automatic play may choose this preset.
    pub(super) fn allows(&self, path: &Path) -> bool {
        !self.costs.contains_key(path)
    }

    /// How many presets are kept out of automatic play.
    pub(super) fn len(&self) -> usize {
        self.costs.len()
    }

    /// The worst switch cost measured for this preset, if it is slow.
    #[cfg(test)]
    fn cost(&self, path: &Path) -> Option<Duration> {
        self.costs.get(path).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(list: &mut SlowPresets, name: &str, milliseconds: u64) -> bool {
        list.record(Path::new(name), Duration::from_millis(milliseconds))
    }

    /// The budget is three frame periods at 60 Hz, and a switch at it is
    /// not over it.
    #[test]
    fn a_switch_is_slow_only_past_the_budget() {
        assert_eq!(SWITCH_BUDGET, Duration::from_millis(50));
        let mut list = SlowPresets::default();
        assert!(!record(&mut list, "quick.milk", 10));
        assert!(!record(&mut list, "right at it.milk", 50));
        assert!(record(&mut list, "slow.milk", 51));
        assert!(list.allows(Path::new("quick.milk")));
        assert!(list.allows(Path::new("right at it.milk")));
        assert!(!list.allows(Path::new("slow.milk")));
        assert_eq!(list.len(), 1);
    }

    /// The list keeps one entry per preset and its worst cost, however many
    /// times it is seen.
    #[test]
    fn a_preset_keeps_its_worst_switch() {
        let mut list = SlowPresets::default();
        assert!(record(&mut list, "a.milk", 60));
        assert!(record(&mut list, "a.milk", 180));
        assert!(record(&mut list, "a.milk", 70));
        assert_eq!(list.len(), 1, "one preset, one entry");
        assert_eq!(
            list.cost(Path::new("a.milk")),
            Some(Duration::from_millis(180))
        );
        assert!(!list.allows(Path::new("a.milk")));
    }

    /// A slow preset is not trusted again by a later cheap switch: the
    /// expensive one is still what the app would bring back.
    #[test]
    fn a_later_cheap_switch_does_not_clear_the_list() {
        let mut list = SlowPresets::default();
        assert!(record(&mut list, "a.milk", 120));
        assert!(record(&mut list, "a.milk", 5), "still slow");
        assert!(!list.allows(Path::new("a.milk")));
        assert_eq!(
            list.cost(Path::new("a.milk")),
            Some(Duration::from_millis(120))
        );
    }

    /// Presets are told apart by their whole path, and a preset never seen
    /// is allowed.
    #[test]
    fn only_the_path_measured_is_slow() {
        let mut list = SlowPresets::default();
        assert!(record(&mut list, "pack/a.milk", 80));
        assert!(!list.allows(Path::new("pack/a.milk")));
        assert!(list.allows(Path::new("pack/b.milk")));
        assert!(list.allows(Path::new("/somewhere/else/a.milk")));
        assert!(list.allows(Path::new("a.milk")));
        assert_eq!(list.len(), 1);
    }
}
