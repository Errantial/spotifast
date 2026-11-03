//! Throwaway feasibility probe for MilkDrop frame cost, preset load cost and
//! window resize cost, plus the shared-memory frame path's CPU cost.
//!
//! Run under software GL (llvmpipe) on a machine without a display:
//!
//!   cargo run --release --example milkdrop_probe -- --presets <dir> [options]
//!
//! Timings from llvmpipe are relative only: they say how presets compare and
//! where the CPU cost sits, not what a GPU will do.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::glow::{self, HasContext};
use projectm_sys as pm;

fn main() {
    let args = Args::parse();
    let Some(context) = device_context() else {
        eprintln!("no graphics device: nothing to measure");
        std::process::exit(3);
    };
    println!("# renderer: {}", renderer_name(&context.gl));
    let presets = spotifast::milkdrop::list_presets(&args.presets);
    if presets.is_empty() {
        eprintln!("no presets in {}", args.presets.display());
        std::process::exit(2);
    }
    println!("# presets in folder: {}", presets.len());
    let sample = sample(&presets, args.limit);
    println!("# sampled: {}", sample.len());

    shm_bench();

    let mut engine = Probe::new(&context.gl, args.seconds);
    engine.render_target([640, 480]);
    match args.mode.as_str() {
        "cost" => cost(&mut engine, &sample, &args),
        "load" => load(&mut engine, &sample, &args),
        "transition" => transition(&mut engine, &sample, &args),
        "resize" => resize(&mut engine, &sample, &args),
        _ => {
            load(&mut engine, &sample, &args);
            cost(&mut engine, &sample, &args);
            transition(&mut engine, &sample, &args);
            resize(&mut engine, &sample, &args);
        }
    }
}

struct Args {
    presets: PathBuf,
    limit: usize,
    frames: usize,
    warmup: usize,
    seconds: u32,
    mode: String,
    sizes: Vec<[u32; 2]>,
}

impl Args {
    fn parse() -> Self {
        let mut args = Self {
            presets: PathBuf::new(),
            limit: 60,
            frames: 30,
            warmup: 10,
            seconds: 10,
            mode: "all".into(),
            sizes: vec![[640, 480], [1280, 720], [1920, 1080]],
        };
        let mut all = std::env::args().skip(1);
        while let Some(arg) = all.next() {
            match arg.as_str() {
                "--presets" => args.presets = PathBuf::from(all.next().unwrap_or_default()),
                "--limit" => {
                    args.limit = all.next().and_then(|v| v.parse().ok()).unwrap_or(60).max(1);
                }
                "--frames" => args.frames = all.next().and_then(|v| v.parse().ok()).unwrap_or(30),
                "--warmup" => args.warmup = all.next().and_then(|v| v.parse().ok()).unwrap_or(10),
                "--seconds" => args.seconds = all.next().and_then(|v| v.parse().ok()).unwrap_or(10),
                "--mode" => args.mode = all.next().unwrap_or_else(|| "all".into()),
                "--sizes" => {
                    args.sizes = all
                        .next()
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|pair| {
                            let (w, h) = pair.split_once(['x', ':'])?;
                            Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
                        })
                        .collect();
                }
                other => eprintln!("ignoring {other}"),
            }
        }
        if args.presets.is_relative() && !args.presets.as_os_str().is_empty() {
            // cargo runs the example from wherever it was invoked; the
            // presets travel with the checkout.
            args.presets = Path::new(env!("CARGO_MANIFEST_DIR")).join(&args.presets);
        }
        if args.sizes.is_empty() {
            args.sizes = vec![[640, 480], [1280, 720], [1920, 1080]];
        }
        if args.presets.as_os_str().is_empty() {
            eprintln!(
                "usage: milkdrop_probe --presets <dir> [--limit N] [--frames N] [--mode cost|load|transition|resize|all]"
            );
            std::process::exit(2);
        }
        args
    }
}

/// Every `step`th preset, so the sample spreads across the folder instead of
/// taking one style.
fn sample(files: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    if files.len() <= limit {
        return files.to_vec();
    }
    let step = files.len() / limit;
    files
        .iter()
        .step_by(step.max(1))
        .take(limit)
        .cloned()
        .collect()
}

struct Probe {
    handle: pm::projectm_handle,
    gl: Arc<glow::Context>,
    framebuffer: glow::Framebuffer,
    texture: glow::Texture,
    size: [u32; 2],
    audio: Vec<f32>,
    audio_at: usize,
}

impl Probe {
    fn new(gl: &Arc<glow::Context>, seconds: u32) -> Self {
        let gl = Arc::clone(gl);
        // SAFETY: the context is current.
        let handle = unsafe { pm::projectm_create() };
        assert!(!handle.is_null(), "projectM could not start");
        unsafe {
            pm::projectm_set_preset_duration(handle, f64::from(seconds));
            pm::projectm_set_easter_egg(handle, 0.0);
            pm::projectm_set_soft_cut_duration(handle, 2.7);
            pm::projectm_set_hard_cut_enabled(handle, false);
            pm::projectm_set_aspect_correction(handle, true);
            pm::projectm_set_mesh_size(handle, 48, 36);
            pm::projectm_set_fps(handle, 60);
        }
        let (framebuffer, texture) = unsafe {
            let texture = gl.create_texture().expect("a texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                2,
                2,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            let framebuffer = gl.create_framebuffer().expect("a framebuffer");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            (framebuffer, texture)
        };
        // 60 frames a second of 44.1 kHz stereo, the way the child feeds it.
        let frames = 44_100 / 60;
        let mut audio = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let sample = (index as f32 * 220.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.3;
            audio.push(sample);
            audio.push(sample);
        }
        let mut probe = Self {
            handle,
            gl,
            framebuffer,
            texture,
            size: [2, 2],
            audio,
            audio_at: 0,
        };
        probe.render_target([640, 480]);
        probe
    }

    /// Reallocates the render target, the way `Engine::resize` does.
    fn render_target(&mut self, [width, height]: [u32; 2]) {
        let (width, height) = (width.max(2) & !1, height.max(2) & !1);
        self.size = [width, height];
        let gl = &self.gl;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.viewport(0, 0, width as i32, height as i32);
        }
        unsafe { pm::projectm_set_window_size(self.handle, width as usize, height as usize) };
    }

    fn feed(&mut self) {
        if self.audio.is_empty() {
            return;
        }
        let count = self.audio.len() / 2;
        let start = self.audio_at % count;
        self.audio_at = (self.audio_at + 37) % count;
        let mut frame = Vec::with_capacity(self.audio.len());
        for index in 0..count {
            let at = ((start + index) % count) * 2;
            frame.push(self.audio[at]);
            frame.push(self.audio[at + 1]);
        }
        // SAFETY: the context is current and `frame` is stereo pairs.
        unsafe {
            pm::projectm_pcm_add_float(
                self.handle,
                frame.as_ptr(),
                count as u32,
                pm::projectm_channels_PROJECTM_STEREO,
            );
        }
    }

    /// One projectM frame into the render target; no readback.
    fn render(&mut self) {
        let gl = &self.gl;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.viewport(0, 0, self.size[0] as i32, self.size[1] as i32);
            pm::projectm_opengl_render_frame_fbo(self.handle, self.framebuffer.0.get());
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }

    /// One frame and a full readback, the way the pane path publishes.
    fn render_readback(&mut self, pixels: &mut [u8]) {
        self.render();
        let gl = &self.gl;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                self.size[0] as i32,
                self.size[1] as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(pixels)),
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }

    fn load(&mut self, path: &Path, smooth: bool) -> (f64, f64) {
        let Ok(path) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) else {
            return (0.0, 0.0);
        };
        let started = Instant::now();
        let cpu = cpu_ms();
        unsafe { pm::projectm_load_preset_file(self.handle, path.as_ptr(), smooth) };
        let wall = started.elapsed().as_secs_f64() * 1000.0;
        (wall, cpu_ms() - cpu)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        unsafe {
            pm::projectm_destroy(self.handle);
            self.gl.delete_framebuffer(self.framebuffer);
            self.gl.delete_texture(self.texture);
        }
    }
}

fn cost(probe: &mut Probe, presets: &[PathBuf], args: &Args) {
    for size in &args.sizes {
        probe.render_target(*size);
        let mut all = Vec::new();
        let mut per_preset: Vec<(String, f64, f64)> = Vec::new();
        for path in presets {
            let _ = probe.load(path, false);
            for _ in 0..args.warmup {
                probe.feed();
                probe.render();
            }
            let mut times = Vec::with_capacity(args.frames);
            for _ in 0..args.frames {
                probe.feed();
                let started = cpu_ms();
                probe.render();
                times.push(cpu_ms() - started);
            }
            times.sort_by(f64::total_cmp);
            let name = preset_name(path);
            per_preset.push((name, median(&times), p95(&times)));
            all.extend_from_slice(&times);
        }
        all.sort_by(f64::total_cmp);
        println!(
            "cost {}x{} frames={} median={:.3} ms p95={:.3} ms max={:.3} ms implied_fps={:.1}",
            size[0],
            size[1],
            args.frames,
            median(&all),
            p95(&all),
            all.last().copied().unwrap_or(0.0),
            1000.0 / median(&all).max(0.001),
        );
        per_preset.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (name, median, p95) in per_preset.iter().take(12) {
            println!("  heavy {median:8.3} ms (p95 {p95:8.3}) {name}");
        }
        if let Some((name, median, _)) = per_preset.last() {
            println!("  lightest {median:8.3} ms {name}");
        }
        // What a full-frame readback adds, which is what the pane path pays
        // on top of the render: the child reads every frame back to the CPU.
        let mut pixels = vec![0u8; (size[0] * size[1] * 4) as usize];
        let _ = probe.load(&presets[0], false);
        for _ in 0..args.warmup {
            probe.feed();
            probe.render();
        }
        let mut readback = Vec::new();
        for _ in 0..args.frames {
            probe.feed();
            let started = cpu_ms();
            probe.render_readback(&mut pixels);
            readback.push(cpu_ms() - started);
        }
        readback.sort_by(f64::total_cmp);
        println!(
            "  readback {}x{} render+read median={:.3} ms",
            size[0],
            size[1],
            median(&readback)
        );
    }
}

fn load(probe: &mut Probe, presets: &[PathBuf], args: &Args) {
    let size = args.sizes[0];
    probe.render_target(size);
    let mut loads = Vec::new();
    let mut loads_cpu = Vec::new();
    let mut firsts = Vec::new();
    let mut per_preset: Vec<(String, f64, f64)> = Vec::new();
    for path in presets {
        let (load, load_cpu) = probe.load(path, false);
        probe.feed();
        let started = cpu_ms();
        probe.render();
        let first = cpu_ms() - started;
        for _ in 0..args.warmup {
            probe.feed();
            probe.render();
        }
        loads.push(load);
        loads_cpu.push(load_cpu);
        firsts.push(first);
        per_preset.push((preset_name(path), load_cpu, first));
    }
    loads.sort_by(f64::total_cmp);
    loads_cpu.sort_by(f64::total_cmp);
    firsts.sort_by(f64::total_cmp);
    println!(
        "load {}x{} wall median={:.2} ms p95={:.2} ms max={:.2} ms (includes disk)",
        size[0],
        size[1],
        median(&loads),
        p95(&loads),
        loads.last().copied().unwrap_or(0.0)
    );
    println!(
        "load {}x{} cpu median={:.2} ms p95={:.2} ms max={:.2} ms",
        size[0],
        size[1],
        median(&loads_cpu),
        p95(&loads_cpu),
        loads_cpu.last().copied().unwrap_or(0.0)
    );
    println!(
        "first-frame {}x{} cpu median={:.2} ms p95={:.2} ms max={:.2} ms",
        size[0],
        size[1],
        median(&firsts),
        p95(&firsts),
        firsts.last().copied().unwrap_or(0.0)
    );
    per_preset.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (name, load, first) in per_preset.iter().take(15) {
        println!("  slow-load {load:8.2} ms (first frame {first:8.2}) {name}");
    }
}

fn transition(probe: &mut Probe, presets: &[PathBuf], args: &Args) {
    let size = args.sizes[0];
    probe.render_target(size);
    let mut steady_all = Vec::new();
    let mut fade_all = Vec::new();
    for pair in presets.chunks(2).filter(|pair| pair.len() == 2) {
        let _ = probe.load(&pair[0], false);
        for _ in 0..args.warmup {
            probe.feed();
            probe.render();
        }
        let mut steady = Vec::with_capacity(args.frames);
        for _ in 0..args.frames {
            probe.feed();
            let started = cpu_ms();
            probe.render();
            steady.push(cpu_ms() - started);
        }
        let _ = probe.load(&pair[1], true);
        let fade_started = Instant::now();
        let mut fade = Vec::new();
        while fade_started.elapsed() < Duration::from_secs_f64(2.7) {
            probe.feed();
            let started = cpu_ms();
            probe.render();
            fade.push(cpu_ms() - started);
        }
        steady.sort_by(f64::total_cmp);
        fade.sort_by(f64::total_cmp);
        println!(
            "transition {:40} steady={:7.3} ms fade={:7.3} ms ratio={:.2}",
            format!("{} -> {}", preset_name(&pair[0]), preset_name(&pair[1])),
            median(&steady),
            median(&fade),
            median(&fade) / median(&steady).max(0.001)
        );
        steady_all.extend(steady);
        fade_all.extend(fade);
    }
    steady_all.sort_by(f64::total_cmp);
    fade_all.sort_by(f64::total_cmp);
    println!(
        "transition all {}x{} steady median={:.3} p95={:.3}; fade median={:.3} p95={:.3}; ratio={:.2}",
        size[0],
        size[1],
        median(&steady_all),
        p95(&steady_all),
        median(&fade_all),
        p95(&fade_all),
        median(&fade_all) / median(&steady_all).max(0.001)
    );
}

fn resize(probe: &mut Probe, presets: &[PathBuf], args: &Args) {
    // A heavy preset, so the reallocation cost is not lost in the noise.
    let path = presets
        .iter()
        .max_by_key(|path| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .cloned()
        .unwrap_or_else(|| presets[0].clone());
    println!("# resize preset: {}", preset_name(&path));
    probe.render_target([640, 480]);
    let _ = probe.load(&path, false);
    for _ in 0..args.warmup {
        probe.feed();
        probe.render();
    }
    // A drag: forty steps of 12 px across and 9 down, one frame each.
    let mut steps = Vec::new();
    let mut setups = Vec::new();
    for step in 1..=40u32 {
        probe.feed();
        let started = cpu_ms();
        probe.render_target([640 + step * 12, 480 + step * 9]);
        setups.push(cpu_ms() - started);
        let started = cpu_ms();
        probe.render();
        steps.push(cpu_ms() - started);
    }
    steps.sort_by(f64::total_cmp);
    setups.sort_by(f64::total_cmp);
    println!(
        "resize step reallocate median={:.2} ms p95={:.2} ms max={:.2} ms",
        median(&setups),
        p95(&setups),
        setups.last().copied().unwrap_or(0.0)
    );
    println!(
        "resize step first frame median={:.2} ms p95={:.2} ms max={:.2} ms",
        median(&steps),
        p95(&steps),
        steps.last().copied().unwrap_or(0.0)
    );
    // One jump, as a double-click to full screen does.
    probe.render_target([640, 480]);
    probe.render();
    probe.feed();
    let started = cpu_ms();
    probe.render_target([1920, 1080]);
    probe.render();
    println!(
        "resize jump 640x480 -> 1920x1080: {:.2} ms cpu",
        cpu_ms() - started
    );
    // Steady-state cost at the new size, to see the pixels' own share.
    let mut steady = Vec::new();
    for _ in 0..args.frames {
        probe.feed();
        let started = cpu_ms();
        probe.render();
        steady.push(cpu_ms() - started);
    }
    steady.sort_by(f64::total_cmp);
    println!(
        "resize steady 1920x1080 median={:.2} ms cpu",
        median(&steady)
    );
}

/// The pane path's CPU cost: publish into shared memory and copy out again,
/// with no GPU work in it.
fn shm_bench() {
    use spotifast::milkdrop::frames::Frames;
    let path = std::env::temp_dir().join(format!("spotifast-probe-frames-{}", std::process::id()));
    let app = Frames::create(&path).expect("a frame buffer");
    let child = Frames::open(&path).expect("the same frame buffer");
    for size in [[640, 480], [1280, 720], [1920, 1080]] {
        let mut seen = 0;
        let mut publish = Vec::new();
        let mut take = Vec::new();
        for index in 0..40u8 {
            let started = Instant::now();
            child.publish(size, |pixels| pixels.fill(index));
            publish.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = Instant::now();
            let frame = app.take(&mut seen);
            take.push(started.elapsed().as_secs_f64() * 1000.0);
            assert!(frame.is_some(), "a frame was published");
        }
        publish.sort_by(f64::total_cmp);
        take.sort_by(f64::total_cmp);
        println!(
            "shm {}x{} publish median={:.3} ms take median={:.3} ms together={:.3} ms implied_fps={:.1}",
            size[0],
            size[1],
            median(&publish),
            median(&take),
            median(&publish) + median(&take),
            1000.0 / (median(&publish) + median(&take)).max(0.001)
        );
    }
    let _ = std::fs::remove_file(&path);
}

fn preset_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// This process's CPU time in milliseconds, from `/proc/self/stat`: llvmpipe
/// rasterizes on threads of this process, so its cost shows here even when
/// the build host is busy with someone else's compile.
fn cpu_ms() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return 0.0;
    };
    let rest = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let ticks = |index: usize| -> u64 {
        fields
            .get(index)
            .and_then(|field| field.parse().ok())
            .unwrap_or(0)
    };
    // After comm: state is 0, so utime (field 14) is 11 and stime (15) is 12.
    (ticks(11) + ticks(12)) as f64 * 10.0
}

fn median(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[sorted.len() / 2]
}

fn p95(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)]
}

fn renderer_name(gl: &glow::Context) -> String {
    unsafe {
        format!(
            "{} / {}",
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VERSION)
        )
    }
}

/// The OpenGL context and what keeps it alive; the same approach as the
/// pane's windowless child on Linux.
struct DeviceContext {
    gl: Arc<glow::Context>,
    _context: glutin::api::egl::context::PossiblyCurrentContext,
}

fn device_context() -> Option<DeviceContext> {
    use glutin::api::egl::device::Device;
    let mut devices: Vec<Device> = Device::query_devices().ok()?.collect();
    devices.sort_by_key(|device| device.extensions().contains("EGL_MESA_device_software"));
    devices.iter().find_map(|device| {
        on_device(device)
            .map_err(|error| eprintln!("# a device refused: {error}"))
            .ok()
    })
}

fn on_device(device: &glutin::api::egl::device::Device) -> Result<DeviceContext, String> {
    use glutin::config::{Api, ConfigSurfaceTypes, ConfigTemplateBuilder};
    use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
    use glutin::display::GlDisplay;

    // SAFETY: no display handle is passed, which is what makes this a device
    // display and needs no display server.
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
    Ok(DeviceContext {
        gl,
        _context: context,
    })
}
