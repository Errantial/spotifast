//! Checks that a change to libprojectM leaves what presets compute and draw
//! exactly as it was.
//!
//! It draws a fixed sample of presets with hard cuts, a fixed frame clock
//! (`projectm_set_frame_time`, 60 frames a second) and the same synthetic
//! audio every run, reads every frame back and prints a hash of its pixels.
//! Each preset is handed to projectM ahead of time on another thread, the way
//! the MilkDrop window's prefetch does, and then loaded from the same bytes.
//! Two builds that print the same lines drew the same pictures.
//!
//! ```sh
//! cargo run --release --example milkdrop_evalcheck -- --presets <folder> --count 60 --frames 90 > a.txt
//! # build the other libprojectM, run again into b.txt, then compare the files
//! ```
//!
//! Run it twice on one build first: a preset that draws from a source this
//! does not pin (the hue offsets libprojectM seeds from the OS, a transition
//! chosen at random) differs between runs too, and is not a regression.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::glow::{self, HasContext};
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use spotifast::milkdrop::engine::Engine;
use spotifast::milkdrop::list_presets;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;
const FPS: f64 = 60.0;
const SAMPLE_RATE: u32 = 48_000;

fn main() {
    let mut presets = PathBuf::from("presets");
    let mut count = 60usize;
    let mut frames = 90usize;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--presets" => presets = args.next().map(PathBuf::from).unwrap_or(presets),
            "--count" => count = args.next().and_then(|v| v.parse().ok()).unwrap_or(count),
            "--frames" => frames = args.next().and_then(|v| v.parse().ok()).unwrap_or(frames),
            other => {
                eprintln!("milkdrop_evalcheck: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let all = list_presets(&presets);
    if all.is_empty() {
        eprintln!("milkdrop_evalcheck: no presets in {}", presets.display());
        std::process::exit(1);
    }
    let count = count.min(all.len());
    let sample: Vec<PathBuf> = (0..count)
        .map(|i| all[i * all.len() / count].clone())
        .collect();
    let event_loop = EventLoop::new().expect("an event loop");
    let mut check = Check {
        folder: presets,
        sample,
        frames,
        live: None,
        preset: 0,
        frame: 0,
        clock: 0,
        audio: Audio::new(),
        pixels: vec![0; (WIDTH * HEIGHT * 4) as usize],
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut check).expect("the event loop");
}

struct Live {
    /// Freed first: projectM's shutdown makes GL calls.
    engine: Engine,
    _context: PossiblyCurrentContext,
    _surface: Surface<WindowSurface>,
    _window: Window,
}

struct Check {
    folder: PathBuf,
    sample: Vec<PathBuf>,
    frames: usize,
    live: Option<Live>,
    preset: usize,
    frame: usize,
    clock: u64,
    audio: Audio,
    pixels: Vec<u8>,
}

impl ApplicationHandler for Check {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_none() {
            match build(event_loop, &self.folder) {
                Ok(live) => self.live = Some(live),
                Err(error) => {
                    eprintln!("milkdrop_evalcheck: {error}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = &mut self.live else {
            return;
        };
        if self.preset >= self.sample.len() {
            event_loop.exit();
            return;
        }
        if self.frame == 0 {
            let path = &self.sample[self.preset];
            let data = std::fs::read(path).unwrap_or_default();
            // Prepared on another thread, as the prefetch does, then loaded
            // from the same bytes on this one.
            let ahead = data.clone();
            std::thread::spawn(move || Engine::prepare_data(&ahead))
                .join()
                .expect("the prepare thread");
            if !live.engine.load_data(path, data, false) {
                live.engine.load(path, false);
            }
            println!(
                "preset {} {}",
                self.preset,
                path.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        live.engine.set_frame_time(self.clock as f64 / FPS);
        let pcm = self.audio.next((f64::from(SAMPLE_RATE) / FPS) as usize);
        live.engine.feed_frames(&pcm);
        live.engine.render_into(WIDTH, HEIGHT, &mut self.pixels);
        println!(
            "frame {} {} {:016x}",
            self.preset,
            self.frame,
            fnv(&self.pixels)
        );
        self.clock += 1;
        self.frame += 1;
        if self.frame >= self.frames {
            self.frame = 0;
            self.preset += 1;
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.live = None;
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn build(event_loop: &ActiveEventLoop, folder: &std::path::Path) -> Result<Live, String> {
    let attributes = Window::default_attributes()
        .with_title("MilkDrop check")
        .with_visible(false)
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(WIDTH as f32, HEIGHT as f32));
    let template = ConfigTemplateBuilder::new().with_alpha_size(0);
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(event_loop, template, |mut configs| {
            configs.next().expect("a GL config")
        })
        .map_err(|error| format!("no GL config: {error}"))?;
    let window = window.ok_or_else(|| "the window was not created".to_string())?;
    let raw = window
        .window_handle()
        .map_err(|error| format!("no window handle: {error}"))?
        .as_raw();
    let context_attributes = ContextAttributesBuilder::new().build(Some(raw));
    // SAFETY: the display outlives the context.
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
    let _ = surface.set_swap_interval(&context, SwapInterval::DontWait);
    // SAFETY: the context is current on this thread.
    let gl = Arc::new(unsafe {
        glow::Context::from_loader_function_cstr(|symbol| config.display().get_proc_address(symbol))
    });
    // SAFETY: the context is current.
    unsafe {
        println!(
            "renderer {} {}",
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VERSION)
        );
    }
    let textures = folder.join("textures");
    let dirs = [folder, textures.as_path()];
    let engine = Engine::new(Arc::clone(&gl), &dirs, 60)
        .ok_or_else(|| "projectM needs OpenGL 3.3".to_string())?;
    Ok(Live {
        engine,
        _context: context,
        _surface: surface,
        _window: window,
    })
}

/// The stutter harness's signal: a bass line, a tone and noise bursts.
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

    fn next_noise(&mut self) -> f64 {
        let mut x = self.noise;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise = x;
        f64::from(x) / f64::from(u32::MAX) * 2.0 - 1.0
    }
}
