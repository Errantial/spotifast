//! Reading the next preset's file while the one playing is on screen.
//!
//! A switch used to read the file on the render thread, in the same call
//! that parses it and compiles its shaders. The read is the part that can
//! happen earlier: [`Presets::lookahead`](super::Presets::lookahead) knows
//! the next preset before the switch, so a worker thread reads its bytes and
//! hands them to projectM to parse and compile ahead
//! ([`Engine::prepare_data`]); the render thread's load of the same bytes
//! then takes that work over.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

use super::engine::Engine;

/// The file read ahead, and the thread that reads it.
pub struct Prefetch {
    /// Asks the worker for a path; `None` once the thread could not start
    /// or has gone.
    requests: Option<Sender<PathBuf>>,
    /// What the worker read, and how it went.
    read: Receiver<(PathBuf, io::Result<Vec<u8>>)>,
    /// The path asked for last, so a frame does not ask again.
    wanted: Option<PathBuf>,
    /// Bytes read and not yet used, with the path they came from.
    ready: Option<(PathBuf, Vec<u8>)>,
    worker: Option<JoinHandle<()>>,
}

impl Prefetch {
    /// Starts the worker. A thread that cannot start leaves every switch
    /// reading its file as before.
    pub fn start() -> Self {
        let (requests, asked) = mpsc::channel::<PathBuf>();
        let (read, readings) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("milkdrop-prefetch".into())
            .spawn(move || {
                while let Ok(path) = asked.recv() {
                    // Only the newest request matters: a burst of Next
                    // presses leaves the presets in between unread.
                    let mut path = path;
                    while let Ok(newer) = asked.try_recv() {
                        path = newer;
                    }
                    let read_ahead = bytes(&path);
                    if let Ok(data) = &read_ahead {
                        // The expensive part of a switch that needs no GL:
                        // done here, while the playing preset draws.
                        Engine::prepare_data(data);
                    }
                    if read.send((path.clone(), read_ahead)).is_err() {
                        break;
                    }
                }
            });
        match spawned {
            Ok(worker) => Self {
                requests: Some(requests),
                read: readings,
                wanted: None,
                ready: None,
                worker: Some(worker),
            },
            Err(error) => {
                log::warn!("could not start reading presets ahead: {error}");
                Self {
                    requests: None,
                    read: readings,
                    wanted: None,
                    ready: None,
                    worker: None,
                }
            }
        }
    }

    /// Asks for the preset the next switch will play. A frame that asks for
    /// the same one again does nothing.
    pub fn want(&mut self, next: Option<&Path>) {
        let Some(next) = next else {
            return;
        };
        if self.wanted.as_deref() == Some(next) {
            return;
        }
        let Some(requests) = &self.requests else {
            return;
        };
        let path = next.to_path_buf();
        if requests.send(path.clone()).is_ok() {
            self.wanted = Some(path);
        }
    }

    /// The bytes read ahead for `path`, when they are ready; this never
    /// waits. A miss leaves the caller to load the file, which is what a
    /// manual Next needs while the read is still under way.
    pub fn take(&mut self, path: &Path) -> Option<Vec<u8>> {
        loop {
            match self.read.try_recv() {
                Ok((read_path, Ok(bytes))) => {
                    // Keep what the next switch will want even while
                    // another preset is played: Previous must not throw
                    // the read ahead away.
                    let wanted = self.wanted.as_deref() == Some(read_path.as_path());
                    if read_path.as_path() == path || wanted {
                        self.ready = Some((read_path, bytes));
                    }
                }
                Ok((read_path, Err(error))) => {
                    // A preset that will not load is reported by the switch
                    // that tries it, with its name; this only says why the
                    // early read failed.
                    log::debug!(
                        "could not read preset ahead {}: {error}",
                        read_path.display()
                    );
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.requests = None;
                    break;
                }
            }
        }
        match self.ready.take() {
            Some((read_path, bytes)) if read_path.as_path() == path => {
                // These are being used, so the next preset has to be asked
                // for, even if it is this one again.
                self.wanted = None;
                Some(bytes)
            }
            other => {
                self.ready = other;
                None
            }
        }
    }

    /// Ends the worker and waits for it, so nothing is left behind when the
    /// window closes. A preset is a small file, so the wait is short.
    fn stop(&mut self) {
        self.requests = None;
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            log::warn!("reading presets ahead ended unexpectedly");
        }
    }
}

impl Drop for Prefetch {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A preset's contents, or why they could not be read. Only ordinary files
/// are read: one that is not (a named pipe named `.milk`, say) would block
/// the worker, and the switch that finds nothing read ahead loads that file
/// itself as it always did.
fn bytes(path: &Path) -> io::Result<Vec<u8>> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    std::fs::read(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spotifast-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Asks for a preset and waits for the read, as a frame would over the
    /// preset's turn on screen.
    fn read_ahead(prefetch: &mut Prefetch, path: &Path) -> Option<Vec<u8>> {
        prefetch.want(Some(path));
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            if let Some(bytes) = prefetch.take(path) {
                return Some(bytes);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    #[test]
    fn the_next_file_is_read_before_it_is_needed() {
        let dir = temp_dir("milkdrop-prefetch");
        let path = dir.join("a.milk");
        std::fs::write(&path, b"[preset00]\n").unwrap();
        let mut prefetch = Prefetch::start();
        let read = read_ahead(&mut prefetch, &path);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(read.as_deref(), Some(&b"[preset00]\n"[..]));
    }

    /// A preset removed or unreadable after it was asked for is a miss, and
    /// the worker lives to read the next one.
    #[test]
    fn a_file_that_cannot_be_read_is_a_miss() {
        let dir = temp_dir("milkdrop-prefetch-missing");
        let missing = dir.join("gone.milk");
        let here = dir.join("here.milk");
        std::fs::write(&here, b"preset").unwrap();
        let mut prefetch = Prefetch::start();
        prefetch.want(Some(&missing));
        assert_eq!(prefetch.take(&missing), None);
        let read = read_ahead(&mut prefetch, &here);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(read.as_deref(), Some(&b"preset"[..]));
    }

    /// The bytes for the preset planned are kept when another one plays:
    /// Previous while a read is under way must not throw them away.
    #[test]
    fn bytes_for_the_planned_preset_wait_for_it() {
        let dir = temp_dir("milkdrop-prefetch-kept");
        let planned = dir.join("planned.milk");
        let playing = dir.join("playing.milk");
        std::fs::write(&planned, b"planned").unwrap();
        let mut prefetch = Prefetch::start();
        prefetch.want(Some(&planned));
        let started = Instant::now();
        while prefetch.ready.is_none() && started.elapsed() < Duration::from_secs(10) {
            assert_eq!(
                prefetch.take(&playing),
                None,
                "another preset takes none of it"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(prefetch.take(&planned).as_deref(), Some(&b"planned"[..]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Closing the window joins the worker; nothing is left running.
    #[test]
    fn stopping_joins_the_worker() {
        let mut prefetch = Prefetch::start();
        assert!(prefetch.worker.is_some());
        prefetch.stop();
        assert!(prefetch.worker.is_none(), "no thread left behind");
        assert_eq!(prefetch.take(Path::new("nowhere.milk")), None);
    }
}
