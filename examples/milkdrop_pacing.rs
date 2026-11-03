//! Measures how punctually the MilkDrop window's frame timer wakes when it
//! is asked for a fixed number of frames a second.
//!
//! The window draws with `SwapInterval::DontWait` and paces itself with a
//! timer, so the interval between frames is whatever the event loop's wait
//! makes of it: this records that interval and how late each wake is against
//! the deadline the window asked for. It is the timer half of the pacing
//! comparison in `PERF_NOTES.md`; the display half (when a frame reaches the
//! glass) needs a real screen.
//!
//! Run it at the screen's rate:
//!
//! ```sh
//! sfb gl cargo run --release --example milkdrop_pacing -- 60
//! ```

use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};

/// Frames measured, so ten seconds at 60 a second.
const FRAMES: usize = 600;

fn main() {
    let fps = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse::<u32>().ok())
        .unwrap_or(60);
    let interval = Duration::from_secs_f32(1.0 / fps as f32);
    let event_loop = EventLoop::new().expect("an event loop, which needs a display on X11");
    let mut probe = Probe::new(interval);
    let _ = event_loop.run_app(&mut probe);
    probe.report(fps);
}

struct Probe {
    interval: Duration,
    /// When the next frame is due.
    next: Instant,
    /// How long after its deadline each frame actually started.
    late: Vec<Duration>,
    /// How long each frame after the first took, start to start.
    gaps: Vec<Duration>,
    last: Option<Instant>,
}

impl Probe {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: Instant::now() + interval,
            late: Vec::new(),
            gaps: Vec::new(),
            last: None,
        }
    }
}

impl ApplicationHandler for Probe {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        _event: WindowEvent,
    ) {
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.gaps.len() >= FRAMES - 1 {
            event_loop.exit();
            return;
        }
        if now >= self.next {
            if let Some(last) = self.last {
                self.gaps.push(now.saturating_duration_since(last));
            }
            self.last = Some(now);
            self.late.push(now.saturating_duration_since(self.next));
            // `Child::schedule_next_frame_at`, step for step: the deadline
            // moves on from the last one, and a missed stretch is let go
            // rather than chased.
            self.next += self.interval;
            if self.next <= now {
                self.next = now + self.interval;
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next));
    }
}

impl Probe {
    fn report(&self, fps: u32) {
        println!(
            "asked for {fps} fps ({} us), {} frames measured",
            self.interval.as_micros(),
            self.gaps.len()
        );
        let ideal = self.interval.as_secs_f64() * 1e6;
        stats("frame interval (us)", &self.gaps, ideal);
        stats("wake lateness (us)", &self.late, 0.0);
        let interval = self.interval.as_secs_f64();
        let held = self
            .gaps
            .iter()
            .filter(|gap| gap.as_secs_f64() > interval * 1.5)
            .count();
        let early = self
            .gaps
            .iter()
            .filter(|gap| gap.as_secs_f64() < interval * 0.5)
            .count();
        println!(
            "gaps over 1.5 intervals: {held} ({:.2}%), under 0.5: {early} ({:.2}%)",
            100.0 * held as f64 / self.gaps.len().max(1) as f64,
            100.0 * early as f64 / self.gaps.len().max(1) as f64
        );
    }
}

/// Mean, standard deviation and percentiles of `samples` in microseconds,
/// against `ideal`.
fn stats(name: &str, samples: &[Duration], ideal: f64) {
    if samples.is_empty() {
        println!("{name}: nothing measured");
        return;
    }
    let mut values: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1e6).collect();
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    let at = |share: f64| values[((values.len() - 1) as f64 * share).round() as usize];
    println!(
        "{name}: mean {mean:.1} (ideal {ideal:.1}), sd {:.1}, p50 {:.1}, p99 {:.1}, max {:.1}",
        variance.sqrt(),
        at(0.50),
        at(0.99),
        values[values.len() - 1]
    );
}
