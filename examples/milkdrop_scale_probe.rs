//! What a soft cut costs, and what drawing it smaller saves.
//!
//! The fade governor in the MilkDrop window steps the render scale down to two
//! while a crossfade cannot hold the frame rate. This measures the lever it
//! pulls, without a window: for a few presets it starts a real soft cut
//! through libprojectM, renders a short run of frames at the window's size,
//! and then the same fade again at half the width and height. Every frame is
//! timed around `projectm_opengl_render_frame_fbo`, so the numbers are the
//! preset's own work plus the readback, on whatever GPU (or software
//! rasteriser) is present.
//!
//! Run it on Linux, where the context comes from a graphics device with no
//! display server:
//!
//! ```sh
//! sfb gl cargo run --release --example milkdrop_scale_probe -- scratch/presets
//! ```
//!
//! Timings are relative: on llvmpipe they are CPU cost, not a real GPU's.

fn main() {
    #[cfg(all(target_os = "linux", feature = "milkdrop"))]
    linux::main();
    #[cfg(not(all(target_os = "linux", feature = "milkdrop")))]
    println!("this probe draws through Linux's surfaceless EGL; nothing to do here");
}

#[cfg(all(target_os = "linux", feature = "milkdrop"))]
mod linux {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use eframe::glow;
    use spotifast::milkdrop::engine::Engine;
    use spotifast::milkdrop::{CROSSFADE_SECONDS, list_presets};

    /// Frames timed at each size. Kept short enough that both runs of a fade
    /// finish inside projectM's own fade, which is wall-clock timed.
    const FRAMES: usize = 8;
    /// How many presets to walk, a spread across the folder.
    const PRESETS: usize = 12;
    /// What a frame should cost at 60 frames a second.
    const BUDGET: f64 = 1.0 / 60.0;

    pub fn main() {
        let mut args = std::env::args().skip(1);
        let folder = PathBuf::from(args.next().unwrap_or_else(|| "scratch/presets".into()));
        let [width, height] = parse_size(args.next().as_deref()).unwrap_or([640, 480]);
        let presets: Vec<PathBuf> = list_presets(&folder)
            .into_iter()
            .filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.len() > 0))
            .collect();
        if presets.len() < 2 {
            println!("not enough presets in {}", folder.display());
            return;
        }
        let Some((gl, _context)) = context() else {
            println!("no graphics device to draw on");
            return;
        };
        let dirs = [folder.as_path()];
        let Some(mut engine) = Engine::new(Arc::clone(&gl), &dirs, 10) else {
            println!("projectM could not start");
            return;
        };

        // A steady tone, so every preset sees the same audio and the beat
        // detection has something to do.
        let audio: Vec<[f32; 2]> = (0..1024)
            .map(|index| {
                let sample = (index as f32 * 0.03).sin() * 0.25;
                [sample; 2]
            })
            .collect();

        let half = [(width / 2).max(2) & !1, (height / 2).max(2) & !1];
        let mut full_buffer = vec![0u8; width as usize * height as usize * 4];
        let mut half_buffer = vec![0u8; half[0] as usize * half[1] as usize * 4];

        let step = (presets.len() / PRESETS).max(1);
        let mut steady = Vec::new();
        let mut fades_full = Vec::new();
        let mut fades_half = Vec::new();

        for pair in presets.chunks(step).take(PRESETS) {
            let (Some(old), Some(next)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            // The preset the fade comes from, cut in, then a few frames of it
            // on its own.
            cut_in(&mut engine, &audio, old, width, height, &mut full_buffer);
            let quiet = run(&mut engine, &audio, width, height, &mut full_buffer, false);
            steady.extend(quiet);

            // The same fade twice, each started fresh so both fit inside its
            // 2.7 seconds: once at the window's size, once at half.
            cut_in(&mut engine, &audio, old, width, height, &mut full_buffer);
            engine.load(next, true);
            let full = run(&mut engine, &audio, width, height, &mut full_buffer, true);
            cut_in(&mut engine, &audio, old, width, height, &mut full_buffer);
            engine.load(next, true);
            let reduced = run(
                &mut engine,
                &audio,
                half[0],
                half[1],
                &mut half_buffer,
                true,
            );
            println!(
                "{}: fade {:.1} ms full, {:.1} ms half",
                old.file_name().unwrap_or_default().to_string_lossy(),
                median(&full).unwrap_or(0.0) * 1e3,
                median(&reduced).unwrap_or(0.0) * 1e3,
            );
            fades_full.extend(full);
            fades_half.extend(reduced);
        }

        println!();
        println!(
            "window {width}x{height}, half {}x{}, {FRAMES} frames per run, fade {CROSSFADE_SECONDS} s, {} presets",
            half[0],
            half[1],
            fades_full.len() / FRAMES
        );
        summary("steady, full size", &steady);
        summary("fade,   full size", &fades_full);
        summary("fade,   half size", &fades_half);
        let (Some(full), Some(reduced)) = (median(&fades_full), median(&fades_half)) else {
            return;
        };
        println!(
            "fade cost at half size: {:.0}% of full size",
            100.0 * reduced / full
        );
    }

    /// Loads `preset` with a hard cut and draws a few frames of it, so the
    /// fade that follows starts from a running preset rather than a stall.
    fn cut_in(
        engine: &mut Engine,
        audio: &[[f32; 2]],
        preset: &std::path::Path,
        width: u32,
        height: u32,
        pixels: &mut [u8],
    ) {
        engine.load(preset, false);
        for _ in 0..2 {
            engine.feed_frames(audio);
            engine.render_into(width, height, pixels);
        }
    }

    /// Renders `FRAMES` frames and returns how long each took. With
    /// `after_resize`, the first frame is left out: it pays for projectM
    /// making its framebuffers for the new size again.
    fn run(
        engine: &mut Engine,
        audio: &[[f32; 2]],
        width: u32,
        height: u32,
        pixels: &mut [u8],
        after_resize: bool,
    ) -> Vec<Duration> {
        let mut times = Vec::with_capacity(FRAMES);
        for index in 0..FRAMES + usize::from(after_resize) {
            engine.feed_frames(audio);
            let started = Instant::now();
            engine.render_into(width, height, pixels);
            let took = started.elapsed();
            if index == 0 && after_resize {
                continue;
            }
            times.push(took);
        }
        times
    }

    fn summary(name: &str, times: &[Duration]) {
        let Some(median) = median(times) else { return };
        let mut sorted: Vec<f64> = times.iter().map(|time| time.as_secs_f64() * 1e3).collect();
        sorted.sort_by(f64::total_cmp);
        let p90 = sorted[((sorted.len() - 1) as f64 * 0.9).round() as usize];
        let over = times
            .iter()
            .filter(|time| time.as_secs_f64() > BUDGET)
            .count();
        println!(
            "{name}: {:.1} ms median, {:.1} ms p90, {:.1} ms max, {}/{} over 16.7 ms",
            median * 1e3,
            p90,
            sorted[sorted.len() - 1],
            over,
            times.len()
        );
    }

    fn median(times: &[Duration]) -> Option<f64> {
        if times.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = times.iter().map(|time| time.as_secs_f64()).collect();
        sorted.sort_by(f64::total_cmp);
        Some(sorted[sorted.len() / 2])
    }

    fn parse_size(pair: Option<&str>) -> Option<[u32; 2]> {
        let (a, b) = pair?.split_once(['x', ','])?;
        Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
    }

    /// An OpenGL context on a graphics device, with no window or display
    /// server: the same way the pane's child draws when it has no window.
    fn context() -> Option<(
        Arc<glow::Context>,
        glutin::api::egl::context::PossiblyCurrentContext,
    )> {
        use glutin::api::egl::device::Device;
        let mut devices: Vec<Device> = Device::query_devices().ok()?.collect();
        devices.sort_by_key(|device| device.extensions().contains("EGL_MESA_device_software"));
        devices.iter().find_map(|device| match on_device(device) {
            Ok(context) => Some(context),
            Err(error) => {
                eprintln!("a device refused: {error}");
                None
            }
        })
    }

    fn on_device(
        device: &glutin::api::egl::device::Device,
    ) -> Result<
        (
            Arc<glow::Context>,
            glutin::api::egl::context::PossiblyCurrentContext,
        ),
        String,
    > {
        use glutin::config::{Api, ConfigSurfaceTypes, ConfigTemplateBuilder};
        use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
        use glutin::display::GlDisplay;

        // SAFETY: no display handle is passed, which is what makes this a
        // device display and needs no display server.
        let display = unsafe { glutin::api::egl::display::Display::with_device(device, None) }
            .map_err(|error| error.to_string())?;
        let template = ConfigTemplateBuilder::new()
            .with_api(Api::OPENGL)
            .with_surface_type(ConfigSurfaceTypes::empty())
            .build();
        // SAFETY: the template holds no window to outlive.
        let config = unsafe { display.find_configs(template) }
            .map_err(|error| error.to_string())?
            .next()
            .ok_or("no OpenGL configuration")?;
        let attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(None);
        // SAFETY: nothing is shared and no window is named.
        let context = unsafe { display.create_context(&config, &attributes) }
            .map_err(|error| error.to_string())?
            .make_current_surfaceless()
            .map_err(|error| error.to_string())?;
        // SAFETY: the context is current on this thread.
        let gl = Arc::new(unsafe {
            glow::Context::from_loader_function_cstr(|symbol| display.get_proc_address(symbol))
        });
        Ok((gl, context))
    }
}
