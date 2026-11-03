//! Measures what a MilkDrop preset switch costs the frame that loads it.
//!
//! It draws exactly the way the MilkDrop window does (a hidden window with
//! the same OpenGL setup, soft cuts through `Engine::load`, a 60 fps
//! software clock) but takes its audio from a synthetic signal and its
//! presets from a folder, switching on a fixed beat. That makes the hitch
//! reproducible and gives numbers to compare before and after a fix.
//!
//! Presets are not in the repository. On the harness machine, run once with
//! `--fetch` to download the two packs the app offers into the preset
//! folder (they are not committed); later runs reuse them.
//!
//! ```sh
//! sfb gl cargo run --release --example milkdrop_stutter -- --fetch
//! sfb gl cargo run --release --example milkdrop_stutter -- --count 200
//! sfb gl cargo run --release --example milkdrop_stutter -- --hard
//! ```
//!
//! Software GL (llvmpipe) draws on the CPU: absolute milliseconds say how
//! this machine behaves, and only their ratios are a fair hint of a real
//! GPU. The renderer is printed so a result is never mistaken for one.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::glow::{self, HasContext};
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use spotifast::milkdrop::engine::Engine;
use spotifast::milkdrop::shm::FRAMES;
use spotifast::milkdrop::{CROSSFADE_SECONDS, PACKS, SWITCH_BUDGET, fetch_pack, list_presets};
use spotifast::settings::ProxyConfig;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Samples a second the engine is fed, the same rate the desktop mixes at.
const SAMPLE_RATE: u32 = 48_000;
/// A load slower than this blocks the render thread, which the plan's
/// success criteria call out.
const BUDGET_MS: f64 = 8.0;
/// A switch (load and first draw) over this is kept out of automatic play
/// by the engine's own policy; the harness counts the same presets.
const SWITCH_BUDGET_MS: f64 = SWITCH_BUDGET.as_millis() as f64;

fn main() {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return,
        Err(error) => {
            eprintln!("milkdrop_stutter: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(options) {
        eprintln!("milkdrop_stutter: {error}");
        std::process::exit(1);
    }
}

struct Options {
    presets: PathBuf,
    count: usize,
    seconds: f64,
    fps: u32,
    width: u32,
    height: u32,
    hard: bool,
    fetch: bool,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut options = Self {
            presets: std::env::temp_dir().join("spotifast-presets"),
            count: 200,
            seconds: 3.0,
            fps: 60,
            width: 640,
            height: 480,
            hard: false,
            fetch: false,
        };
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--presets" => {
                    options.presets = PathBuf::from(args.next().ok_or("--presets needs a folder")?);
                }
                "--count" => {
                    options.count = args
                        .next()
                        .ok_or("--count needs a number")?
                        .parse()
                        .map_err(|_| "--count needs a number")?;
                    if options.count == 0 {
                        return Err("--count needs a number above zero".into());
                    }
                }
                "--seconds" => {
                    options.seconds = args
                        .next()
                        .ok_or("--seconds needs a number")?
                        .parse()
                        .map_err(|_| "--seconds needs a number")?;
                    if options.seconds <= 0.0 {
                        return Err("--seconds needs a positive number".into());
                    }
                }
                "--fps" => {
                    options.fps = args
                        .next()
                        .ok_or("--fps needs a number")?
                        .parse()
                        .map_err(|_| "--fps needs a number")?;
                    if options.fps == 0 {
                        return Err("--fps needs a number above zero".into());
                    }
                }
                "--size" => {
                    let value = args.next().ok_or("--size needs WIDTHxHEIGHT")?;
                    let (width, height) = value
                        .split_once(['x', 'X', ','])
                        .ok_or("--size needs WIDTHxHEIGHT")?;
                    options.width = width
                        .trim()
                        .parse()
                        .map_err(|_| "--size needs WIDTHxHEIGHT")?;
                    options.height = height
                        .trim()
                        .parse()
                        .map_err(|_| "--size needs WIDTHxHEIGHT")?;
                }
                "--hard" => options.hard = true,
                "--fetch" => options.fetch = true,
                "--help" | "-h" => {
                    println!(
                        "usage: milkdrop_stutter [--presets DIR] [--count N] [--seconds S]\n\
                         \x20      [--fps N] [--size WxH] [--hard] [--fetch]\n\
                         \n\
                         Switches through N presets, measuring each load and the frames\n\
                         after it. --fetch downloads the preset packs into DIR first.\n\
                         --hard cuts instead of fading, for comparison."
                    );
                    return Ok(None);
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(Some(options))
    }
}

fn run(options: Options) -> Result<(), String> {
    if options.fetch {
        fetch_packs(&options.presets)?;
    }
    let all = list_presets(&options.presets);
    if all.is_empty() {
        return Err(format!(
            "no .milk presets in {}: run once with --fetch",
            options.presets.display()
        ));
    }
    let presets = sample(&all, options.count);
    println!(
        "milkdrop_stutter: {} of {} presets from {}, every {:.2}s, {} fps, {} cuts, {}x{}",
        presets.len(),
        all.len(),
        options.presets.display(),
        options.seconds,
        options.fps,
        if options.hard { "hard" } else { "soft" },
        options.width,
        options.height,
    );
    let event_loop = EventLoop::new().map_err(|error| format!("no event loop: {error}"))?;
    let mut harness = Harness {
        options,
        presets,
        live: None,
        bench: None,
        code: 0,
    };
    event_loop
        .run_app(&mut harness)
        .map_err(|error| format!("event loop failed: {error}"))?;
    if harness.code == 0 {
        Ok(())
    } else {
        Err("the harness could not start".into())
    }
}

/// Downloads the packs the app offers; a preset is small and the two packs
/// are what the app itself would fetch into a fresh MilkDrop folder.
fn fetch_packs(folder: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let proxy = ProxyConfig::System;
    for pack in &PACKS {
        eprintln!("milkdrop_stutter: fetching {} ({})", pack.name, pack.note);
        let written = fetch_pack(pack, folder, &proxy)?;
        eprintln!(
            "milkdrop_stutter: wrote {written} presets into {}",
            folder.display()
        );
    }
    Ok(())
}

/// `count` presets spread over the folder in its own order, so a run covers
/// as many authors as the folder holds and repeats the same sample.
fn sample(files: &[PathBuf], count: usize) -> Vec<PathBuf> {
    if count == 0 || files.len() <= count {
        return files.to_vec();
    }
    (0..count)
        .map(|at| files[at * files.len() / count].clone())
        .collect()
}

/// The hidden window, its OpenGL context and projectM, built the way
/// `milkdrop::child::build` builds the window's.
struct Live {
    /// Freed first: projectM's shutdown makes GL calls, so the context must
    /// still be current.
    engine: Engine,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
}

fn build(event_loop: &ActiveEventLoop, options: &Options) -> Result<Live, String> {
    let attributes = Window::default_attributes()
        .with_title("MilkDrop switch harness")
        .with_visible(false)
        .with_decorations(false)
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(
            options.width as f32,
            options.height as f32,
        ));
    // Opaque, as the MilkDrop window asks for.
    let template = ConfigTemplateBuilder::new().with_alpha_size(0);
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(event_loop, template, |mut configs| {
            configs.next().expect("a GL config for the harness")
        })
        .map_err(|error| format!("no GL config: {error}"))?;
    let window = window.ok_or_else(|| "the window was not created".to_string())?;
    let raw = window
        .window_handle()
        .map_err(|error| format!("no window handle: {error}"))?
        .as_raw();
    let context_attributes = ContextAttributesBuilder::new().build(Some(raw));
    // SAFETY: the display outlives the context, and a raw window handle is
    // what glutin reads it with.
    let not_current = unsafe {
        config
            .display()
            .create_context(&config, &context_attributes)
    }
    .map_err(|error| format!("no GL context: {error}"))?;
    let inner = window.inner_size();
    let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw,
        NonZeroU32::new(inner.width).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(inner.height).unwrap_or(NonZeroU32::MIN),
    );
    // SAFETY: the surface is made for this window, which outlives it.
    let surface = unsafe {
        config
            .display()
            .create_window_surface(&config, &surface_attributes)
    }
    .map_err(|error| format!("no GL surface: {error}"))?;
    let context = not_current
        .make_current(&surface)
        .map_err(|error| format!("could not make the context current: {error}"))?;
    // The same pacing as the window: a software clock decides the frame,
    // so a swap never waits for vsync.
    let _ = surface.set_swap_interval(&context, SwapInterval::DontWait);
    // SAFETY: the context is current on this thread.
    let gl = Arc::new(unsafe {
        glow::Context::from_loader_function_cstr(|symbol| config.display().get_proc_address(symbol))
    });
    // SAFETY: the context is current.
    unsafe {
        println!(
            "milkdrop_stutter: renderer: {}; vendor: {}; version: {}",
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VENDOR),
            gl.get_parameter_string(glow::VERSION),
        );
    }
    let textures = options.presets.join("textures");
    let dirs = [options.presets.as_path(), textures.as_path()];
    let engine = Engine::new(Arc::clone(&gl), &dirs, 60)
        .ok_or_else(|| "projectM needs OpenGL 3.3".to_string())?;
    Ok(Live {
        engine,
        context,
        surface,
        window,
    })
}

struct Harness {
    options: Options,
    presets: Vec<PathBuf>,
    live: Option<Live>,
    bench: Option<Bench>,
    code: i32,
}

impl ApplicationHandler for Harness {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }
        match build(event_loop, &self.options) {
            Ok(live) => {
                self.live = Some(live);
                self.bench = Some(Bench::new(
                    self.presets.clone(),
                    Duration::from_secs_f64(self.options.seconds),
                    self.options.fps,
                    self.options.hard,
                ));
            }
            Err(error) => {
                eprintln!("milkdrop_stutter: {error}");
                self.code = 1;
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, _event: WindowEvent) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(live), Some(bench)) = (&mut self.live, &mut self.bench) else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        if bench.done() {
            bench.summarize();
            event_loop.exit();
            return;
        }
        if Instant::now() >= bench.next_frame {
            bench.frame(live);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(bench.next_frame));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // The engine frees projectM with GL calls while the context is
        // current, so it goes before the context does.
        self.live = None;
    }
}

/// One preset switch: when it started, when its fade ends and every frame
/// that was drawn while it was the preset on show.
struct Switch {
    name: String,
    load_ms: f64,
    /// The first draw of the new preset, where its shaders compile.
    first_draw_ms: f64,
    /// When the interval this preset holds ends, and its fade does.
    end: Instant,
    fade_start: Instant,
    fade_end: Instant,
    /// Every frame drawn while it played: when it began, when it ended and
    /// how long it took in milliseconds.
    frames: Vec<(Instant, Instant, f64)>,
}

struct Bench {
    presets: Vec<PathBuf>,
    next: usize,
    interval: Duration,
    fps: u32,
    hard: bool,
    current: Option<Switch>,
    next_frame: Instant,
    started: Instant,
    last_frame: Instant,
    audio: Audio,
    audio_at: Instant,
    loads: Vec<(String, f64)>,
    /// The first draw of each preset, apart from its load.
    draws: Vec<(String, f64)>,
    /// Load and first draw together: what a switch cost its frame.
    costs: Vec<(String, f64)>,
    /// Stereo frames of audio handed to projectM each frame.
    samples: Vec<usize>,
    crossfade: Vec<f64>,
    steady: Vec<f64>,
    worsts: Vec<f64>,
    first_second: Vec<f64>,
    stalls: Vec<f64>,
    frames: u64,
    summarized: bool,
}

impl Bench {
    fn new(presets: Vec<PathBuf>, interval: Duration, fps: u32, hard: bool) -> Self {
        let now = Instant::now();
        Self {
            presets,
            next: 0,
            interval,
            fps,
            hard,
            current: None,
            next_frame: now,
            started: now,
            last_frame: now,
            audio: Audio::new(),
            audio_at: now,
            loads: Vec::new(),
            draws: Vec::new(),
            costs: Vec::new(),
            samples: Vec::new(),
            crossfade: Vec::new(),
            steady: Vec::new(),
            worsts: Vec::new(),
            first_second: Vec::new(),
            stalls: Vec::new(),
            frames: 0,
            summarized: false,
        }
    }

    fn done(&self) -> bool {
        self.current.is_none() && self.next >= self.presets.len()
    }

    fn frame(&mut self, live: &mut Live) {
        let at = Instant::now();
        let switched = self.current.is_none();
        if switched {
            self.start_switch(live);
        }
        // The sound the app's ring would have caught up with, a burst after
        // a long frame included. The cap is the ring's own: `Ring::since`
        // cannot return more than it holds.
        let elapsed = at.saturating_duration_since(self.audio_at);
        let samples = ((elapsed.as_secs_f64() * f64::from(SAMPLE_RATE)) as usize).min(FRAMES);
        self.audio_at = at;
        self.samples.push(samples);
        let pcm = self.audio.next(samples);
        live.engine.feed_frames(&pcm);
        // The first draw of a newly loaded preset is timed apart from its
        // load: this is where the driver compiles and links its shaders.
        let draw_started = Instant::now();
        let size = live.window.inner_size();
        live.engine
            .render(0, 0, size.width.max(1), size.height.max(1), 1);
        let draw = draw_started.elapsed().as_secs_f64() * 1_000.0;
        if switched && let Some(current) = &mut self.current {
            current.first_draw_ms = draw;
        }
        let drawn = Instant::now();
        let took = drawn.saturating_duration_since(self.last_frame);
        self.last_frame = drawn;
        let began = drawn.checked_sub(took).unwrap_or(drawn);
        if let Some(current) = &mut self.current {
            current
                .frames
                .push((began, drawn, took.as_secs_f64() * 1_000.0));
        }
        self.frames += 1;
        let _ = live.surface.swap_buffers(&live.context);
        // The same clock the window keeps: from the last deadline, and let
        // go when it has already passed.
        let interval = Duration::from_secs_f32(1.0 / self.fps.max(1) as f32);
        self.next_frame += interval;
        let after = Instant::now();
        if self.next_frame <= after {
            self.next_frame = after + interval;
        }
        if let Some(current) = &self.current
            && drawn >= current.end
        {
            self.finish_switch();
        }
    }

    fn start_switch(&mut self, live: &mut Live) {
        let Some(path) = self.presets.get(self.next).cloned() else {
            return;
        };
        self.next += 1;
        let started = Instant::now();
        live.engine.load(&path, !self.hard);
        let load = started.elapsed();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.loads
            .push((name.clone(), load.as_secs_f64() * 1_000.0));
        self.current = Some(Switch {
            name,
            load_ms: load.as_secs_f64() * 1_000.0,
            first_draw_ms: 0.0,
            end: started + self.interval,
            fade_start: started + load,
            fade_end: started + load + Duration::from_secs_f64(CROSSFADE_SECONDS),
            frames: Vec::new(),
        });
    }

    fn finish_switch(&mut self) {
        let Some(current) = self.current.take() else {
            return;
        };
        // The frame that paid for the load began before the fade did. Its
        // whole time is what a person sees as the hitch.
        let stall = current
            .frames
            .iter()
            .find(|(began, _, _)| *began < current.fade_start)
            .map(|(_, _, ms)| *ms)
            .unwrap_or(0.0);
        let switch = current.load_ms + current.first_draw_ms;
        // A frame that began before the fade did is the one that paid for
        // the load; it is the stall, not the crossfade's cost.
        let crossfade: Vec<f64> = current
            .frames
            .iter()
            .filter(|(began, _, _)| *began >= current.fade_start && *began < current.fade_end)
            .map(|(_, _, ms)| *ms)
            .collect();
        let steady: Vec<f64> = current
            .frames
            .iter()
            .filter(|(began, _, _)| *began >= current.fade_end)
            .map(|(_, _, ms)| *ms)
            .collect();
        let worst = current
            .frames
            .iter()
            .map(|(_, _, ms)| *ms)
            .fold(0.0, f64::max);
        // The plan's criterion is the worst frame in the second after a
        // switch, which starts when the load does.
        let after_one = current.end - self.interval + Duration::from_secs(1);
        let worst_first_second = current
            .frames
            .iter()
            .filter(|(_, end, _)| *end <= after_one)
            .map(|(_, _, ms)| *ms)
            .fold(0.0, f64::max);
        println!(
            "milkdrop_stutter: switch {:>3}/{} {:?} load={:.1}ms first_draw={:.1}ms switch={:.1}ms stall={:.1}ms other={:.1}ms worst_{:.0}s={:.1}ms frames={} crossfade[p50={:.1} p99={:.1} n={}] steady[p50={:.1} p99={:.1} n={}]",
            self.next,
            self.presets.len(),
            current.name,
            current.load_ms,
            current.first_draw_ms,
            switch,
            stall,
            (stall - switch).max(0.0),
            self.interval.as_secs_f64(),
            worst,
            current.frames.len(),
            percentile(&crossfade, 0.5),
            percentile(&crossfade, 0.99),
            crossfade.len(),
            percentile(&steady, 0.5),
            percentile(&steady, 0.99),
            steady.len(),
        );
        self.draws
            .push((current.name.clone(), current.first_draw_ms));
        self.costs.push((current.name.clone(), switch));
        self.stalls.push(stall);
        self.worsts.push(worst);
        self.first_second.push(worst_first_second);
        self.crossfade.extend(crossfade);
        self.steady.extend(steady);
    }

    fn summarize(&mut self) {
        if self.summarized {
            return;
        }
        self.summarized = true;
        // A last switch whose interval ended after the loop checked.
        self.finish_switch();
        let elapsed = self.last_frame.saturating_duration_since(self.started);
        let loads: Vec<f64> = self.loads.iter().map(|(_, ms)| *ms).collect();
        let draws: Vec<f64> = self.draws.iter().map(|(_, ms)| *ms).collect();
        let costs: Vec<f64> = self.costs.iter().map(|(_, ms)| *ms).collect();
        let over_budget = loads.iter().filter(|ms| **ms > BUDGET_MS).count();
        let over_switch = costs.iter().filter(|ms| **ms > SWITCH_BUDGET_MS).count();
        println!();
        println!(
            "milkdrop_stutter: summary: {} presets, {} frames in {:.1}s, {:.1} fps, {} cuts",
            self.loads.len(),
            self.frames,
            elapsed.as_secs_f64(),
            self.frames as f64 / elapsed.as_secs_f64().max(f64::EPSILON),
            if self.hard { "hard" } else { "soft" },
        );
        println!(
            "  preset load ms:        p50={:.1} p99={:.1} max={:.1} ({} over {:.0}ms)",
            percentile(&loads, 0.5),
            percentile(&loads, 0.99),
            percentile(&loads, 1.0),
            over_budget,
            BUDGET_MS,
        );
        println!(
            "  first draw ms:         p50={:.1} p99={:.1} max={:.1}",
            percentile(&draws, 0.5),
            percentile(&draws, 0.99),
            percentile(&draws, 1.0),
        );
        println!(
            "  switch cost ms:        p50={:.1} p99={:.1} max={:.1} ({} over {:.0}ms budget)",
            percentile(&costs, 0.5),
            percentile(&costs, 0.99),
            percentile(&costs, 1.0),
            over_switch,
            SWITCH_BUDGET_MS,
        );
        let samples: Vec<f64> = self.samples.iter().map(|n| *n as f64).collect();
        let frame_audio = f64::from(SAMPLE_RATE) / f64::from(self.fps.max(1));
        let bursts = samples.iter().filter(|n| **n > 2.0 * frame_audio).count();
        println!(
            "  pcm per frame:         p50={:.0} p99={:.0} max={:.0} samples ({} over two frames of {:.0})",
            percentile(&samples, 0.5),
            percentile(&samples, 0.99),
            percentile(&samples, 1.0),
            bursts,
            frame_audio,
        );
        println!(
            "  frame ms during fade:  p50={:.1} p99={:.1} max={:.1} (n={})",
            percentile(&self.crossfade, 0.5),
            percentile(&self.crossfade, 0.99),
            percentile(&self.crossfade, 1.0),
            self.crossfade.len(),
        );
        println!(
            "  frame ms steady:       p50={:.1} p99={:.1} max={:.1} (n={})",
            percentile(&self.steady, 0.5),
            percentile(&self.steady, 0.99),
            percentile(&self.steady, 1.0),
            self.steady.len(),
        );
        println!(
            "  worst frame per switch p50={:.1} p99={:.1} max={:.1}",
            percentile(&self.worsts, 0.5),
            percentile(&self.worsts, 0.99),
            percentile(&self.worsts, 1.0),
        );
        println!(
            "  worst in 1s after switch p50={:.1} p99={:.1} max={:.1}",
            percentile(&self.first_second, 0.5),
            percentile(&self.first_second, 0.99),
            percentile(&self.first_second, 1.0),
        );
        println!(
            "  stall frame ms:        p50={:.1} p99={:.1} max={:.1}",
            percentile(&self.stalls, 0.5),
            percentile(&self.stalls, 0.99),
            percentile(&self.stalls, 1.0),
        );
        let steady_p99 = percentile(&self.steady, 0.99);
        if steady_p99 > 0.0 {
            println!(
                "  fade p99 / steady p99: {:.2}x",
                percentile(&self.crossfade, 0.99) / steady_p99
            );
        }
        let mut slowest = self.loads.clone();
        slowest.sort_by(|a, b| b.1.total_cmp(&a.1));
        println!("  slowest loads:");
        for (rank, (name, ms)) in slowest.iter().take(20).enumerate() {
            println!("   {:>2}. {:>8.1} ms  {}", rank + 1, ms, shorten(name, 72));
        }
        // The list the engine's skip policy would build, most costly first,
        // with the two halves of each switch kept apart.
        let mut slowest = self.costs.clone();
        slowest.sort_by(|a, b| b.1.total_cmp(&a.1));
        println!("  slowest switches:");
        for (rank, (name, ms)) in slowest.iter().take(20).enumerate() {
            let load = self
                .loads
                .iter()
                .find(|(load_name, _)| load_name == name)
                .map(|(_, ms)| *ms)
                .unwrap_or(0.0);
            println!(
                "   {:>2}. switch={:>8.1} ms  load={:>6.1}  first_draw={:>7.1}  {}",
                rank + 1,
                ms,
                load,
                (ms - load).max(0.0),
                shorten(name, 64)
            );
        }
    }
}

fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .chain(['…'])
        .collect()
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let last = values.len() - 1;
    let at = (fraction * last as f64).round() as usize;
    values[at.min(last)]
}

/// A stereo signal with a bass line, a tone and noise bursts: enough for
/// presets that answer the beat to move, without any audio device.
struct Audio {
    samples: u64,
    noise: u32,
}

impl Audio {
    fn new() -> Self {
        Self {
            samples: 0,
            noise: 0x9e37_79b9,
        }
    }

    fn next(&mut self, frames: usize) -> Vec<[f32; 2]> {
        let mut pcm = Vec::with_capacity(frames);
        for _ in 0..frames {
            let t = self.samples as f64 / f64::from(SAMPLE_RATE);
            self.samples += 1;
            let bass = (std::f64::consts::TAU * 55.0 * t).sin();
            let mid = (std::f64::consts::TAU * 220.0 * t).sin();
            let high = (std::f64::consts::TAU * 880.0 * t).sin();
            // A quarter-second burst every two seconds, so the beat-driven
            // parts of a preset have something to follow.
            let burst = (t % 2.0) < 0.25;
            let (noise_left, noise_right) = if burst {
                (self.next_noise(), self.next_noise())
            } else {
                (0.0, 0.0)
            };
            let left = 0.38 * bass + 0.22 * mid + 0.10 * high + 0.20 * noise_left;
            let right = 0.36 * bass + 0.20 * mid + 0.12 * high + 0.18 * noise_right;
            pcm.push([left as f32, right as f32]);
        }
        pcm
    }

    /// Xorshift, so a run is the same signal as the last one.
    fn next_noise(&mut self) -> f64 {
        let mut x = self.noise;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise = x;
        f64::from(x) / f64::from(u32::MAX) * 2.0 - 1.0
    }
}
