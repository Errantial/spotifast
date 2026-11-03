//! MilkDrop visualization through libprojectM.
//!
//! projectM reads MilkDrop `.milk` presets and renders them with OpenGL.
//! MilkDrop runs as this binary with `--milkdrop-child` because winit permits
//! one event loop per process and eframe owns the main one. Audio reaches the
//! child through a shared-memory ring.
//!
//! The same child can draw without a window of its own and publish frames to
//! shared memory ([`frames`]), for the app to show inside a skin ([`pane`]).
//! projectM stays out of the app's process either way: on egui's thread it
//! would freeze the interface.
//!
//! [`Presets`] lists the config directory's `milkdrop` folder and downloads
//! optional projectM preset packs. No presets are bundled.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Presets whose switch costs too much to be chosen automatically.
mod budget;
/// The MilkDrop child process: its own window, context, and event loop.
#[cfg(feature = "milkdrop")]
pub mod child;
#[cfg(feature = "milkdrop")]
pub mod engine;
/// Finished frames in shared memory, from the child to a skin's pane.
#[cfg(feature = "milkdrop")]
pub mod frames;
/// Whether a fade has to give up detail to keep the frame rate.
#[cfg(feature = "milkdrop")]
mod governor;
/// The app's side of the child process: spawning it and talking to it.
#[cfg(feature = "milkdrop")]
pub mod host;
/// The child's mode with no window, drawing frames for a pane.
#[cfg(feature = "milkdrop")]
mod offscreen;
/// Fading text over the picture: the keys, the playing song.
pub mod overlay;
/// The app's side of a pane's picture: the stream of frames and its child.
#[cfg(feature = "milkdrop")]
pub mod pane;
/// Reading the next preset's file while the one playing is on screen.
#[cfg(feature = "milkdrop")]
mod prefetch;
/// The shared-memory audio ring between the app and the MilkDrop child.
#[cfg(feature = "milkdrop")]
pub mod shm;
/// Frame and preset-load timings for the child, off unless asked for.
#[cfg(feature = "milkdrop")]
mod trace;

/// How long a preset plays before the next fades in.
pub const DEFAULT_SECONDS: u32 = 10;
/// How many frames a second the window draws by default.
pub const DEFAULT_FPS: u32 = 60;
/// Configurable frame-rate range. Zero is handled separately as uncapped.
pub const FPS_RANGE: std::ops::RangeInclusive<u32> = 10..=360;
/// Whether a refresh rate a screen reports is a rate at all: Windows reports
/// 0 or 1 Hz for "the hardware's default", which says nothing. Screens
/// faster than the dial goes still count.
pub fn is_screen_rate(hz: u32) -> bool {
    hz >= *FPS_RANGE.start()
}
/// Frame-rate slider stops: 30, 60, screen rate, current rate, and uncapped.
pub fn fps_stops(screen: u32, current: u32) -> Vec<u32> {
    let mut rates = vec![30, 60];
    let screen = if is_screen_rate(screen) { screen } else { 0 };
    for extra in [screen, current] {
        if extra > 0 && !rates.contains(&extra) {
            rates.push(extra);
        }
    }
    rates.sort_unstable();
    rates.push(0);
    rates
}

/// Label for a frame-rate stop, marking the screen's refresh rate.
pub fn fps_label(locale: crate::i18n::Locale, rate: u32, screen: u32) -> String {
    use crate::i18n::gettext;
    match rate {
        0 => gettext(locale, "Uncapped").into_owned(),
        rate if rate == screen => gettext(
            locale,
            // Translators: A frame rate that matches the display. Keep {rate}.
            "{rate} fps, your screen",
        )
        .replace("{rate}", &rate.to_string()),
        rate => format!("{rate} fps"),
    }
}

/// A screen's refresh rate in whole hertz, from the millihertz a monitor
/// reports, when it is a rate at all (see [`is_screen_rate`]).
pub fn screen_hz(millihertz: u32) -> Option<u32> {
    let hz = (f64::from(millihertz) / 1000.0).round() as u32;
    is_screen_rate(hz).then_some(hz)
}

/// The window's size when it first opens, in logical points.
pub const DEFAULT_SIZE: [f32; 2] = [640.0, 480.0];
/// The smallest the window may be dragged.
pub const MIN_SIZE: [f32; 2] = [320.0, 240.0];
/// MilkDrop's preset crossfade duration.
pub const CROSSFADE_SECONDS: f64 = 2.7;
/// How far behind the newest sample the picture runs: the same lag as the
/// analyser, so the picture and the speaker agree.
pub use crate::vis::LAG;
/// A switch (load and first draw) over this keeps a preset out of automatic
/// play; see `src/milkdrop/budget.rs`.
pub use budget::SWITCH_BUDGET;
/// How long a listing of the folder is trusted.
const LISTED_FRESH: Duration = Duration::from_secs(30);
/// How deep into the folder presets are looked for: packs come in a folder
/// of folders.
const MAX_DEPTH: usize = 4;

/// A preset pack projectM curates, fetched as a zip of `.milk` files.
pub struct Pack {
    pub name: &'static str,
    pub url: &'static str,
    /// What it holds, and roughly how big the download is.
    pub note: &'static str,
}

pub const PACKS: [Pack; 2] = [
    Pack {
        name: "MilkDrop's own",
        url: "https://github.com/projectM-visualizer/presets-milkdrop-original/archive/refs/heads/master.zip",
        note: "The 550 presets that shipped with MilkDrop 2; about 1 MB.",
    },
    Pack {
        name: "Cream of the Crop",
        url: "https://github.com/projectM-visualizer/presets-cream-of-the-crop/archive/refs/heads/master.zip",
        note: "Jason Fletcher's pick of 9,800 presets the community made; about 25 MB.",
    },
];

/// What the window wants of the engine at its next frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Fade into a preset, or cut straight to it.
    Load { path: PathBuf, smooth: bool },
}

/// The presets in the folder and the order they have played in.
pub struct Presets {
    files: Vec<PathBuf>,
    /// When the listing in `files`, or the one under way, was started.
    listed: Option<Instant>,
    /// A listing of the folder under way on a thread of its own: a pack
    /// holds thousands of presets, and walking them took a third of a second
    /// on Windows, which froze the window whenever it ran on the UI thread.
    listing: Option<mpsc::Receiver<Vec<PathBuf>>>,
    /// Played presets and the current position for Previous and Next.
    history: Vec<PathBuf>,
    at: usize,
    /// Whether automatic preset changes are disabled.
    pub locked: bool,
    /// Random order when true; folder order when false.
    random: bool,
    /// The preset the next change will play, chosen before it is needed so
    /// the child can read its file while this one plays. It joins `history`
    /// only when it plays. Kept in step with `random`, `files` and `at`.
    planned: Option<PathBuf>,
    /// Presets whose switch was measured too slow to choose by themselves.
    /// Keys and clicks still reach them.
    slow: budget::SlowPresets,
    pending: Option<Request>,
    /// When the playing preset was asked for, which its time counts from.
    started: Instant,
    /// When a fade into the playing preset began, while one is still running.
    fading_since: Option<Instant>,
    download: Option<(&'static str, mpsc::Receiver<Result<usize, String>>)>,
}

impl Default for Presets {
    fn default() -> Self {
        Self::new()
    }
}

impl Presets {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            listed: None,
            listing: None,
            history: Vec::new(),
            at: 0,
            locked: false,
            random: true,
            planned: None,
            slow: budget::SlowPresets::default(),
            pending: None,
            started: Instant::now(),
            fading_since: None,
            download: None,
        }
    }

    /// Takes a finished listing, and lists the folder again on another
    /// thread if the listing is old. The new listing shows from a later
    /// call; `wake`, if given, asks for that call once it is ready.
    pub fn refresh(&mut self, folder: &Path, wake: Option<egui::Context>) {
        if let Some(receiver) = &self.listing {
            match receiver.try_recv() {
                Ok(files) => {
                    self.files = files;
                    // A pick made from the old listing may not be in this
                    // one; choose again from what is there now.
                    self.planned = None;
                    self.listing = None;
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => self.listing = None,
            }
        }
        if self.listed.is_some_and(|at| at.elapsed() <= LISTED_FRESH) {
            return;
        }
        self.listed = Some(Instant::now());
        let (sender, receiver) = mpsc::channel();
        let folder = folder.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("milkdrop-list".into())
            .spawn(move || {
                if sender.send(list_presets(&folder)).is_ok()
                    && let Some(ctx) = wake
                {
                    ctx.request_repaint();
                }
            });
        match spawned {
            Ok(_) => self.listing = Some(receiver),
            Err(error) => log::warn!("could not start listing presets: {error}"),
        }
    }

    pub fn count(&self) -> usize {
        self.files.len()
    }

    /// Current preset, if one has been selected.
    pub fn current(&self) -> Option<&Path> {
        self.history.get(self.at).map(PathBuf::as_path)
    }

    /// Moves forward through history or selects the next preset. A key or a
    /// click reaches any preset, including one whose switch is slow.
    pub fn next(&mut self, hard: bool) {
        self.step(hard, false);
    }

    /// The preset the next change will play, chosen now if it is not known:
    /// the next entry in history when Previous has gone back, otherwise a
    /// new pick, which `next` then plays. Reading it early lets the child
    /// have its file in memory before the switch.
    ///
    /// This is the pick a key would play, so automatic play may pass it
    /// over when its switch was measured too slow; the child then reads
    /// that preset when it gets there. A read thrown away is cheaper than
    /// a switch nobody can watch.
    pub fn lookahead(&mut self) -> Option<&Path> {
        if self.at + 1 < self.history.len() {
            return self.history.get(self.at + 1).map(PathBuf::as_path);
        }
        if self.planned.is_none() {
            self.planned = self.pick();
        }
        self.planned.as_deref()
    }

    /// Fades to the next preset once this one has had `seconds`, unless it
    /// is being kept.
    ///
    /// The time is counted here rather than left to projectM, which fixes a
    /// preset's length when the preset starts, so a new Time per preset
    /// would wait for the next one, and which never asks again after a
    /// preset fails to load (#1120). A preset that fails leaves the one
    /// playing on for another turn.
    pub fn advance(&mut self, now: Instant, seconds: u32) {
        // A fade cut short jumps to its end.
        let fade = Duration::from_secs_f64(CROSSFADE_SECONDS);
        let turn = Duration::from_secs(seconds.into()).max(fade);
        if !self.locked && now.saturating_duration_since(self.started) >= turn {
            // Automatic play chooses for the picture, not for a key
            // press: a preset too slow to switch into is passed over.
            self.step(false, true);
        }
    }

    /// Notes what changing to a preset cost the frame that loaded it, so a
    /// preset that makes the picture jump is left out of automatic play.
    pub(crate) fn record_switch(&mut self, path: &Path, cost: Duration) {
        let known = !self.slow.allows(path);
        if self.slow.record(path, cost) && !known {
            log::debug!(
                "MilkDrop: switching to {} costs {:.0} ms, so it is left out of automatic play ({} presets now)",
                path.display(),
                cost.as_secs_f64() * 1_000.0,
                self.slow.len()
            );
        }
    }

    /// One turn: `automatic` when the app chose the preset by itself,
    /// which is when a slow one is passed over.
    fn step(&mut self, hard: bool, automatic: bool) {
        // A history entry is next when Previous has gone back. Automatic
        // play skips one whose switch was too slow and picks afresh.
        if self.at + 1 < self.history.len() {
            let path = &self.history[self.at + 1];
            if !automatic || self.slow.allows(path) {
                self.at += 1;
                self.request(!hard);
                return;
            }
        }
        // The pick made ahead, so the file could be read early, is the
        // one that plays; a pick not made yet is made now.
        let planned = self
            .planned
            .take()
            .filter(|path| !automatic || self.slow.allows(path));
        let pick = planned.or_else(|| {
            if automatic {
                self.pick_automatic()
            } else {
                self.pick()
            }
        });
        let Some(pick) = pick else {
            return;
        };
        // Keep the most recent 100 presets.
        if self.history.len() >= 100 {
            self.history.remove(0);
        }
        self.history.push(pick);
        self.at = self.history.len() - 1;
        self.request(!hard);
    }

    /// Whether a fade into the playing preset is still running: projectM
    /// takes [`CROSSFADE_SECONDS`] from the moment the preset was asked for.
    pub fn fading(&self, now: Instant) -> bool {
        self.fading_since.is_some_and(|started| {
            now.saturating_duration_since(started) < Duration::from_secs_f64(CROSSFADE_SECONDS)
        })
    }

    /// Back to the preset before this one.
    pub fn previous(&mut self) {
        if self.at == 0 {
            return;
        }
        self.at -= 1;
        self.request(true);
    }

    /// Toggles random and folder order, returning the new random state.
    pub fn toggle_order(&mut self) -> bool {
        self.random = !self.random;
        // The pick ahead was made in the order just left.
        self.planned = None;
        self.random
    }

    /// A preset chosen at random, not the one playing when there is a
    /// choice; in sequential order, the one after this one.
    fn pick(&self) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        let current = self.current();
        if !self.random {
            let next = current
                .and_then(|path| self.files.iter().position(|file| file == path))
                .map(|at| (at + 1) % self.files.len())
                .unwrap_or(0);
            return Some(self.files[next].clone());
        }
        let mut index = rand::random_range(0..self.files.len());
        if self.files.len() > 1 && Some(self.files[index].as_path()) == current {
            index = (index + 1) % self.files.len();
        }
        Some(self.files[index].clone())
    }

    /// The pick automatic play makes: the same choice as `pick`, but never
    /// a preset whose switch was measured too slow, and never the one
    /// playing while there is another. When nothing is left to switch to
    /// without a hitch, it picks as usual rather than stop the picture.
    fn pick_automatic(&self) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        let at = self
            .current()
            .and_then(|path| self.files.iter().position(|file| file == path));
        let allowed = |index: usize, file: &PathBuf| Some(index) != at && self.slow.allows(file);
        if !self.random {
            let files = self.files.len();
            for step in 1..=files {
                let index = match at {
                    Some(at) => (at + step) % files,
                    None => (step - 1) % files,
                };
                if allowed(index, &self.files[index]) {
                    return Some(self.files[index].clone());
                }
            }
            return self.pick();
        }
        let count = self
            .files
            .iter()
            .enumerate()
            .filter(|(index, file)| allowed(*index, file))
            .count();
        if count == 0 {
            return self.pick();
        }
        let chosen = rand::random_range(0..count);
        self.files
            .iter()
            .enumerate()
            .filter(|(index, file)| allowed(*index, file))
            .nth(chosen)
            .map(|(_, file)| file.clone())
    }

    fn request(&mut self, smooth: bool) {
        if let Some(path) = self.current() {
            self.pending = Some(Request::Load {
                path: path.to_path_buf(),
                smooth,
            });
            self.started = Instant::now();
            self.fading_since = smooth.then_some(self.started);
        }
    }

    /// Reloads the current preset without a fade after the engine restarts.
    pub fn reload_current(&mut self) {
        if self.current().is_some() {
            self.request(false);
        }
    }

    /// Takes the pending engine request.
    pub fn take_request(&mut self) -> Option<Request> {
        self.pending.take()
    }

    /// Downloads one preset pack on a worker thread.
    pub fn download(
        &mut self,
        pack: &'static Pack,
        folder: PathBuf,
        ctx: egui::Context,
        proxy: crate::settings::ProxyConfig,
    ) {
        if self.download.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("milkdrop-presets".into())
            .spawn(move || {
                let _ = sender.send(fetch_pack(pack, &folder, &proxy));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.download = Some((pack.name, receiver)),
            Err(error) => log::warn!("could not start fetching presets: {error}"),
        }
    }

    /// Fetches every pack in one go, for a first open with nothing
    /// there; failures after the first pack still land what arrived.
    pub fn download_missing(
        &mut self,
        folder: PathBuf,
        ctx: egui::Context,
        proxy: crate::settings::ProxyConfig,
    ) {
        if self.download.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("milkdrop-presets".into())
            .spawn(move || {
                let mut total = 0usize;
                let mut failed = None;
                for pack in &PACKS {
                    match fetch_pack(pack, &folder, &proxy) {
                        Ok(count) => total += count,
                        Err(error) => {
                            failed = Some(error);
                            break;
                        }
                    }
                }
                let outcome = match failed {
                    Some(error) if total == 0 => Err(error),
                    _ => Ok(total),
                };
                let _ = sender.send(outcome);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.download = Some(("the preset packs", receiver)),
            Err(error) => log::warn!("could not start fetching presets: {error}"),
        }
    }

    /// The pack on its way, if one is.
    pub fn downloading(&self) -> Option<&'static str> {
        self.download.as_ref().map(|(name, _)| *name)
    }

    /// How a download went, once it has: the number of presets added.
    pub fn poll(&mut self) -> Option<Result<usize, String>> {
        let (_, receiver) = self.download.as_ref()?;
        match receiver.try_recv() {
            Ok(result) => {
                self.download = None;
                // The folder has changed under the listing.
                self.listed = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.download = None;
                None
            }
        }
    }
}

/// Every `.milk` file in a folder and the folders inside it, sorted by
/// path without regard to case.
pub fn list_presets(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(folder, 0, &mut files);
    files.sort_by_cached_key(|path| path.to_string_lossy().to_lowercase());
    files
}

fn walk(folder: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_folder(&entry) {
            if depth + 1 < MAX_DEPTH {
                walk(&path, depth + 1, files);
            }
        } else if is_preset(&path) {
            files.push(path);
        }
    }
}

/// The program to start MilkDrop's child from. An update that replaces the
/// running file leaves `current_exe` naming nothing, but Linux keeps the
/// running file reachable through `/proc`.
pub(crate) fn this_program() -> std::io::Result<std::path::PathBuf> {
    let own = Path::new("/proc/self/exe");
    if cfg!(target_os = "linux") && own.exists() {
        return Ok(own.to_path_buf());
    }
    std::env::current_exe()
}

/// Whether the folder holds a preset, looking no further than the first:
/// cheap enough for the UI thread, where a full listing is not.
pub fn has_presets(folder: &Path) -> bool {
    fn find(folder: &Path, depth: usize) -> bool {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            if is_folder(&entry) {
                depth + 1 < MAX_DEPTH && find(&path, depth + 1)
            } else {
                is_preset(&path)
            }
        })
    }
    find(folder, 0)
}

/// Whether a folder entry is a folder, following a link to one. The entry
/// already knows its type on every system, which saves asking the file
/// system again for each of thousands of presets.
fn is_folder(entry: &std::fs::DirEntry) -> bool {
    entry
        .file_type()
        .is_ok_and(|kind| kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()))
}

pub fn is_preset(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("milk"))
}

/// A name that stays in the folder it is joined to on every platform: one
/// ordinary component, without the drive prefix (`C:x.milk`) or stream
/// suffix that a colon means on Windows.
fn is_plain_file_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
        && !name.contains(':')
}

/// Downloads a pack and unpacks its presets into the folder, flat, since
/// the packs keep theirs in folders by style and the names do not clash.
/// Returns how many were written.
pub fn fetch_pack(
    pack: &Pack,
    folder: &Path,
    proxy: &crate::settings::ProxyConfig,
) -> Result<usize, String> {
    let http = crate::http::build_blocking(proxy, Duration::from_secs(300))
        .map_err(|error| error.to_string())?;
    let bytes = http
        .get(pack.url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.bytes())
        .map_err(|error| format!("{}: {error}", pack.name))?;
    unpack_presets(&bytes, folder)
}

/// Writes the `.milk` files of a zip into the folder.
pub fn unpack_presets(zip: &[u8], folder: &Path) -> Result<usize, String> {
    let archive = crate::skin::zip::Archive::parse(zip).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let mut written = 0;
    for entry in archive.entries() {
        if entry.is_dir() {
            continue;
        }
        let file_name = entry.base_name();
        if !is_plain_file_name(file_name) || !is_preset(Path::new(file_name)) {
            continue;
        }
        let bytes = archive.read(entry).map_err(|error| error.to_string())?;
        std::fs::write(folder.join(file_name), bytes).map_err(|error| error.to_string())?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presets that have just started the first of `files`.
    fn playing(files: &[&str]) -> Presets {
        let mut presets = Presets::new();
        presets.files = files.iter().map(PathBuf::from).collect();
        presets.random = false;
        presets.next(false);
        presets.take_request();
        presets
    }

    fn asked_for(presets: &mut Presets) -> Option<(PathBuf, bool)> {
        presets.take_request().map(|request| match request {
            Request::Load { path, smooth } => (path, smooth),
        })
    }

    /// #1120: a preset lasts the time asked for, then fades to the next.
    #[test]
    fn a_preset_ends_when_its_time_is_up() {
        let mut presets = playing(&["a.milk", "b.milk"]);
        let start = presets.started;
        presets.advance(start + Duration::from_millis(9_999), 10);
        assert_eq!(asked_for(&mut presets), None);
        presets.advance(start + Duration::from_secs(10), 10);
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("b.milk"), true))
        );
    }

    /// #1120: a new time reaches the preset already playing, where projectM
    /// would keep the length that preset started with.
    #[test]
    fn a_new_time_applies_to_the_playing_preset() {
        let mut presets = playing(&["a.milk", "b.milk"]);
        let now = presets.started + Duration::from_secs(20);
        presets.advance(now, 300);
        assert_eq!(asked_for(&mut presets), None);
        presets.advance(now, 5);
        assert!(asked_for(&mut presets).is_some());
    }

    /// #1120: turns keep coming with nothing said about the last load, so
    /// one that failed cannot stop them, as it stops projectM's own timer.
    #[test]
    fn a_failed_load_does_not_stop_the_turns() {
        let mut presets = playing(&["a.milk", "b.milk"]);
        for _ in 0..3 {
            let start = presets.started;
            presets.advance(start + Duration::from_secs(10), 10);
            assert!(asked_for(&mut presets).is_some());
        }
    }

    #[test]
    fn a_kept_preset_stays() {
        let mut presets = playing(&["a.milk", "b.milk"]);
        presets.locked = true;
        presets.advance(presets.started + Duration::from_secs(3600), 10);
        assert_eq!(asked_for(&mut presets), None);
    }

    /// The shortest time is shorter than the fade: the fade finishes first.
    #[test]
    fn a_fade_is_never_cut_short() {
        let mut presets = playing(&["a.milk", "b.milk"]);
        let start = presets.started;
        presets.advance(start + Duration::from_secs(2), 2);
        assert_eq!(asked_for(&mut presets), None);
        presets.advance(start + Duration::from_secs_f64(CROSSFADE_SECONDS), 2);
        assert!(asked_for(&mut presets).is_some());
    }

    /// A soft cut is known to be running for exactly the fade's length, and a
    /// hard cut is over as it starts, so nothing shrinks the picture around
    /// one.
    #[test]
    fn a_fade_is_known_while_it_runs() {
        let mut presets = Presets::new();
        presets.files = vec![PathBuf::from("a.milk"), PathBuf::from("b.milk")];
        presets.random = false;
        presets.next(true);
        presets.take_request();
        let started = presets.started;
        assert!(!presets.fading(started), "a hard cut is over as it starts");

        presets.next(false);
        let started = presets.started;
        assert!(presets.fading(started));
        assert!(presets.fading(started + Duration::from_secs(1)));
        assert!(
            !presets.fading(started + Duration::from_secs_f64(CROSSFADE_SECONDS)),
            "the fade is over when its time is"
        );

        // Previous fades too; a hard cut and a reload do not.
        presets.previous();
        assert!(presets.fading(Instant::now()));
        presets.next(true);
        assert!(!presets.fading(Instant::now()), "a hard cut does not fade");
        presets.previous();
        presets.reload_current();
        assert!(!presets.fading(Instant::now()), "a reload cuts in");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spotifast-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn presets_are_found_in_folders_and_sorted_without_case() {
        let dir = temp_dir("milkdrop-list");
        std::fs::create_dir_all(dir.join("Sparkle")).unwrap();
        std::fs::write(dir.join("zebra.milk"), b"").unwrap();
        std::fs::write(dir.join("Apple.MILK"), b"").unwrap();
        std::fs::write(dir.join("readme.txt"), b"").unwrap();
        std::fs::write(dir.join("Sparkle").join("glow.milk"), b"").unwrap();
        let names: Vec<String> = list_presets(&dir)
            .into_iter()
            .map(|path| {
                path.strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            names,
            ["Apple.MILK", "Sparkle/glow.milk", "zebra.milk"]
                .map(|name| name.replace('/', std::path::MAIN_SEPARATOR_STR))
        );
        assert!(list_presets(Path::new("/nonexistent/milkdrop")).is_empty());
    }

    /// A pack holds thousands of presets, too many to walk on the UI
    /// thread: the listing runs on another and shows from a later refresh,
    /// and a fresh listing is not walked again.
    #[test]
    fn presets_are_listed_off_the_calling_thread() {
        let dir = temp_dir("milkdrop-refresh");
        std::fs::write(dir.join("a.milk"), b"").unwrap();
        std::fs::write(dir.join("b.milk"), b"").unwrap();
        let mut presets = Presets::new();
        presets.refresh(&dir, None);
        assert_eq!(presets.count(), 0, "the first refresh only starts it");
        let started = Instant::now();
        while presets.count() == 0 && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
            presets.refresh(&dir, None);
        }
        assert_eq!(presets.count(), 2);
        assert!(presets.listing.is_none());
        std::fs::write(dir.join("c.milk"), b"").unwrap();
        presets.refresh(&dir, None);
        assert!(presets.listing.is_none(), "a fresh listing is kept");
        assert_eq!(presets.count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Opening MilkDrop fetches the packs only into a folder without a
    /// preset, which is told without listing the whole folder.
    #[test]
    fn a_folder_has_presets_when_one_is_found_at_any_depth() {
        let dir = temp_dir("milkdrop-has");
        assert!(!has_presets(&dir));
        assert!(!has_presets(&dir.join("missing")));
        std::fs::create_dir_all(dir.join("pack").join("style")).unwrap();
        std::fs::write(dir.join("readme.txt"), b"").unwrap();
        assert!(!has_presets(&dir));
        std::fs::write(dir.join("pack").join("style").join("glow.milk"), b"").unwrap();
        assert!(has_presets(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Windows's "hardware default" is no screen rate; any real one is,
    /// screens faster than the dial goes included.
    #[test]
    fn a_hardware_default_is_no_screen_rate() {
        assert!(!is_screen_rate(0));
        assert!(!is_screen_rate(1), "Windows's hardware default");
        assert!(is_screen_rate(60));
        assert!(is_screen_rate(165));
        assert!(is_screen_rate(480), "faster than the dial goes");
        assert_eq!(fps_stops(1, 60), vec![30, 60, 0], "no stop for it");
    }

    /// The dial stops at the two rates everyone knows, the screen's own
    /// when it is neither of them, and uncapped, in that order.
    #[test]
    fn the_frame_rate_stops_where_it_is_worth_stopping() {
        assert_eq!(fps_stops(144, 144), vec![30, 60, 144, 0]);
        assert_eq!(fps_stops(0, 60), vec![30, 60, 0], "no screen has said");
        assert_eq!(fps_stops(60, 60), vec![30, 60, 0], "and no stop twice");
        assert_eq!(
            fps_stops(50, 60),
            vec![30, 50, 60, 0],
            "a slower screen takes its place in the order"
        );
        assert_eq!(
            fps_stops(144, 90),
            vec![30, 60, 90, 144, 0],
            "a rate set by hand keeps a stop of its own"
        );
        assert_eq!(fps_label(crate::i18n::Locale::English, 0, 144), "Uncapped");
        assert_eq!(
            fps_label(crate::i18n::Locale::English, 144, 144),
            "144 fps, your screen"
        );
        assert_eq!(fps_label(crate::i18n::Locale::English, 30, 144), "30 fps");
    }

    /// A monitor's millihertz round to the hertz the dial shows, the default
    /// Windows reports instead of a rate is no rate at all, and a screen
    /// faster than the dial goes keeps its rate.
    #[test]
    fn a_screen_reports_its_rate_in_millihertz() {
        assert_eq!(screen_hz(59_940), Some(60));
        assert_eq!(screen_hz(143_981), Some(144));
        assert_eq!(screen_hz(164_000), Some(164));
        assert_eq!(screen_hz(0), None);
        assert_eq!(screen_hz(1_000), None, "Windows's hardware default");
        assert_eq!(screen_hz(480_000), Some(480), "faster than the dial goes");
    }

    /// R switches between random and the folder's own order; in order,
    /// the next preset is the next file, wrapping at the end.
    #[test]
    fn sequential_order_walks_the_folder() {
        let mut presets = Presets::new();
        presets.files = vec![
            PathBuf::from("a.milk"),
            PathBuf::from("b.milk"),
            PathBuf::from("c.milk"),
        ];
        assert!(!presets.toggle_order(), "R turns the random order off");
        presets.next(true);
        assert_eq!(presets.current(), Some(Path::new("a.milk")));
        presets.next(true);
        assert_eq!(presets.current(), Some(Path::new("b.milk")));
        presets.next(true);
        assert_eq!(presets.current(), Some(Path::new("c.milk")));
        presets.next(true);
        assert_eq!(
            presets.current(),
            Some(Path::new("a.milk")),
            "the end of the folder wraps to its start"
        );
        assert!(presets.toggle_order(), "R turns it back on");
    }

    #[test]
    fn next_picks_anew_and_previous_walks_back() {
        let mut presets = Presets::new();
        presets.files = vec![PathBuf::from("a.milk"), PathBuf::from("b.milk")];
        assert_eq!(presets.current(), None);
        presets.next(false);
        let first = presets.current().unwrap().to_path_buf();
        assert_eq!(
            presets.take_request(),
            Some(Request::Load {
                path: first.clone(),
                smooth: true
            })
        );
        assert_eq!(presets.take_request(), None);
        // A second pick is never the same one while there is a choice.
        presets.next(true);
        let second = presets.current().unwrap().to_path_buf();
        assert_ne!(first, second);
        assert_eq!(
            presets.take_request(),
            Some(Request::Load {
                path: second.clone(),
                smooth: false
            })
        );
        presets.previous();
        assert_eq!(presets.current(), Some(first.as_path()));
        presets.previous();
        assert_eq!(presets.current(), Some(first.as_path()));
        presets.next(false);
        assert_eq!(presets.current(), Some(second.as_path()));
    }

    #[test]
    fn an_empty_folder_asks_for_nothing() {
        let mut presets = Presets::new();
        presets.next(false);
        assert_eq!(presets.current(), None);
        assert_eq!(presets.lookahead(), None, "and plans nothing ahead");
        assert_eq!(presets.take_request(), None);
    }

    /// The next preset is chosen before it plays, so its file can be read
    /// while this one is on screen, and that is the one that plays.
    #[test]
    fn the_next_preset_is_chosen_before_it_plays() {
        let mut presets = playing(&["a.milk", "b.milk", "c.milk"]);
        let ahead = presets.lookahead().unwrap().to_path_buf();
        assert_eq!(ahead, PathBuf::from("b.milk"));
        presets.next(false);
        assert_eq!(
            asked_for(&mut presets),
            Some((ahead.clone(), true)),
            "the preset read ahead is the one asked for"
        );
        assert_eq!(
            presets.lookahead(),
            Some(Path::new("c.milk")),
            "and the one after it is chosen once it plays"
        );
    }

    /// Asking again does not reroll a random pick, or the file read ahead
    /// would not be the one played.
    #[test]
    fn a_pick_ahead_stays_the_same_until_it_plays() {
        let mut presets = Presets::new();
        presets.files = ["a.milk", "b.milk", "c.milk", "d.milk"]
            .map(PathBuf::from)
            .to_vec();
        presets.next(false);
        presets.take_request();
        let current = presets.current().unwrap().to_path_buf();
        let ahead = presets.lookahead().unwrap().to_path_buf();
        assert_ne!(ahead, current, "a pick is never the one playing");
        for _ in 0..5 {
            assert_eq!(presets.lookahead(), Some(ahead.as_path()));
        }
        presets.next(false);
        assert_eq!(presets.current(), Some(ahead.as_path()));
    }

    /// Previous moves back through history, so the lookahead is the history
    /// entry next, not a fresh pick; only past its end is one made.
    #[test]
    fn going_back_plans_the_history_ahead() {
        let mut presets = playing(&["a.milk", "b.milk", "c.milk", "d.milk"]);
        presets.next(false);
        presets.take_request();
        presets.next(false);
        presets.take_request();
        assert_eq!(presets.current(), Some(Path::new("c.milk")));
        presets.previous();
        presets.take_request();
        assert_eq!(presets.current(), Some(Path::new("b.milk")));
        assert_eq!(
            presets.lookahead(),
            Some(Path::new("c.milk")),
            "the history entry next"
        );
        presets.next(false);
        presets.take_request();
        presets.next(false);
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("d.milk"), true)),
            "and a new pick only past the end"
        );
    }

    /// R changes the order, so a pick made in the other one is dropped.
    #[test]
    fn changing_the_order_chooses_again() {
        let mut presets = Presets::new();
        presets.files = ["a.milk", "b.milk", "c.milk", "d.milk"]
            .map(PathBuf::from)
            .to_vec();
        presets.next(false);
        presets.take_request();
        let current = presets.current().unwrap().to_path_buf();
        let at = presets
            .files
            .iter()
            .position(|file| file == &current)
            .unwrap();
        let in_order = presets.files[(at + 1) % presets.files.len()].clone();
        assert!(!presets.toggle_order(), "R turns the random order off");
        assert_eq!(
            presets.lookahead(),
            Some(in_order.as_path()),
            "folder order, not the random pick made before"
        );
        presets.next(false);
        assert_eq!(asked_for(&mut presets), Some((in_order, true)));
    }

    /// A kept preset stays, and the pick ahead waits for when it is free.
    #[test]
    fn a_kept_preset_keeps_its_pick_ahead() {
        let mut presets = playing(&["a.milk", "b.milk"]);
        assert_eq!(presets.lookahead(), Some(Path::new("b.milk")));
        presets.locked = true;
        presets.advance(presets.started + Duration::from_secs(3600), 10);
        assert_eq!(asked_for(&mut presets), None);
        assert_eq!(presets.lookahead(), Some(Path::new("b.milk")));
        presets.locked = false;
        presets.advance(presets.started + Duration::from_secs(3600), 10);
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("b.milk"), true))
        );
    }

    /// A new listing replaces the files, so a pick from the old one that is
    /// no longer there is not used.
    #[test]
    fn a_new_listing_replaces_the_pick_ahead() {
        let dir = temp_dir("milkdrop-lookahead-listing");
        let (a, b, c) = (dir.join("a.milk"), dir.join("b.milk"), dir.join("c.milk"));
        std::fs::write(&a, b"").unwrap();
        std::fs::write(&b, b"").unwrap();
        let mut presets = Presets::new();
        presets.random = false;
        let started = Instant::now();
        while presets.count() < 2 && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
            presets.refresh(&dir, None);
        }
        presets.next(false);
        presets.take_request();
        assert_eq!(presets.lookahead(), Some(b.as_path()));

        std::fs::remove_file(&b).unwrap();
        std::fs::write(&c, b"").unwrap();
        presets.listed = None;
        let started = Instant::now();
        while presets.lookahead() != Some(c.as_path())
            && started.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(5));
            presets.refresh(&dir, None);
        }
        let ahead = presets.lookahead().map(Path::to_path_buf);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(ahead, Some(c), "the preset now after a, not the one gone");
    }

    /// A preset whose switch is too slow is passed over when the app
    /// changes preset by itself, and the following one plays.
    #[test]
    fn automatic_play_passes_over_a_slow_preset() {
        let mut presets = playing(&["a.milk", "b.milk", "c.milk"]);
        presets.record_switch(Path::new("b.milk"), Duration::from_millis(120));
        let start = presets.started;
        presets.advance(start + Duration::from_secs(10), 10);
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("c.milk"), true))
        );
        assert_eq!(presets.current(), Some(Path::new("c.milk")));
    }

    /// A key press still reaches it: only the app's own choices avoid it.
    #[test]
    fn a_key_still_reaches_a_slow_preset() {
        let mut presets = playing(&["a.milk", "b.milk", "c.milk"]);
        presets.record_switch(Path::new("b.milk"), Duration::from_millis(120));
        presets.next(false);
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("b.milk"), true))
        );
        assert_eq!(presets.current(), Some(Path::new("b.milk")));
    }

    /// So does Previous, walking back through what has played.
    #[test]
    fn previous_still_reaches_a_slow_preset() {
        let mut presets = playing(&["a.milk", "b.milk", "c.milk"]);
        presets.next(false);
        presets.take_request();
        presets.record_switch(Path::new("b.milk"), Duration::from_millis(120));
        presets.next(false);
        presets.take_request();
        presets.previous();
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("b.milk"), true))
        );
    }

    /// Automatic play does not walk forward into a slow entry of history
    /// either; it chooses again past it.
    #[test]
    fn automatic_play_leaves_a_slow_history_entry_behind() {
        let mut presets = playing(&["a.milk", "b.milk", "c.milk", "d.milk"]);
        presets.next(false);
        presets.take_request();
        presets.next(false);
        presets.take_request();
        assert_eq!(presets.current(), Some(Path::new("c.milk")));
        presets.previous();
        presets.take_request();
        assert_eq!(presets.current(), Some(Path::new("b.milk")));
        presets.record_switch(Path::new("c.milk"), Duration::from_millis(120));
        presets.advance(presets.started + Duration::from_secs(10), 10);
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("d.milk"), true)),
            "c is next in history but too slow to switch into"
        );
    }

    /// In random order automatic play only samples the presets that are
    /// not slow.
    #[test]
    fn automatic_play_never_picks_a_slow_preset_at_random() {
        let mut presets = Presets::new();
        presets.files = ["a.milk", "b.milk", "c.milk", "d.milk"]
            .map(PathBuf::from)
            .to_vec();
        presets.next(false);
        presets.take_request();
        let current = presets.current().unwrap().to_path_buf();
        let fast = presets
            .files
            .iter()
            .find(|file| **file != current)
            .unwrap()
            .clone();
        let files = presets.files.clone();
        for file in &files {
            if file != &fast {
                presets.record_switch(file, Duration::from_millis(120));
            }
        }
        presets.advance(presets.started + Duration::from_secs(10), 10);
        assert_eq!(presets.current(), Some(fast.as_path()));
        assert_eq!(asked_for(&mut presets), Some((fast, true)));
    }

    /// When every preset is slow the picture keeps changing: the list is a
    /// preference, not a reason to stop.
    #[test]
    fn automatic_play_goes_on_when_every_preset_is_slow() {
        let mut presets = playing(&["a.milk", "b.milk"]);
        presets.record_switch(Path::new("a.milk"), Duration::from_millis(120));
        presets.record_switch(Path::new("b.milk"), Duration::from_millis(120));
        presets.advance(presets.started + Duration::from_secs(10), 10);
        assert_eq!(
            asked_for(&mut presets),
            Some((PathBuf::from("b.milk"), true))
        );
    }

    /// The pick a key plays ahead is the next file even when it is slow,
    /// so the file read early is still useful.
    #[test]
    fn a_key_plays_the_slow_pick_ahead() {
        let mut presets = playing(&["a.milk", "b.milk", "c.milk"]);
        presets.record_switch(Path::new("b.milk"), Duration::from_millis(120));
        let ahead = presets.lookahead().unwrap().to_path_buf();
        assert_eq!(ahead, PathBuf::from("b.milk"));
        presets.next(false);
        assert_eq!(asked_for(&mut presets), Some((ahead, true)));
    }

    #[test]
    fn a_pack_is_unpacked_flat() {
        let dir = temp_dir("milkdrop-pack");
        let zip = crate::skin::zip::write(&[
            (
                "pack-master/Dancer/One.milk",
                b"[preset00]".as_slice(),
                true,
            ),
            ("pack-master/README.md", b"hello".as_slice(), true),
            ("pack-master/Two.milk", b"[preset00]".as_slice(), false),
        ]);
        assert_eq!(unpack_presets(&zip, &dir), Ok(2));
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["One.milk", "Two.milk"]);
    }

    #[test]
    fn a_pack_cannot_name_a_file_outside_its_folder() {
        for name in [
            "C:Evil.milk",
            "pack/D:Evil.milk",
            "Good.milk:stream.milk",
            "..",
        ] {
            assert!(!is_plain_file_name(name), "{name}");
        }
        assert!(is_plain_file_name("Good One.milk"));
        let dir = temp_dir("milkdrop-confined");
        let zip = crate::skin::zip::write(&[
            ("pack/C:Evil.milk", b"[preset00]".as_slice(), false),
            ("pack/Good.milk", b"[preset00]".as_slice(), false),
        ]);
        let written = unpack_presets(&zip, &dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written, Ok(1));
    }
}
