# MilkDrop preset-switch performance notes

Numbers for `PLAN-milkdrop-transitions.md`, kept in one place so every work
package compares like with like. Nothing here promises how a real GPU
behaves: the build host draws with Mesa llvmpipe on the CPU, so absolute
milliseconds describe that machine, and the ratios and load times are the
part that carries over.

## How to measure

`examples/milkdrop_stutter.rs` opens the same hidden window, OpenGL context
and `Engine` the MilkDrop window uses (mesh 48x36, soft cut 2.7 s,
`SwapInterval::DontWait` with the child's software frame clock), feeds
synthetic stereo PCM (sines and noise bursts) and cycles a fixed sample of
presets every 3 s. For each switch it prints the synchronous `Engine::load`
time in `load=`, the whole frame that paid for it in `stall=`, the worst
frame until the next switch, and p50/p99 for the frames during the fade and
for steady frames. The summary ranks the slowest loads and reports how many
presets took over 8 ms to load.

```sh
# once, or when the folder has no presets: downloads the app's two packs
sfb gl cargo run --release --example milkdrop_stutter -- --presets .perf-presets --fetch
# the usual run
sfb gl cargo run --release --example milkdrop_stutter -- --presets .perf-presets --count 200 --seconds 3
# hard cuts instead of fades, for a control
sfb gl cargo run --release --example milkdrop_stutter -- --presets .perf-presets --count 20 --seconds 3 --hard
```

`--presets` defaults to `/tmp/spotifast-presets`. Presets are never
committed. `--fetch` downloads the app's two packs in full, so its 200
presets are a different stride than the folder described below; use the
same folder for both runs of a before/after comparison. Each `sfb`
invocation uploads the tree and rebuilds the release library and example
(about 6 to 15 minutes on the shared build host, depending on what else is
building), so batch several runs rather than starting them one by one.

The same measurements are available from the child process during normal
use when `SPOTIFAST_MILKDROP_TRACE` is set in its environment: one
`milkdrop trace: switch ...` line per switch (`load_ms`, the first 30
frames' p50 and max, and the worst frame in the next second) and one
`milkdrop trace: summary ...` line when it exits. That is for local
investigation; it is off, and costs one cached check per frame, when the
variable is unset.

## Baseline (llvmpipe, 2026-11-03)

Environment: build host `desktop-kc5chj8` (WSL, Linux), Xvfb, Mesa llvmpipe
LLVM 21.1.8, OpenGL 4.5 Core Profile (Mesa 26.0.8), release build, 640x480,
60 fps requested clock, `CROSSFADE_SECONDS = 2.7`. The preset folder held
the 552 `.milk` files of MilkDrop's own pack plus 200 files taken from Cream
of the Crop (every `i * 9795 / 200`-th `.milk` of the sorted archive, 200
files); `--count 200` then strides that sorted folder. Other work packages
were building and running GL harnesses on the same host during these runs,
which adds noise.

Exact command:

```sh
sfb gl cargo run --release --example milkdrop_stutter -- --presets .perf-presets --count 200 --seconds 3
```

### Cold and warm

Two runs of the same command, back to back, 200 presets, 609 s each. Mesa
keeps compiled shaders on disk and Linux keeps the preset files in page
cache, so the second run is the one to compare a candidate against: run the
candidate after one warm-up run of the same sample.

| metric | first run (cold) | second run (warm) |
|---|---:|---:|
| preset load p50 (ms) | 14.4 | 10.2 |
| preset load p99 (ms) | 84.9 | 24.8 |
| preset load max (ms) | 112.3 | 26.4 |
| loads over 8 ms | 117 of 200 | 107 of 200 |
| fade frame p50 (ms) | 70.2 | 71.1 |
| fade frame p99 (ms) | 165.5 | 136.8 |
| fade frame max (ms) | 396.1 | 305.8 |
| steady frame p50 (ms) | 48.2 | 47.5 |
| steady frame p99 (ms) | 78.9 | 83.0 |
| steady frame max (ms) | 134.0 | 193.3 |
| worst frame per switch p50 (ms) | 130.1 | 97.8 |
| worst frame per switch p99 (ms) | 918.0 | 267.9 |
| worst frame per switch max (ms) | 1044.4 | 345.7 |
| worst frame in the 1 s after a switch, p99 (ms) | 809.6 | 256.2 |
| stall frame p50 (ms) | 114.0 | 92.0 |
| stall frame p99 (ms) | 918.0 | 256.2 |
| fade p99 / steady p99 | 2.10x | 1.65x |
| achieved frame rate | 13.2 fps | 14.0 fps |

The stall frame is the whole frame that contains the load. It is far larger
than the load itself (warm p50 92 ms against 10 ms), so in this environment
most of the hitch is the first draw of a newly loaded preset (shader
compile and link, and llvmpipe's JIT of the generated shaders), not reading
and parsing the file. WP3 should look there. WP2 should show mostly in the
load row.

Slowest loads in the cold run (warm values are lower because the files were
already in page cache): `martin - butterflies.milk` 112.3 ms,
`Flexi - fractal jellyfish [moebius mix].milk` 97.1 ms,
`Unchained - Quine.milk` 84.9 ms, `276 loavthepsyqulux trux.milk` 83.7 ms,
`martin - golden mirror.milk` 70.6 ms, then a tail between 65 and 50 ms
(martin, amandio c, waste, Goody + martin, MegalomaGreg, Flexi, Rovastar,
Zylot). The full ranked top 20 of any run is printed by the harness.

### Hard cuts against fades

The 20-preset hard run used the same folder and argument as the soft runs,
with `--hard`; its 20 presets are a subset of the 200-preset sample, so the
table pairs each of the 20 against its warm soft run. In hard mode the
"fade window" label still covers the first 2.7 s after a switch, but only
the new preset is drawn there, which is what makes it a control for the
cost of the crossfade itself.

| paired medians, 20 presets | soft cuts | hard cuts |
|---|---:|---:|
| preset load (ms) | 10.6 | 10.6 |
| stall frame (ms) | 92.8 | 72.2 |
| worst frame per switch (ms) | 93.8 | 72.2 |
| frames in the fade window, p50 (ms) | 68.8 | 49.8 |
| frames in the fade window, p99 (ms) | 85.2 | 59.0 |
| steady frame p50 (ms) | 50.5 | 51.0 |
| steady frame p99 (ms) | 52.9 | 54.8 |
| frames drawn across the 20 switches | 890 | 1180 |

The fade window costs about 1.4x the steady frame in this environment, and
25% of the frames that a hard cut draws in the same wall time are lost to
it. Steady frames and loads are the same either way, so the difference is
the double rendering during a fade.

A smaller run at 320x240 drew frames in about the same time as 640x480
(steady p50 49.4 ms against 47.5 ms), so on llvmpipe the frame time is not
mainly pixel count; do not read the resolution as a way to buy frames here.

## WP3: the first draw is most of the switch (llvmpipe, 2026-11-03)

WP3 split a switch into its two halves and budgeted them. The load is
`Engine::load` (read, parse, HLSL-to-GLSL); the first draw is the first
`engine.render` of the new preset, where the driver compiles and links its
shaders. The harness now prints `load=`, `first_draw=`, `switch=` (their
sum) and `other=` (the rest of the stall frame) on every switch, ranks the
slowest switches with both halves, and prints the PCM fed each frame. The
child's `SPOTIFAST_MILKDROP_TRACE` reports the same split.

Same host, folder, sample and command as the baseline above:

```sh
sfb gl cargo run --release --example milkdrop_stutter -- --presets .perf-presets --count 200 --seconds 3
```

Two runs before the change (the WP1 harness at the merge commit `d331d61`)
and two after. The second run of each pair is the table's warm run; Mesa's
shader cache already held the sample from the first pair, so both tables
read as warm. The two after runs are 609.5 s and 609.4 s; the two before
runs are 609.1 s and 609.2 s.

| metric, warm | before | after |
|---|---:|---:|
| preset load p50 / p99 / max (ms) | 10.2 / 34.8 / 53.9 | 9.4 / 26.3 / 29.2 |
| first draw p50 / p99 / max (ms) | not measured | 67.0 / 164.2 / 236.1 |
| switch (load + first draw) p50 / p99 / max (ms) | not measured | 75.4 / 183.8 / 251.1 |
| stall frame p50 / p99 / max (ms) | 94.5 / 229.3 / 263.8 | 94.8 / 204.0 / 271.1 |
| worst frame per switch p50 / p99 / max (ms) | 96.7 / 239.4 / 263.8 | 98.6 / 204.0 / 271.1 |
| worst frame in the 1 s after a switch, p50 / p99 / max (ms) | 96.3 / 239.4 / 263.8 | 97.4 / 204.0 / 271.1 |
| frame ms during fade, p50 / p99 (ms) | 75.7 / 130.5 | 76.1 / 142.7 |
| frame ms steady, p50 / p99 (ms) | 50.0 / 78.8 | 50.7 / 80.1 |
| fade p99 / steady p99 | 1.66x | 1.78x |
| achieved frame rate | 13.2 fps | 13.1 fps |
| loads over 8 ms | 103 of 200 | 101 of 200 |
| switches over 50 ms | not counted | 185 of 200 |

At the median, the load is 9.4 ms and the first draw 67.0 ms, so about 88%
of a switch is the first draw. Everything the before run could measure agrees
within run-to-run noise: the budget changes which presets automatic play
chooses, not what a switch costs, so there is no frame-time gain to show for
it. The 50 ms budget is three 60 Hz frames, well above the plan's 25 ms frame
ceiling; see `src/milkdrop/budget.rs` for why it is deliberately not the
criterion itself.

The warm switch-cost distribution after is: min 38.1, p10 52.6, p50 75.4,
p90 109.6, max 251.1 ms. Fifteen presets of the 200 are at or under the
50 ms budget, so on llvmpipe the skip list would narrow automatic play to
those fifteen and fall back to the rest once they have all played; on a GPU
with a driver shader cache the list should be much shorter. The slowest
switches are shader-heavy presets, not large files:

| rank | switch (ms) | load (ms) | first draw (ms) | preset |
|---:|---:|---:|---:|---|
| 1 | 251.1 | 15.0 | 236.1 | suksma - criminally negligent craft and bake sale ... |
| 2 | 234.4 | 15.4 | 218.9 | $$$ Royal - Mashup (255) |
| 3 | 183.8 | 19.7 | 164.2 | suksma - selectron validumb blegula auralsect muert |
| 4 | 174.3 | 12.6 | 161.8 | suksma - veiny texture of your m gloves ... |
| 5 | 155.3 | 26.3 | 129.0 | amandio c - polar value [shader pimped] |
| 6 | 143.3 | 19.6 | 123.7 | Adam Eatit Mashup FX 2 martin - disco mix + Lodus ... |
| 7 | 139.9 | 12.9 | 127.0 | suksma - dotes hostile undertake ... |
| 8 | 134.5 | 18.8 | 115.7 | shifter + Flexi - liquid circuitry ... |
| 9 | 129.2 | 18.4 | 110.8 | suksma - fiShbRaiN - witchcraft (phugly shame on remix) ... |
| 10 | 126.1 | 19.9 | 106.1 | Flexi - intensive shader fractal [suksma comp shader mix] |

### The sound after a long frame

`prepare_frame` calls `Ring::since` once per frame and feeds every frame it
returns, in order. A long frame therefore hands projectM the audio that frame
missed in one call. That is catch-up, not duplication: the cursor moves past
it, the ring test in `shm.rs` checks nothing is fed twice, and projectM's
analysis is counted in samples, so dropping or spreading the burst would move
the picture behind the music for good. The harness now prints the count:
warm, p50 3559 samples (74 ms at 48 kHz), p99 6831, max 13029 (271 ms, on the
slowest switch frame). The ring holds 22050 frames (459 ms), so neither run
lost a sample. On this host every frame hands over more than two 60 Hz frames
of audio, only because llvmpipe draws at 13 fps; a two-frame cap would cap
every frame here and starve the analysis on any machine that cannot hold
60 fps. No feed code was changed. The harness's own audio cap was corrected
from one second to the ring's 22050 frames so it draws from the same sound
the child would.

### When the load runs

`Child::render` calls `prepare_frame` first, so a switch is the first work
after the `RedrawRequested` that `about_to_wait` asked for when the frame's
deadline passed. There is no work in front of the load to move it behind, and
moving the load later in the frame would only move the same stall to the next
frame. The harness's `other=` column, the stall frame minus load and first
draw, is about 20 ms on llvmpipe: the previous frame's present, the audio
feed and the frame clock, none of it ahead of the load. No code was changed
for this part. WP4c covers the other half, how the frame deadline relates to
a real display's vertical blank.

## WP4a: the soft cut runs on the wall clock, not on frames

Sources read at the revision this fork actually builds: `projectm-sys` is
patched in `Cargo.toml` to `crmne/projectm-rs` `93852ff7299e5a63ed1315437593fc70915c0863`,
whose `libprojectM` submodule is `crmne/projectm` `ffb109857b9d31d71eef7df44f6e34b5a788e556`
(branch `spotifast`).

- `src/libprojectM/Renderer/PresetTransition.cpp`: `Progress(currentFrameTime)`
  is `(currentFrameTime - m_transitionStartTime) / m_durationSeconds`, clamped
  to `[0, 1]`, and `Draw()` passes the same `secondsSinceStart` to the
  transition shader as `durationParams`/`timeParams`. There is no frame
  counter in the transition at all.
- `src/libprojectM/ProjectM.cpp` `RenderFrame()`: `m_timeKeeper->UpdateTimers()`
  runs first (line 125); the fade is ended when
  `m_timeKeeper->SmoothRatio() >= 1.0` (line 166); the transition is drawn
  with `m_timeKeeper->GetFrameTime()` (line 208).
- `src/libprojectM/TimeKeeper.cpp` `UpdateTimers()`: with no user clock set,
  the timestamp is `std::chrono::high_resolution_clock::now()` minus the
  start time. `m_userSpecifiedTime` defaults to `-1.0`
  (`TimeKeeper.hpp`), and Spotifast never calls `projectm_set_frame_time`, so
  the default wall clock is always the one used. `SmoothRatio()` is
  `(m_currentTime - m_presetTimeB) / m_softCutDuration`, again seconds.

`projectm_set_fps(60)` does not pace anything: `projectm_set_fps` calls
`ProjectM::SetTargetFramesPerSecond`, which only assigns `m_targetFps`
(`ProjectMCWrapper.cpp` line 363, `ProjectM.cpp` line 486). Its single reader
is `RenderContext::fps` (`ProjectM.cpp` line 583), which becomes the read-only
`fps` variable of a preset's per-frame equations (`PerFrameContext.cpp` lines
46 and 168). Nothing in `RenderFrame`, `TimeKeeper` or `PresetTransition`
reads it. Rendering is driven only by our `Child::render` calls.

What this means for pacing:

- A fade cannot be stalled by our `fps` limit or by `schedule_next_frame`. At
  10 fps the fade still finishes 2.7 s after it starts, in 27 drawn frames; at
  60 fps in 162.
- A long frame does not freeze the fade, it skips it: the frame that takes
  200 ms advances the fade by 200 ms, about 7% of it. A low fps or a stall
  makes the fade coarse, never longer than its timer.
- `projectm_set_soft_cut_duration(CROSSFADE_SECONDS)` and
  `Presets::fading()` use the same constant, so the governor's idea of when a
  fade is running matches projectM's.
- One thing to keep in mind: `Presets::started` (and so the next preset turn,
  and the new `fading()` window) is stamped when the preset is *asked for*,
  before the synchronous `projectm_load_preset_file`. The load time therefore
  comes out of the 2.7 s fade, by design of the existing code; the fade after
  a 200 ms load is 2.5 s of visible time. This is unchanged by WP4b.

## WP4b: adaptive detail during a fade

`Presets::fading(now)` answers whether a soft cut is still running, from the
same `CROSSFADE_SECONDS` projectM is given, and `Child::render` feeds the
frame's own cost (the work from `prepare_frame` through `swap_buffers`) to
`milkdrop::governor::Governor`. When 5 of the last 8 frames of a fade ran over
the frame budget, the governor multiplies the render scale by 2 (a quarter of
the pixels) for the rest of the fade; it never restores inside a fade, and it
holds the smaller picture for 2 s after the fade so the seam does not flicker.
The budget is the window's own frame limit, or 60 fps when it is uncapped.
A fade the machine keeps up with never steps down, and frames outside a fade
never step down at all (a slow preset is the quality the user picked). The
logic is covered by `cargo test --lib milkdrop` with a fake clock, including
"one stall is not enough", "no pumping inside a fade", "the hold", and "the
next fade decides afresh".

### What the lever is worth, measured

`examples/milkdrop_scale_probe.rs` loads a preset with a hard cut, starts a
real soft cut through libprojectM with the next one, and times 8 frames of the
fade at the window's size, then starts the same fade again and times 8 frames
at half the width and height. The frame is timed around
`projectm_opengl_render_frame_fbo`, so it includes the readback the pane path
does; on llvmpipe that is part of the cost, on a real GPU it is a memcpy after
the draw.

12 presets from `presets-milkdrop-original` (the 552 `.milk` files of
`https://github.com/projectM-visualizer/presets-milkdrop-original`), taken one
in every 46 in name order, llvmpipe, `sfb gl`:

```sh
sfb gl cargo run --release --example milkdrop_scale_probe -- <folder> 640x480
sfb gl cargo run --release --example milkdrop_scale_probe -- <folder> 1920x1080
```

| window | steady | fade, full | fade, half | half / full |
|---|---|---|---|---|
| 640x480 (0.31 MP) | 22.8 ms | 45.3 ms | 37.6 ms | 83% |
| 1920x1080 (2.07 MP) | 37.0 ms | 84.7 ms | 46.9 ms | 55% |

- A fade costs about twice a steady frame at both sizes (1.99x and 2.29x),
  which is the premise of the whole work package, and every frame of every
  measured fade was over the 16.7 ms budget on llvmpipe.
- The half-size step removed 7.7 ms of a 45.3 ms fade frame at 640x480, but
  37.8 ms of an 84.7 ms one at 1080p. The 640x480 number is the one to
  distrust: llvmpipe's per-pass overhead (30-40 ms per fade frame here, nearly
  independent of resolution) swamps the fill cost that the scale divides. At
  1080p fill dominates and the lever is close to what a fill-bound GPU would
  give, since the step quarters the pixels the two presets and the transition
  shader touch.
- So on the machines this is for (a GPU that reaches 60 fps for one preset but
  not for two), the step should bring a fade frame of 20-30 ms to 11-17 ms.
  The exact figure needs a real GPU; llvmpipe cannot show it.

Limits of this measurement: it is the pane's code path (framebuffer object
plus readback), not the window's blit to the default framebuffer, and it does
not include the window's timer or compositor. It also deliberately skips the
first frame after a size change, which pays for projectM making its
framebuffers again; that one-frame cost is unmeasured and is why the governor
takes its step at most once per fade.

The governor watches the work inside a frame, not the gap between frames: the
gap includes the pacing timer's own sleep and its jitter (up to a whole timer
tick on a machine with a coarse clock), and a governor that reacted to that
would shrink the picture on a GPU that is keeping up perfectly. The other side
of that choice is that a GPU which drops frames silently, without ever
blocking the driver queue, looks cheaper than it is; on a real GPU that is
what `swap_buffers` and the next frame's first GL call do block on once the
queue is full, with a frame or two of lag.

The step is multiplied with the user's own quality setting and capped at 4
(about a quarter of the width and height), so someone who already chose the
smallest picture gets no further reduction from it.

## WP4c: frame pacing

The window draws with `SwapInterval::DontWait` and paces itself with a timer:
`Child::schedule_next_frame_at` moves a deadline on by the frame interval and
`about_to_wait` sleeps until it with `ControlFlow::WaitUntil`.

### What the timer does, measured

`examples/milkdrop_pacing.rs` runs the same wait loop with no window and
records, for 600 frames asked at 60 fps: the interval between wakes, and how
long after the deadline each wake happened. Run on the 24-core Linux build
host through `sfb gl` (winit 0.30 fork, Mesa llvmpipe under Xvfb; the timer
path is the same one the child uses):

```sh
sfb gl cargo run --release --example milkdrop_pacing -- 60
```

```
asked for 60 fps (16666 us), 599 frames measured
frame interval (us): mean 16665.2 (ideal 16666.7), sd 119.2, p50 16668.7, p99 17027.8, max 17316.9
wake lateness (us): mean 232.2 (ideal 0.0), sd 92.4, p50 205.9, p99 654.4, max 1176.8
gaps over 1.5 intervals: 0 (0.00%), under 0.5: 0 (0.00%)
```

The mean lateness includes a constant offset from the deadline being set
before the loop started; what matters for judder is the spread: sd 0.12 ms,
p99 0.65 ms, worst 1.18 ms, and no doubled or halved intervals in 599 frames.
On Windows the same wait is a `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` timer
(winit's `platform_impl/windows/event_loop.rs`), not the old 15.6 ms clock, so
a sub-millisecond spread is the expected order there too; on macOS it is an
AppKit run-loop timeout.

### Why the display still judges it

`DontWait` means the swap only hands the frame to the compositor. The timer
has no relationship to the display's vertical blank:

- At 60.000 fps against a 59.94 Hz panel the app produces 0.06 frame a second
  too many, so the compositor drops one every ~17 s: a single 33 ms hold, then
  back on the beat.
- A wake up to 1.2 ms late costs a whole refresh (33 ms) whenever the deadline
  falls within that 1.2 ms before a vblank, about 7% of the phase cycle.
- Nothing in the timer path keeps the phase, so those slips happen at
  arbitrary moments instead of being corrected on the next frame.

With `SwapInterval::Wait(1)` the swap blocks until the next vblank, so the
display sets the rhythm: intervals are the refresh period (jitter is the
display clock, tens of microseconds), there is no frequency mismatch and no
phase to drift. Its cost is the mirror image: a frame that overruns one
period misses a whole vblank and the next frame starts from that vblank, so a
GPU sitting at 17 ms a frame drops to a steady 30 fps instead of the
occasional 33 ms hold the queue is able to hide. It also only works when the
chosen fps equals the screen rate, and it adds up to one refresh of latency.

### What could not be measured here, and the decision

None of the display half can be measured on server1 or the build host: there
is no display, and llvmpipe under Xvfb ignores the swap interval and has no
vblank at all. The timer numbers above describe only half of the comparison,
and they cannot show what a real compositor does with a frame that arrives a
millisecond after a vblank.

No pacing code was changed. The data does not clearly favour swap interval 1:
the timer is accurate to a fraction of a millisecond, the known defects of the
timer path are the steady-state effects above, and the known defect of
interval 1 (a hard drop to half rate as soon as frames overrun) is worst on
exactly the machines this work package is about. A human with a Windows 11
machine at 60 Hz and a Mac at 60 Hz should A/B it before anyone changes it:
set `SwapInterval::Wait(1)` only when `fps == screen_hz` (which the child
already knows from `screen_hz`), leave Linux as it is because the egui fork
paces Wayland frames by the compositor's callbacks (AGENTS.md), and compare
frame-interval histograms from WP1's trace plus the eye.

## WP5: the integrated branch (llvmpipe, 2026-11-03)

WP4 merged into WP2 and WP3, then the same command and the same 200-preset
sample as the tables above, twice, back to back. The second run is the warm
one and the comparable one. The host was busier than on the WP3 day (the
first run's switch tail reached 1.2 s, and its own fade ratio was 2.28x), so
the first run is given only for the noise it shows.

| metric, warm, 200 presets | WP1 baseline | WP3 after | integrated |
|---|---:|---:|---:|
| preset load p50 / p99 / max (ms) | 10.2 / 24.8 / 26.4 | 9.4 / 26.3 / 29.2 | 11.2 / 28.4 / 35.6 |
| first draw p50 / p99 / max (ms) | not measured | 67.0 / 164.2 / 236.1 | 68.8 / 213.7 / 218.5 |
| switch p50 / p99 / max (ms) | not measured | 75.4 / 183.8 / 251.1 | 81.5 / 230.8 / 239.0 |
| stall frame p50 / p99 (ms) | 92.0 / 256.2 | 94.8 / 204.0 | 101.2 / 234.8 |
| worst frame per switch p50 / p99 / max (ms) | 97.8 / 267.9 / 345.7 | 98.6 / 204.0 / 271.1 | 103.3 / 234.8 / 259.4 |
| worst frame in the 1 s after, p99 (ms) | 256.2 | 239.4 | 234.8 |
| frame ms during fade, p50 / p99 (ms) | 71.1 / 136.8 | 76.1 / 142.7 | 76.1 / 134.4 |
| frame ms steady, p50 / p99 (ms) | 47.5 / 83.0 | 50.7 / 80.1 | 49.6 / 78.4 |
| fade p99 / steady p99 | 1.65x | 1.78x | 1.71x |
| achieved frame rate | 14.0 fps | 13.1 fps | 13.0 fps |
| loads over 8 ms | 107 of 200 | 101 of 200 | 112 of 200 |
| switches over 50 ms | not counted | 185 of 200 | 190 of 200 |

Every median agrees with the WP3 run within 10%, and the fade window costs
1.71x a steady frame against 1.78x. That is the expected result, not a gain:
`examples/milkdrop_stutter.rs` drives `Engine` itself, so the prefetch worker,
the slow list and the fade governor (all of them in the child) are not on its
path. The harness can show a regression in the engine and the crossfade
constant; it cannot show a gain from WP2 to WP4.

### The governor's lever on the same tree

`examples/milkdrop_scale_probe.rs`, same day and host, on the integrated tree,
12 presets from the same folder, 8 frames per fade:

| window | steady | fade, full | fade, half | half / full |
|---|---:|---:|---:|---:|
| 640x480 | 27.4 ms | 56.7 ms | 50.9 ms | 90% |
| 1920x1080 | 42.3 ms | 90.8 ms | 59.8 ms | 66% |

WP4's earlier run of the same probe read 83% and 55%; this host is slower
today and llvmpipe's per-pass overhead swamps the fill the scale divides at
640x480, so the two disagree where the measurement is weakest and agree in
direction. At 1080p the step removed 31 ms of a 90.8 ms fade frame here. What
this says for a real GPU is only that the lever quarters the pixels the two
presets and the transition shader touch; the fill-bound gain itself needs a
GPU with a display.

### The two work packages that could have fought

The governor and the switch budget both judge a frame, and each can mislead
the other: a switch drawn while the step down is held measures a quarter of
its real first draw, which would let a slow preset back into automatic play;
and the long frame that loads a preset would, inside a fade, count as a fade
that cannot keep up when the user is pressing Next. The governor now refuses
to judge a frame that loaded a preset, and the child only records a switch
cost while the governor is at full detail. Both rules are unit tested
(`governor.rs`), and the harness numbers above are unchanged by them because
its frames never load through the child.

## What is relative only

- Absolute frame times (45 to 70 ms steady) are llvmpipe drawing on the
  CPU at 13 to 14 fps, not a GPU at 60. The 60 fps pacing is requested but
  not reached on this host.
- Shader compile and link times are llvmpipe's. A real driver's cost, and
  therefore the stall frame, will differ in absolute terms.
- The ratio between fade-window and steady frames (the plan's 1.25x
  criterion) is the portable part, together with `load=` and the ordering
  of slow presets.
- Mesa's shader cache and the page cache make a second run of the same
  preset set much cheaper than the first. Always compare a candidate with a
  run made under the same cache state, ideally the second run in a row, or
  the difference will be mostly cache, not code.
- The WP4 pacing probe (under Xvfb) and scale probe (through EGL's
  surfaceless device) are llvmpipe too, and neither has a vblank, so the
  swap interval and the frame-rate lever cannot be judged here at all.
