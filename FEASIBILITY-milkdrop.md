# MilkDrop feasibility: 120 fps, custom resizing, and per-scene control

Feasibility study for the fork `Errantial/spotifast`, branch
`feasibility-milkdrop`, off `milkdrop-smooth-transitions`. Nothing here is
implemented; this file records what the code does today, what a change would
touch, and whether it is worth doing. Numbers came from the build host's
software OpenGL (llvmpipe under xvfb) and are **relative only**.

Method: read `src/milkdrop*`, `src/ui/**`, `src/settings.rs`, `src/app.rs`,
`src/power.rs`, the projectM 4 C API headers, and winit 0.30's platform
backends; measured with `examples/milkdrop_probe.rs` (kept on this branch) on
120 presets sampled from Cream of the Crop. See the appendix for the commands.

Terms: a **preset** is a `.milk` file (the issue calls these "scenes").
**Standalone window** is the separate OS window the child process opens.
**Pane** is MilkDrop drawn inside the app's own windows (classic Winamp skin,
Windows Media Player skin, or full-screen lyrics backdrop).

## What the pipeline is today

Two different paths share one engine:

| | Standalone window (`child.rs`) | Pane (`offscreen.rs` + `pane.rs`) |
| --- | --- | --- |
| Event loop | winit window, one per child process | winit hidden window on macOS/Windows; EGL device on Linux |
| Presentation | `surface.swap_buffers`, `SwapInterval::DontWait` (`child.rs:1062`) | none; `glReadPixels` into shared memory (`engine.rs:291`) |
| Frame reaches the app | never; the child draws its own window | shared-memory slots (`frames.rs`), copied by the UI thread and uploaded as an egui texture (`pane.rs:166-180`) |
| Frame rate | `--milkdrop-fps` / settings, 0 = uncapped (`child.rs:291-293`) | `milkdrop_pane_fps()` and a hard `min(DEFAULT_FPS)` = 60 (`app.rs:4972-4974`) |
| Pacing | software timer, `ControlFlow::WaitUntil` (`child.rs:275-293`, `536-563`) | `due += 1/fps`, `thread::sleep` (`offscreen.rs:123-125`) |

Both call `prepare_frame()` (`child.rs:961-980`) every frame: refresh the
preset listing, advance the preset timer, take a pending load, and feed the
PCM that arrived since the last frame through the shared ring.

---

## Q1. Can MilkDrop run at 120 fps, and be stable at it?

### 1.1 120 fps is already selectable, with two hard caps behind it

The frame-rate dial is not a missing feature. `milkdrop_fps` exists
(`settings.rs:431`), its stops include the screen's own rate and Uncapped
(`milkdrop.rs:56-67`), and the range runs to 360 (`milkdrop.rs:48`). On a
120 Hz screen the "your screen" stop is 120. A rate can also be typed
(`ui/settings.rs:295-315`), landing on the nearest stop.

What is capped today:

1. **Every in-app pane is capped at 60 fps**, whatever the setting says:
   `app.rs:4961-4975` computes the rate then returns
   `rate.min(crate::milkdrop::DEFAULT_FPS)`. That covers the Winamp-framed
   window, the classic skin's panel, the WMP skin pane, and the lyrics
   backdrop.
2. **The pane's visible rate is also the app's own frame rate.**
   `theme::visualiser_frame` requests a repaint after `1/rate`
   (`theme.rs:879-905`), and eframe's main window renders with vsync on
   (`entrypoint.rs:1122-1145`). On macOS that vsync path is exactly what
   upstream #711 reports as stuck near 60 Hz on a 120 Hz ProMotion display
   (open PR #966 proposes turning vsync off there). A 120 fps pane cannot
   look faster than the window it is painted into.
3. **The standalone window is not capped**: it takes the setting as-is
   (`host.rs:133-138`, `child.rs:108`), including 0 = uncapped, which is a
   busy loop (`ControlFlow::Poll`, `child.rs:548-552`).

So the question is not "add 120 fps" but "is the standalone window's 120 fps
stable, and can the pane ever join it".

### 1.2 projectM's own frame cost and what `projectm_set_fps` does

`engine.rs:106` calls `projectm_set_fps(handle, 60)` once, at startup, and
never again. The projectM 4 header is explicit that this value is **only a
preset variable**: "The value is passed on to presets, which may choose to use
it for calculations. It is not used in any other way by the library"
(`parameters.h`, `projectm_set_fps`). `ProjectM::GetRenderContext()` copies it
into `renderContext.fps`; nothing else reads it.

That matters because presets do use it to normalise motion. In the sampled
120 Cream-of-the-Crop presets, **35 (29%) use `fps` in their per-frame,
per-pixel or per-vertex equations**, e.g.
`per_frame_10=mtime=mtime+vol*0.01*(48/fps)`; three more mention it only in
comments. With the value stuck at 60, a preset played at 30 fps (the
energy-saver cap, `power.rs:44-52`) or at 120 fps animates at the wrong
speed. This is a real bug today at 30 fps, not only at 120.

The cost side is entirely unmeasured in production: there is no frame-time
telemetry in the child. WP1 in `PLAN-milkdrop-transitions.md` adds it and is
the prerequisite for every number below on real hardware.

### 1.3 The child render loop, per frame

`child.rs:522-525` → `render()` (`355-390`):

1. `prepare_frame`: listing check (cheap while fresh, `milkdrop.rs:192-195`),
   `Presets::advance` (Instant compare), `engine.set_seconds` (no-op when
   unchanged, `engine.rs:134-140`), `ring.since` which **allocates a `Vec`
   every frame** (`shm.rs:104-120`), `projectm_pcm_add_float`.
2. `engine.render` (`engine.rs:174-258`): round the window's physical pixels
   down to even, divide by the user's `scale` (1, 2 or 4), reallocate the copy
   FBO and texture if the size changed (`engine.rs:186-193`, `303-356`),
   render projectM at the inner size, then **two full-frame blits**: inner →
   copy FBO, copy FBO → window rect scaled (`engine.rs:219-246`).
3. Up to four overlay draws (the keys card, the song card, three corner
   statuses) — one textured quad each once their bitmap exists
   (`overlay.rs:517+`; corners rewrite at most every 400 ms,
   `child.rs:805-866`).
4. `swap_buffers` with swap interval 0 (`child.rs:385-387`, `1062`).
5. `update_corners` and `report_geometry`; the latter queries the window's
   position and size every frame and reports only on change
   (`child.rs:392-419`).

### 1.4 Swap interval, pacing, and presentation per platform

`build()` sets `SwapInterval::DontWait` unconditionally (`child.rs:1062`).
Pacing is therefore purely the software timer in `schedule_next_frame_at`
(`child.rs:275-293`): add the interval to the previous deadline; if that
deadline has already passed, restart the rhythm from now. `about_to_wait`
sleeps with `ControlFlow::WaitUntil` or polls when uncapped
(`child.rs:536-563`).

Consequences at 8.33 ms frames:

- **No phase lock to the compositor.** A frame that lands a millisecond after
  a vblank waits a whole refresh: at 120 Hz that is 8.33 ms of extra latency
  and a visible hitch, while at 60 Hz the same miss costs 16.7 ms but happens
  half as often relative to the beat. Timer wake-up error (~1 ms typical, OS
  dependent) is 12% of a 120 Hz frame and 6% of a 60 Hz one. This is the main
  stability risk on Windows and Wayland, independent of GPU cost.
- **Windows**: a WGL swap with interval 0 goes through DWM, which still
  presents on the monitor's vblank. Frames that miss the beat are dropped or
  wait, so in the worst case the cadence becomes 120 with occasional
  60 Hz pairs unless presentation is vsync-aligned.
- **macOS**: the child's context is a plain NSOpenGL context with interval 0,
  so it is not the eframe path that #711/#966 discuss. Whether AppKit gives an
  undecorated 120 Hz window 120 presents per second is unverified here; it
  needs a ProMotion machine.
- **Wayland**: the child is a normal `xdg_toplevel` GL surface. With interval
  0 the commit does not wait for the compositor's frame callback; the
  compositor drops what it cannot show. AGENTS.md's note about the egui fork
  pacing by frame callbacks is about eframe's window, not this child (the
  child uses glutin directly), but the rule "Do not add a vsync decision of
  our own" still applies to the app's window.
- **Uncapped (0)** is a `Poll` loop: one core at 100% plus the GPU. It is not
  a 120 fps mode; 120 should be selected explicitly.

### 1.5 Measurements (llvmpipe, relative only)

Host: `llvmpipe (LLVM 21.1.8, 256 bits)`, Mesa 26.0.8, xvfb, 24 shared
cores. Sample: 30 of the 120 sampled Cream-of-the-Crop presets, 10 timed
frames each after 10 warm-up frames. Frame times are **process CPU time**
(10 ms resolution), so other builds on the host do not inflate them.

| Measurement | Result |
| --- | --- |
| projectM frame, no readback, median over presets: 640x480 / 1280x720 / 1920x1080 | **120 / 240 / 480 ms** |
| Same, p95 | 280 / 660 / 1380 ms |
| Heaviest preset at 1080p vs lightest | 2290 ms (Acid Mandala v5a) vs 290 ms (EoS pointfield) = **7.9x spread** |
| Soft-cut crossfade vs steady, median / p95 / worst pair | **2.45x / 2.26x / 6.0x** |
| Preset load (parse + shader compile), wall | **37.8 ms median, 75.1 ms p95, 97.5 ms max** |
| First render after a load, at 640x480 | **350 ms median, 1090 ms p95** (about 3x a steady 480p frame) |
| Full-frame readback added to a render (what the pane path does) | about +10 ms at 480p, +30 ms at 720p, +40 ms at 1080p |
| Shared-memory publish + take (CPU only, no GPU) | **0.07 / 0.28 / 1.20 ms** at 480p / 720p / 1080p |
| Resize: our reallocation (`projectm_set_window_size` + FBO + texture) | 0 ms median, 10 ms p95 |
| Resize: the next frame | 290 ms median, 660 ms p95, 1200 ms max |
| Resize jump 640x480 → 1920x1080 | 1200 ms for that frame; steady at 1080p for that preset 980 ms |

What transfers and what does not:

- **Transfers**: cost tracks pixels almost linearly (120 → 240 → 480 ms for
  480p → 720p → 1080p, i.e. 1x → 2x → 4x time for 1x → 2.25x → 5.06x
  pixels). The `scale` lever (half, quarter) is therefore the right knob,
  and a preset that is heavy at 60 is heavy at 120.
- **Transfers**: the spread between presets (~8x) and the crossfade
  multiplier (~2.5x) are shape facts that a GPU will not change much.
- **Does not transfer**: anything absolute. llvmpipe is CPU rasterization
  and these frames are 50-200x a mid-range GPU. Do not read "480 ms at
  1080p" as a GPU cost.
- The shared-memory bookkeeping is cheap in CPU terms (1.2 ms for a 1080p
  frame at 120 Hz is 14% of an 8.33 ms budget), but it excludes the
  GPU→CPU readback, which is a pipeline stall on real hardware and is the
  reason the pane path is expensive.

### 1.6 Frame-time budget

| Budget | 60 fps | 120 fps |
| --- | --- | --- |
| Frame | 16.67 ms | 8.33 ms |
| projectM render + 2 blits (GPU, unmeasured here) | ? | ? |
| Full-frame readback + shm copy (pane only) | 1.20 ms at 1080p + readback stall | same |
| Soft cut for 2.7 s (two presets + blend) | **2.45x** measured | 2.45x |
| Preset load (parse + compile) | **37.8 ms** median, 97.5 ms max | same, i.e. **4.5 to 11.7 dropped frames** |
| First render after a load | 3x a steady frame (llvmpipe) | same, i.e. 3 dropped frames |

**Reading the budget.** The only measured terms that do not shrink on a
faster GPU are the load hitch and the fixed CPU work: a 37.8 ms load is 4.5
frames at 120 no matter how fast the card is, and the first render after it
is another 3. That is the case for doing WP2/WP3 before claiming 120. The
render term is unknown on hardware; the crossfade multiplies whatever it is
by ~2.5, so a steady state above ~3.3 ms cannot fade at 120 without help.
On the llvmpipe arithmetic above, 720p or Half resolution is the only
setting where the median heavy preset would plausibly fit an 8.33 ms frame,
and 1080p Full would need the fade to drop quality. That is a hypothesis to
test with WP1 and a real GPU, not a conclusion.

### 1.7 Heaviest presets as the real risk

The same names are heaviest at every size, so a budget or blocklist built
from one machine's numbers will catch the same presets elsewhere. In this
sample (llvmpipe, 1080p, ms):

| Preset | 480p | 720p | 1080p |
| --- | --- | --- | --- |
| Fumbling_Foo / Flexi / Martin / ORB - Acid Mandala v5a | 390 | 980 | 2290 |
| LuxXx - SquidgyFace I | 230 | 590 | 1370 |
| cpe domains flacc / phlegmly sczhultz hog rot2 | 180 | 450 | 1230 |
| EVET / flexi - undercurrents 3 | 190 | 660 | 1040 |
| suksma - selectron validumb (put the screws to you) | 280 | 510 | 960 |
| Martin - Pixies Party (Hakan mash-up 6-7) | 190 | 470 | 960 |
| Median of the 30 | 120 | 240 | 480 |

A 120 Hz budget is 8.33 ms. If a GPU is 100x faster on fill at 1080p, the
median lands near 4.8 ms and these presets near 10-23 ms; at 720p the median
is ~2.4 ms and the worst ~10 ms. That is arithmetic on an unmeasured speedup,
not a measurement, but it says where the risk is: **1080p + heavy preset +
crossfade is over a 120 Hz budget on any plausible mid-range GPU, while 720p
or Full/Half resolution is not.** This is why the existing Resolution setting
is the right first answer, and why adaptive quality during a fade (WP4b) is
the right second one.

### 1.8 Interaction with the transition work

- **Load hitch.** `prepare_frame` loads synchronously on the render thread
  (`child.rs:975-977` → `engine.rs:145-151` → `projectm_load_preset_file`).
  Disk read, `.milk` parse, HLSL→GLSL translation and shader compile all
  happen there. At 120 fps the measured 37.8 ms median load is 4.5 missed
  frames (11.7 at the 97.5 ms maximum), and the audio ring then delivers a
  burst (`shm.rs:104-120`, bounded by the 0.5 s ring).
  WP2 and WP3 address this and are prerequisites for any smooth 120.
- **Crossfade doubling.** projectM renders both presets during a soft cut
  (`ProjectM::RenderFrame`, upstream source), for 2.7 s. Measured **2.45x**
  the steady cost at the median and up to **6.0x** on a pair. If steady
  state is already near 8.33 ms, the fade cannot hold 120 without adaptive
  quality (WP4b) or a shorter fade.
- **Adaptive quality.** The `scale` lever reallocates the copy FBO and
  projectM's own textures through `Engine::resize` on every change, so
  switching it per frame would be worse than the problem. It needs hysteresis
  and a stable step (WP4b), and the resize measurement in Q2 bounds how often
  it may switch.

### 1.9 Should 120 be opt-in or automatic? (no new settings)

AGENTS.md and CONTRIBUTING.md forbid new user settings without the
maintainer's approval in an issue, and CONTRIBUTING.md:166-174 says the answer
is "usually no: prefer a good default that serves everyone, or a keyboard
shortcut or command". This feature needs no new setting:

- The dial already offers the screen's rate; on a 120 Hz screen that is 120.
  Nothing to add, and nothing to document as new.
- "Automatic" already happens: the first time the window reports a real
  screen rate, `milkdrop_fps` is set to it if the user never chose one
  (`app.rs:5001-5012`), so a 120 Hz screen already defaults to 120.
- The one thing that *should* change is not a setting: report the real frame
  rate to projectM (`projectm_set_fps`) so presets animate correctly at 30,
  120 or the screen's rate.

Upstream has already made a pacing decision in this area: PR #732, "Pace the
visualisers and MilkDrop by what can be seen" (merged 2026-10-12), is where
the 30 fps energy-saver cap and the visibility budgets live (`power.rs`). A
120 mode must coexist with that policy rather than bypass it, which is one
more reason not to add a separate switch.

So: **no opt-in switch**. The honest position is that 120 is the screen-rate
option, and it becomes stable or it does not; a "120 fps" toggle that stutters
is worse than the screen-rate stop that does not promise anything.

### 1.10 Verdict

**Go, but not as a feature.** 120 fps already exists as the screen-rate
default on a 120 Hz display, so the honest work is to make that rate
trustworthy rather than to add a mode:

- **Go (0.5 d):** pass the real rate to `projectm_set_fps`, so the 29% of
  presets that read `fps` animate at the right speed at 30, 120 or the
  screen's rate. This is a bug at 30 fps today.
- **Go (already planned):** WP1 telemetry, WP2/WP3 load off the render
  thread, WP4a time-based fades, WP4b adaptive quality. Without them the
  measured 4.5-11.7 dropped frames per preset switch at 120 are unavoidable.
- **Go (1-2 d + hardware):** WP4c's pacing experiment, which is the only way
  to know whether `DontWait` + software timer can hold 120 on Windows and
  macOS. If it cannot, the answer is "120 is the screen-rate option and it is
  as smooth as the compositor allows", not a new setting or a vsync change
  of our own.
- **No-go:** a "120 fps" toggle or any new frame-rate setting (the dial has
  the screen rate; the first 120 Hz report already selects it); uncapped as a
  stand-in for 120 (a `Poll` busy loop); raising the pane cap before the app
  window paints at 120 (`#711`, PR `#966`).
- **Expected stability:** at 720p or Half resolution, plausible on a
  mid-range GPU for the median preset, with the heavy tail needing the
  adaptive quality from WP4b. At 1080p Full, the measured 8x preset spread
  and 2.45x fade make 120 unlikely for the heavy presets without dropping
  resolution during the fade. At any resolution, the synchronous load is a
  guaranteed multi-frame stall until WP2/WP3 land.

Effort for the 120-specific part: **about 1 day of code** (the fps report)
plus 1-2 days of pacing measurement; everything else is the transition plan
that is needed for 60 fps anyway.

A human must verify on a real GPU: frame-time distribution at 120 on a
120 Hz Windows 11 machine and on a ProMotion Mac, with and without a
crossfade, and whether the picture visibly tears or judders. llvmpipe cannot
answer either question.

---

## Q2. Can we add custom resizing for the MilkDrop window?

### 2.1 How the window and surface are made today

`child.rs:1009-1092` builds a winit window: title "MilkDrop", no decorations,
resizable, minimum 320x240 logical (`milkdrop.rs:91-94`), initial size from
`--milkdrop-size` in **logical points**. Then glutin creates the GL context
and a window surface sized from `window.inner_size()` (**physical pixels**),
and sets `SwapInterval::DontWait`. `--milkdrop-fullscreen` turns on
`Fullscreen::Borderless(None)` (`child.rs:334-344`, `1086-1090`).

Resizing is by the custom 16x16 corner grip (`child.rs:421-427`, `571-589`):
`drag_resize_window(SouthEast)`; dragging elsewhere moves the window; a
double-click toggles full screen. Geometry is reported to the app in logical
points (`child.rs:392-419`) and saved as `milkdrop_size` / `milkdrop_pos`
(`app.rs:4805-4811`; saved at most every 2 s, `app.rs:4695`).

The pane path has its own notion of size: `frames::fit`
(`frames.rs:82-88`) divides by `scale`, keeps the pane's shape, rounds to even
and caps the longest side at 1280 (pane) or 1920 (full-screen lyrics)
(`frames.rs:26-31`).

### 2.2 What happens on a resize

1. winit delivers `WindowEvent::Resized`; the child calls
   `surface.resize` and requests a redraw (`child.rs:499-507`).
2. The next frame passes the new physical size to `Engine::render`.
3. If the rounded, scale-divided inner size changed, `Engine::resize`
   (`engine.rs:303-356`) calls `projectm_set_window_size`, **deletes** the
   old FBO and texture and creates new ones.
4. projectM only stashes the size (`ProjectM::SetWindowSize`); its internal
   textures are reallocated at the next rendered frame (header comment on
   `projectm_set_window_size`).
5. The frame is rendered at the inner size and blitted twice (Q1.3).

So every pixel step of a drag is one full reallocation of our FBO plus
projectM's internal buffers. `Engine::render` is only called from
`RedrawRequested`, so it is at most once per frame, but a 120 Hz drag can ask
for 120 reallocations a second.

Aspect: projectM's aspect correction is on (`engine.rs:103`) and the
renderer computes `aspectX/aspectY` from the window size every frame
(`ProjectM::GetRenderContext`). With `scale` 1 the inner texture is the
window size, so the two blits do not change the picture's shape. With `scale`
2 or 4 the division and even-rounding can shift the ratio by a fraction of a
percent (e.g. 1001x753 → 500x376), which the NEAREST blit reproduces; that is
not visible on a full-screen effect.

DPI: the saved size is logical, the render size is physical. A 640x480
logical window on a 2x display renders 1280x960; a 1920x1080 logical window
on a 200%-scaled 4K display renders 3840x2160. **The standalone window has no
cap on the render size**, unlike the pane's 1280/1920 cap. That is the one
concrete cost bug in this area.

### 2.3 Platform differences

- **Windows**: grip resize works (winit sends `WM_NCLBUTTONDOWN` with the
  matching hit test); DPI changes are handled by winit/glutin, with a
  `Resized` event. Borderless full screen covers the monitor.
- **X11/Wayland**: grip resize works (`_NET_WM_MOVERESIZE` /
  `xdg_toplevel.resize`). Wayland forbids placing the window yourself, so
  `--milkdrop-pos` cannot work there; that is already accepted.
- **macOS**: `drag_resize_window` is **not implemented** in the winit fork
  this branch builds (`crmne/winit` `0bed9dc`, 0.30.13:
  `src/platform_impl/macos/window_delegate.rs:1179-1181` returns
  `NotSupported`; upstream 0.30.12 has the same stub at 1173-1175). The
  window is undecorated and has no OS resize edge, so
  today **the MilkDrop window cannot be resized at all on macOS** except by
  going full screen. This contradicts `docs/_guide/milkdrop.md` ("Drag the
  lower-right corner to resize it"), which is not platform-specific. This is
  a real, bounded gap and the strongest argument for touching resizing.

### 2.4 What "custom resizing" could mean

| Option | Today | Design | Risk / effort |
| --- | --- | --- | --- |
| Free resize | Yes, except macOS | macOS: handle the grip drag ourselves with `CursorMoved` + `request_inner_size`, since winit refuses. No native window code, so the "window procedures go to winit" rule does not bite. | 0.5-1 d + Mac verification. Small and worthwhile. |
| Aspect snapping / fixed sizes (16:9, 720p/1080p/4K) | No | Keys or a menu that call `request_inner_size` with a computed size. | New visible UI; needs visual approval; HiDPI and Wayland placement caveats; low value for a full-screen effect. 1-2 d. |
| Internal resolution independent of window ("render 1080p, stretch to the window") | Partly: `scale` 1/2/4 only | Cap the inner size the way `frames::fit` does (reuse the pane's 1920 cap, or the monitor's pixel size), and let the blit scale up. | No new setting, no UI. 0.5 d. Helps every high-DPI and 4K case, and 120 fps by making the render size bounded. |
| Coalesce resize storms | No | Mark a pending size and apply it only when it has been stable for one frame (or quantise to 8 px), stretching the last texture meanwhile. | 0.5-1 d; needs the measurement below to justify. |
| Borderless full screen | Yes (F, Alt+Enter, double-click) | Native full screen (exclusive) is not offered and should not be: projectM gains nothing and it breaks multi-monitor. | None. |
| Confine to a chosen monitor | Partly | `--milkdrop-pos` places it; no monitor picker. | Not worth it. |

### 2.5 Measurements (llvmpipe, relative only)

| Measurement | Result |
| --- | --- |
| Our reallocation: `projectm_set_window_size` + new FBO + new texture | median **0 ms**, p95 10 ms |
| The frame that follows a resize step | median **290 ms**, p95 660 ms, max 1200 ms |
| Jump 640x480 → 1920x1080 in one frame | **1200 ms**; steady at 1080p for the same preset 980 ms |
| Steady frame at 1080p, same preset | 980 ms |

The useful part: **our** `Engine::resize` is not the spike. projectM's
reallocation and the first render at the new size are, and they are timed
inside the next frame. On a GPU the absolute numbers shrink by orders of
magnitude, but a driver-side reallocation (a new 1080p texture and all of
projectM's internal buffers) is exactly the kind of thing that costs several
milliseconds and would show as a hitch at 120. Whether coalescing is worth
it cannot be decided from llvmpipe; WP1's frame telemetry during a drag on
the reference machine can decide it in an hour.

### 2.6 Verdict

**Go for two small, bounded changes; no-go for a resizing UI.**

- **Go (0.5-1 d): macOS grip resize.** It is broken today (winit returns
  `NotSupported`), and the guide already promises it. Handling the grip's
  drag ourselves with `request_inner_size` is ordinary winit API, not native
  window code, so the "window procedures go upstream" rule does not apply.
- **Go (0.5 d): cap the inner render size**, the way the pane already does
  (`frames::fit`, 1280/1920). Use one fixed policy: the longest side at the
  monitor's pixel size, or the existing `FULL_SCREEN_SIDE` = 1920. This
  bounds 4K and 2x-DPI windows, costs nothing in UI, and is 120 fps budget.
  It must be documented as a fixed policy, not a setting.
- **Conditional (0.5-1 d): coalesce resize steps** (apply a pending size
  after it has been stable for a frame, stretch the previous frame in the
  meantime). Do it only if WP1 shows a visible spike per drag step on
  hardware; the llvmpipe data cannot tell.
- **No-go now:** fixed sizes (720p/1080p/4K), aspect snapping, a monitor
  picker, exclusive full screen. They are a new visible UI surface for a
  full-screen effect, need maintainer approval and visual evidence, and
  solve no reported problem. `#704` (window size) is about the main window,
  not MilkDrop, and no upstream issue asks for this.

Effort: **1-1.5 days** for the two go items, **+1 day** if coalescing is
warranted. This is the cheapest of the three questions and the only one that
fixes something already promised to users.

A human must verify on a real GPU: a slow drag and a 640x480 → full-screen
jump on Windows and Linux, with the frame-time telemetry from WP1; and on
macOS, whether the self-driven grip resize feels like a window resize at all
(no live OS resize animation).

---

## Q3. Can we preview, enable and disable individual presets?

### 3.1 How presets are found, downloaded and chosen

- **Discovery**: `list_presets` walks the `milkdrop` folder up to four levels
  deep, keeps `.milk` files, and sorts by lower-cased path
  (`milkdrop.rs:404-425`). `has_presets` does the cheap version for the UI
  thread (`milkdrop.rs:440-455`). The folder is
  `~/.config/spotifast/milkdrop` (`paths.rs:150-152`).
- **Download**: two zips (`milkdrop.rs:114-125`), fetched on a worker thread
  (`milkdrop.rs:316-337`), unpacked **flat** by base name, skipping anything that is not
  a plain filename or not `.milk` (`milkdrop.rs:485-519`). An existing file
  with the same name is overwritten; there is no merge or manifest.
- **Listing lifecycle**: `Presets::refresh` starts a worker thread and keeps a
  listing for 30 s (`milkdrop.rs:181-211`); the UI triggers it
  (`ui/settings.rs:2084-2088`), and the child triggers it every frame
  (`child.rs:969`).
- **Selection**: `Presets` holds `files`, a 100-entry `history`, `at`,
  `locked` (L), `random` (R) and a pending `Request::Load`
  (`milkdrop.rs:135-154`, `223-301`). `pick` is either random (not the same
  preset twice while there is a choice) or the next file in folder order
  (`274-291`). `advance` switches on a wall-clock timer so a failed load
  cannot stop the turns (`248-255`).
- **Who owns the choice**: the **child process**, not the app. The child
  creates its own empty `Presets` and lists the folder itself
  (`child.rs:242`, `969-977`); the app's copy (`app.winamp.presets`) is used
  for the count and for downloads only (`ui/settings.rs:2084-2088`). The
  only app→child controls today are fps, seconds, scale, the song lines and
  close (`host.rs:176-213`); the only child→app ones are window geometry,
  screen rate, fullscreen, a closed flag and playback commands
  (`host.rs:33-42`).
- **Current preset name**: projectM 4 has no API for it (checked every header
  in `src/api/include/projectM-4`), so only the child knows; it shows it with
  T (`child.rs:650-652`, `792-800`). The app cannot show it today.

### 3.2 Where per-preset enabled/disabled state could live

The rule, quoted from `CONTRIBUTING.md:166-174`:

> Do not add a setting (a new field in `Settings`, a toggle, a slider, a
> choice of options) without the maintainer's approval in the issue first.
> The answer is usually no: prefer a good default that serves everyone, or a
> keyboard shortcut or command for the people who want something else. A pull
> request that adds a setting without that approval will be asked to remove
> it. CI fails when a pull request adds a field to a `Settings` struct in
> `src/settings.rs`, until the maintainer adds the `setting-approved` label.

AGENTS.md adds: "Do not add a setting without the maintainer's approval in
the issue first; prefer a good default, a shortcut or a command".

That rules out `milkdrop_disabled_presets: Vec<String>` in `Settings`. It
does not rule out a **state file**, which is how the app already keeps things
that are not preferences:

| Where | Precedent | Trade-off |
| --- | --- | --- |
| `~/.local/state/spotifast/milkdrop-presets.json` (next to `history.json`, `paths.rs:160-163`) | `history.json`, `session.json` | Survives a settings reset, is not a setting, written atomically through `fastframe_file::Writer` like history. Deleting it loses the curation; docs list it under "State". |
| `~/.config/spotifast/milkdrop/presets.json` inside the presets folder | none, but the folder is already app-managed (`unpack_presets`) | Travels with a copied config folder; a user deleting the preset folder loses both, which is consistent. It puts a non-`.milk` file where the docs promise presets; `list_presets` ignores it. |
| `session.json` | `milkdrop_pos` (`settings.rs:1902`) | Wrong semantics: "Last session ... Yes" in the docs, meant to be disposable. |

Recommendation: the state file. Format, one line per disabled preset, by
path relative to the folder, so a moved folder keeps working:
`{"version":1,"disabled":["Cream of the Crop/suksma - foo.milk"]}`.
Write it from the app's `Writer`; the child only reads it (or receives the
list over stdin), so there is one writer and no lock.

### 3.3 How a preview could work

Three designs, cheapest first. Note that "a second projectM instance" in this
codebase means **a second process**, not a second handle: winit allows one
event loop per process, and the hidden `--milkdrop-frames` child already is
that second process (`offscreen.rs`). Adding a second instance inside the
window child would need a shared GL context and a worker thread, which is
exactly the design the child process exists to avoid.

1. **"Play this one now"** (reuse the running engine). Add a `load` command
   to the child's `Control` (`child.rs:35-43`, `469-494`) and have `Presets`
   accept an explicit path (set `history.push(path)`, `at = len-1`, request a
   smooth or hard load). The user picks a row in Settings, the standalone
   window cuts or fades to it. No second instance, no thumbnails, no extra
   GPU cost beyond the load itself. For the pane child, `pane.rs` already
   holds the child's stdin (`pane.rs:58-61`, `_stdin`) but never writes to
   it; `offscreen.rs` would need a stdin command reader, or a path slot in
   the frames header (`frames.rs:50-66`), which is atomics-only today.
2. **Thumbnails, cached on disk.** A second hidden child (the existing
   `--milkdrop-frames` mode) renders each preset at 320x180 for N frames,
   reads it back and writes a PNG to `~/.cache/spotifast/milkdrop-previews/`,
   named by a hash of path + size + mtime. The UI decodes visible PNGs and
   shows them in a grid. This is exactly the shape of `skin_previews.rs`
   (worker thread, one at a time, PNG cache, bounded textures, uploads per
   frame, visible cards only; `skin_previews.rs:1-12`, `26-46`, `371-390`)
   and `paths.rs:208-210` already has a cache-folder precedent. Cost: one
   preset load plus ~10 frames per thumbnail. With loads at **37.8 ms**
   median and a 320x180 frame a fraction of the 480p cost measured above, a
   thumbnail is roughly 0.1-0.35 s here, and the load, not the pixels, is
   most of it. For all 9,795 presets that is tens of minutes to an hour of
   background work, so it must be on demand for visible rows.
3. **Hover preview (live).** The same hidden child renders the hovered preset
   at pane size and the UI shows the newest frame in a small box. Same
   machinery as (2) but no disk cache and higher GPU contention with the live
   window. Only worth it after (2).

Upstream has already considered a related request: **crmne/spotifast#1195,
"Favourites for Milkdrop presets", closed as completed**. The reporter's
minimum ask was to know the current preset's name, which T already does;
after a bot reply pointing at T (2026-11-02) they said they no longer found
it worth staying open, and the issue closed. There is no maintainer comment
on the feature itself, so this is not an approval, and no PR for a preset
browser exists.

### 3.4 UI integration points

- **Enable/disable the playing preset**: a key in the child window. It has a
  free key map (`child.rs:595-656`) and a text overlay for the answer
  (`show_note`, `725-742`); the key list (`658-722`) must gain the entry,
  and `docs/_guide/milkdrop.md` too. This is the "keyboard shortcut"
  CONTRIBUTING prefers, and needs no Settings field.
- **A list to browse and enable/disable**: Settings already has a MilkDrop
  section with rows and pack buttons (`ui/settings.rs:2084-2230`). A
  virtualised list of presets there needs an `Action` per toggle
  (`model.rs:1480-1501` is the existing set), an `app.winamp.presets` filtered
  view, and a `fastframe_file` write. That is a real interface addition:
  AGENTS.md's "Interface review" rules make it a reviewable redesign, so it
  needs before/after captures and, upstream, maintainer approval.
- **Preview**: the picture already reaches egui as `TextureId`
  (`pane.rs:41-50`, `181-184`), and `skin_previews.rs` shows how to make
  thumbnails cheap. A grid would live in Settings or a new window; either is
  a larger visual change than the toggles.

### 3.5 Custom scenes, validation and hot reload

This part already works: any `.milk` a user drops into
`~/.config/spotifast/milkdrop` (or up to three folders deep) is picked up by
the next listing, within 30 s (`milkdrop.rs:101`, `411-425`). Nothing is
needed for "add custom scenes". The gaps are quality of life:

- **Validation**: projectM reports a failed preset through
  `projectm_set_preset_switch_failed_event_callback` (`engine.rs:38-58`,
  `90-94`), which only logs. The previous preset keeps playing and the timer
  moves on next turn (`milkdrop.rs:248-255`, test
  `a_failed_load_does_not_stop_the_turns`). A browser should mark a preset
  that failed instead of retrying it every cycle: keep a "broken" set in
  memory (or the state file) when the callback fires for the requested path.
- **A pathological preset** (huge shader, long loop) can block the child
  longer than the pane's 15 s hang watchdog (`pane.rs:29`), which restarts it
  (`249-294`). A thumbnail worker needs its own timeout for exactly this; the
  skin preview's `BUDGET` is the model (`skin_previews.rs:44-45`).
- **Hot reload**: the 30 s listing is the reload. A "refresh" button or
  re-listing when the folder's mtime changes would make it feel immediate;
  the listing itself runs on a thread (`milkdrop.rs:196-210`), so it is cheap
  to trigger.
- **Naming collisions**: packs are unpacked flat by base name
  (`milkdrop.rs:502-519`), so downloading a pack writes over any custom file
  with the same name. That is already true today and matters more once users
  curate a folder; a browser should at least show the folder a preset came
  from, and a download should not silently replace an enabled custom scene.

### 3.6 Verdict

**Go for enable/disable and "play this one"; conditional for thumbnails.**

- **Go (2-3 d): per-preset enable/disable, driven by a key in the child
  window.** A key such as X marks the playing preset enabled or disabled,
  the list persists in `state/milkdrop-presets.json`, `pick()` and the
  automatic advance skip disabled presets, and Previous/Next still reach
  them when asked explicitly. No `Settings` field, no new UI surface: this
  is the "shortcut or command" CONTRIBUTING prefers. Tests go in
  `src/milkdrop.rs` next to the existing history/order tests.
- **Go (2-3 d more): "play this preset now"** from a list in Settings.
  Needs a `load` control on the child's stdin (`host.rs` already writes
  JSON-lines; `Control` gains a field), the app's `Presets` list to drive
  the rows, and an `Action` per click. It is still a list, not a browser.
- **Conditional (5-8 d): thumbnails / hover preview.** The machinery
  exists (`skin_previews.rs`, the pane child, the PNG cache folder) but it
  doubles GPU load and is a visible feature that needs maintainer approval
  and before/after evidence. Only worth it if the list is wanted at all.
- **Already done: custom scenes.** Any `.milk` dropped in the folder works
  within 30 s, with no code. What is missing is only quality of life: mark
  a preset that failed to load as broken (the failure callback already
  fires and only logs), and offer a re-list when the folder changes. 1-2 d.
- **Upstream fit is the weakest of the three.** #1195 was closed by its
  reporter as completed after learning T shows the name, so there is no
  maintainer decision to point at; a favourites pool needs a new issue and
  approval, and a full browser with thumbnails cuts against the "Focused"
  and "One look" principles. The key + state file is the smallest form that
  plausibly fits, and it is also the one a fork can carry alone.

A human must verify: the child window's new key and overlay text on all three
platforms. If thumbnails are built, that the grid scrolls without stuttering
while MilkDrop plays (two GPU consumers at once).

---

## Prioritised roadmap

Ordered by what unblocks the most. "WPn" are the work packages in
`PLAN-milkdrop-transitions.md`.

1. **WP1 telemetry** (already planned, 1-2 d). Nothing below can be judged
   without frame-time and load-time numbers from the child. It is also what
   turns every conclusion in this file into a hardware fact.
2. **WP2 + WP3, the load hitch** (already planned, 3-6 d). The biggest visible
   defect at any rate and the hard blocker for 120: a synchronous load blocks
   the render thread for its whole parse and shader compile, so at 120 fps a
   load of `L` ms drops `L / 8.33` frames.
3. **Q2-A: macOS grip resize + a cap on the inner render size** (1-1.5 d,
   independent of the transition work). Fixes behaviour the guide already
   promises on macOS, and bounds the render size on high-DPI and 4K displays,
   which is also 120 fps budget. No new setting.
4. **WP4a/b: fade pacing and adaptive quality** (already planned, 3-4 d).
   Required for 120 during fades, where projectM renders both presets.
5. **WP4c: pacing experiment, which decides Q1** (1-2 d plus hardware time).
   Compare `DontWait` + software timer with swap-interval 1 at the screen
   rate, on Windows and macOS only. This is the test that says whether a
   stable 120 is possible in the child at all; do not change pacing before it.
6. **Q1 follow-up** (0.5 d): report the real rate to projectM
   (`projectm_set_fps`), keep the screen-rate default, and document 120 as
   "your screen" rather than a separate mode. No new setting.
7. **Q3 enable/disable, plus play-now** (4-6 d): the child window key and the
   state file first (2-3 d), the Settings list with a play button after
   (2-3 d, interface review needed). Independent of the transition work and
   safe to do in parallel.
8. **Optional, only with approval**: Q3 thumbnails / preset browser (5-8 d)
   and Q2-B fixed sizes or aspect snapping (1-2 d). Both are visible features
   that need a maintainer decision and visual evidence; neither is needed for
   the transition goals.

What not to do:

- Do not raise the pane cap to 120 before the app window itself paints at
  120 (`#711`, open PR `#966`). It would burn GPU for frames nobody sees.
- Do not treat Uncapped as a 120 mode: it is a `Poll` loop at 100% of a core.
- Do not add a frame-rate setting or a "120 fps" toggle. The dial already has
  the screen's rate, and the first report of a 120 Hz screen already selects
  it.
- Do not add a second projectM instance for previews until the thumbnail
  worker has been shown not to steal frames from the live window (skin
  previews is the model: one worker, one item at a time, visible rows only).

## Open risks

- **The measurements here are llvmpipe.** They overstate absolute cost and
  can misrank presets against a real GPU (fill-rate bound here, shader or
  bandwidth bound there). Every claim about 120 fps stability needs the
  reference machines.
- **Presentation may cap 120 regardless of render cost.** With
  `SwapInterval::DontWait` there is no phase lock to the compositor; on
  Windows and Wayland the achievable cadence at 120 can be "mostly 120, with
  regular single-refresh waits". WP4c must measure it; if it cannot be fixed
  without a vsync decision, say so rather than adding one (AGENTS.md).
- **macOS 120 Hz on an undecorated NSOpenGL window is unverified.** The
  #711/#966 discussion is about eframe's window, not this child.
- **Build host contention.** The measurement runs above were made on a
  shared build host; the probe reports process CPU time per frame so other
  builds do not inflate the render numbers, but load times still include
  scheduling and disk.
- **Adaptive quality and resize coalescing both reallocate** the copy FBO and
  projectM's textures. Used together without hysteresis they can pump. One
  control (the render size) should own the reallocation.
- **A preset browser doubles the GPU load** (live window plus a thumbnail
  worker) and adds a cache folder. A pathological preset must time out like
  `skin_previews`' `BUDGET`, or the worker hangs on someone's `.milk`.
- **A per-preset state file is a new user-visible file.** It needs a line in
  `docs/_reference/settings-and-files.md`, a decision about what deleting it
  means, and one writer (the app), with the child only reading it.
- **Upstream fit is unproven for Q3.** `#1195` was closed by its reporter as
  completed, not by a maintainer decision. A favourites pool or a preset
  browser needs a new issue and approval; the "one look" and "focused"
  principles in `CONTRIBUTING.md` argue against a large new UI surface, while
  a keyboard toggle plus a state file is the smallest form that fits the
  "prefer a good default, a shortcut or a command" rule.

## Appendix: reproducing the measurements

The harness is `examples/milkdrop_probe.rs` on this branch; it is a
throwaway, not shipped code. It builds only with the `milkdrop` feature
(default) and uses the same EGL device context as the pane's windowless
child on Linux.

```sh
# On a Linux host with a display or xvfb, from the repository root:
cargo run --release --example milkdrop_probe -- \
  --presets scratch/presets --limit 30 --frames 10 --mode all
```

The numbers above came from one such `--mode all` run with `--limit 30
--frames 10` (30 presets, 10 timed frames each) plus `--mode cost` passes at
all three sizes.

`--mode` is `cost`, `load`, `transition`, `resize`, or `all`. Frame times
are this process's CPU time from `/proc/self/stat`, so other work on a shared
build host does not inflate them (llvmpipe rasterizes on our own threads).

The 120-preset sample is not committed (third-party preset text). It was
every 80th `.milk` of
`https://github.com/projectM-visualizer/presets-cream-of-the-crop/archive/refs/heads/master.zip`
(9,795 files, 120 taken, 1.4 MB in all), flattened and numbered. Re-fetch that
zip to reproduce; the pack changes upstream, so exact files can drift.

On `server1`, all of this runs through `sfb`:

```sh
sfb gl cargo run --release --example milkdrop_probe -- \
  --presets scratch/presets --limit 30 --frames 10 --mode all
```

`sfb` uploads the checkout to the build host, which has no GPU: it renders
with llvmpipe under xvfb, so its numbers are relative.

Checks on this branch (build host, pinned 1.98.0 toolchain):
`cargo test --lib milkdrop` 50 passed, 0 failed; `cargo clippy --lib -- -D
warnings` clean; `cargo fmt --check` clean; `milkdrop_probe` builds and runs.
No library or UI code was changed.

