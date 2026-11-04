//! Presets whose switch costs too much, twice, to be chosen automatically.
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
//! One slow switch is not enough. The first switch into a preset often pays
//! for work that happens once: a cold shader transpile and compile, the first
//! open of its file. With an empty cache that was 32 of 40 presets, most of
//! which switch in about 10 ms the next time. A preset is kept out only after
//! its second switch over the budget in this session, which a one-off cost
//! cannot cause.
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

/// How many switches over the budget keep a preset out of automatic play.
const SLOW_SWITCHES: u32 = 2;

/// What has been measured for one preset.
#[derive(Debug, Default)]
struct Seen {
    /// Switches over `SWITCH_BUDGET`. A cheap switch does not take one away:
    /// a preset that has been slow twice has shown it can drop frames, and
    /// the cheap one in between may only have found the file in the cache.
    slow: u32,
    /// The worst switch cost measured.
    worst: Duration,
}

/// The presets seen to switch too slowly, with the worst cost each was
/// measured at. A preset with one slow switch is tracked but still allowed.
#[derive(Debug, Default)]
pub(super) struct SlowPresets {
    seen: HashMap<PathBuf, Seen>,
}

impl SlowPresets {
    /// Notes what a switch cost. Returns whether the preset is now kept out
    /// of automatic play: its second slow switch does that, and a cheap
    /// switch afterwards does not make it trusted again, because the
    /// switches that were slow are the ones a person would have seen.
    pub(super) fn record(&mut self, path: &Path, switch: Duration) -> bool {
        if switch <= SWITCH_BUDGET {
            return !self.allows(path);
        }
        let seen = self.seen.entry(path.to_path_buf()).or_default();
        seen.slow = seen.slow.saturating_add(1);
        seen.worst = seen.worst.max(switch);
        seen.slow >= SLOW_SWITCHES
    }

    /// Whether automatic play may choose this preset.
    pub(super) fn allows(&self, path: &Path) -> bool {
        self.seen
            .get(path)
            .is_none_or(|seen| seen.slow < SLOW_SWITCHES)
    }

    /// How many presets are kept out of automatic play.
    pub(super) fn len(&self) -> usize {
        self.seen
            .values()
            .filter(|seen| seen.slow >= SLOW_SWITCHES)
            .count()
    }

    /// The worst switch cost measured for this preset, if it had a slow one.
    #[cfg(test)]
    fn cost(&self, path: &Path) -> Option<Duration> {
        self.seen
            .get(path)
            .filter(|seen| seen.slow > 0)
            .map(|seen| seen.worst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(list: &mut SlowPresets, name: &str, milliseconds: u64) -> bool {
        list.record(Path::new(name), Duration::from_millis(milliseconds))
    }

    /// The budget is three frame periods at 60 Hz, and a switch at it is
    /// not over it. Two switches over it keep a preset out.
    #[test]
    fn a_switch_is_slow_only_past_the_budget() {
        assert_eq!(SWITCH_BUDGET, Duration::from_millis(50));
        let mut list = SlowPresets::default();
        for _ in 0..3 {
            assert!(!record(&mut list, "quick.milk", 10));
            assert!(!record(&mut list, "right at it.milk", 50));
        }
        assert!(!record(&mut list, "slow.milk", 51));
        assert!(record(&mut list, "slow.milk", 51));
        assert!(list.allows(Path::new("quick.milk")));
        assert!(list.allows(Path::new("right at it.milk")));
        assert!(!list.allows(Path::new("slow.milk")));
        assert_eq!(list.len(), 1);
    }

    /// One slow switch alone excludes nothing: it is often the cold first
    /// switch, which pays for work done once.
    #[test]
    fn one_slow_switch_does_not_exclude() {
        let mut list = SlowPresets::default();
        assert!(!record(&mut list, "a.milk", 400));
        assert!(list.allows(Path::new("a.milk")));
        assert_eq!(list.len(), 0);
        assert_eq!(
            list.cost(Path::new("a.milk")),
            Some(Duration::from_millis(400)),
            "the cost is kept even while the preset is allowed"
        );
    }

    #[test]
    fn the_second_slow_switch_excludes() {
        let mut list = SlowPresets::default();
        assert!(!record(&mut list, "a.milk", 400));
        assert!(record(&mut list, "a.milk", 60));
        assert!(!list.allows(Path::new("a.milk")));
        assert_eq!(list.len(), 1);
    }

    /// A cheap switch between two slow ones does not start the count over.
    #[test]
    fn slow_cheap_slow_excludes() {
        let mut list = SlowPresets::default();
        assert!(!record(&mut list, "a.milk", 120));
        assert!(!record(&mut list, "a.milk", 5), "still only one slow");
        assert!(list.allows(Path::new("a.milk")));
        assert!(record(&mut list, "a.milk", 90));
        assert!(!list.allows(Path::new("a.milk")));
    }

    /// Cheap switches alone never count, however many there are.
    #[test]
    fn cheap_switches_never_exclude() {
        let mut list = SlowPresets::default();
        for _ in 0..10 {
            assert!(!record(&mut list, "a.milk", 8));
        }
        assert!(list.allows(Path::new("a.milk")));
        assert_eq!(list.cost(Path::new("a.milk")), None);
    }

    /// The list keeps one entry per preset and its worst cost, however many
    /// times it is seen, whichever switch was the worst.
    #[test]
    fn a_preset_keeps_its_worst_switch() {
        let mut list = SlowPresets::default();
        assert!(!record(&mut list, "a.milk", 60));
        assert!(record(&mut list, "a.milk", 180));
        assert!(record(&mut list, "a.milk", 70));
        assert_eq!(list.len(), 1, "one preset, one entry");
        assert_eq!(
            list.cost(Path::new("a.milk")),
            Some(Duration::from_millis(180))
        );
        assert!(!list.allows(Path::new("a.milk")));

        let mut list = SlowPresets::default();
        assert!(!record(&mut list, "b.milk", 300));
        assert!(record(&mut list, "b.milk", 70));
        assert_eq!(
            list.cost(Path::new("b.milk")),
            Some(Duration::from_millis(300)),
            "the cold first switch stays the worst"
        );
    }

    /// An excluded preset is not trusted again by a later cheap switch: the
    /// expensive ones are still what the app would bring back.
    #[test]
    fn a_later_cheap_switch_does_not_clear_the_list() {
        let mut list = SlowPresets::default();
        assert!(!record(&mut list, "a.milk", 120));
        assert!(record(&mut list, "a.milk", 120));
        assert!(record(&mut list, "a.milk", 5), "still excluded");
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
        assert!(!record(&mut list, "pack/a.milk", 80));
        assert!(record(&mut list, "pack/a.milk", 80));
        assert!(!list.allows(Path::new("pack/a.milk")));
        assert!(list.allows(Path::new("pack/b.milk")));
        assert!(list.allows(Path::new("/somewhere/else/a.milk")));
        assert!(list.allows(Path::new("a.milk")));
        assert_eq!(list.len(), 1);
    }
}
