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

## WP6: the recording fix, the harness on the app's path, and where a switch's time goes (RTX 3080 Laptop, 2026-11-03)

Branch `wp6-fix-stutter`. This is the first section with numbers from a real
GPU: Windows 11 on the laptop, NVIDIA GeForce RTX 3080 Laptop GPU, driver
617.14, 1920x1080 at 100%, a hidden 640x480 or 1920x1080 GL window, the
10,184 `.milk` files of the full Cream of the Crop folder in
`C:\Users\emilg\sfwp5\presets`. Build: MSVC 18, Ninja, release.

Exact commands (run through `cmd` after `call C:\Users\emilg\sfwp5\env.cmd`):

```sh
cargo build --release --example milkdrop_stutter
target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 200 --seconds 3 --path old
target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 200 --seconds 3 --path new
target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 200 --seconds 3 --path new --size 1920x1080
target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 40 --passes 2 --seconds 3 --path new
target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 40 --seconds 6 --path old
target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 40 --seconds 6 --path new
```

Each 200-preset run takes about 10 minutes here; 40 presets about 2 (3 s
each) or 4 (6 s each) minutes. Runs are one at a time, nothing else drawing.

### The harness now drives the app's frame rules

`--path new` (the default) calls `milkdrop::child::prepare_frame` (so it uses
`Presets::lookahead`, the prefetch worker, `Engine::load_data`, the
slow-preset list and `Presets::advance`), `milkdrop::policy::Policy` (the
same fade governor and switch-recording rule as the window child) and the
same 48x36 mesh, soft cut and 60 fps software clock. Its audio goes through
the real `shm::Ring` at the analyser's lag. `--path old` keeps the previous
behaviour (`Engine::load` on the frame, no prefetch, no policy) as the
baseline. `--passes N` repeats the sample, which is how the slow list is
shown working.

The sample is hardlinked (or copied, if the file system refuses a link) into
`%TEMP%\spotifast-milkdrop-sample-<pid>` for `Presets` to list, in the same
order, and removed when the run ends.

### The recording bug, and the two other review findings

Before this branch the child timed a load only when `SPOTIFAST_MILKDROP_TRACE`
was set. `record_switch` and the governor's `switched` flag were therefore
fed nothing in ordinary sessions: the slow list never learned, and the
governor judged the load frame as a fade frame. A load is now timed
unconditionally (two clock reads); the trace only decides whether it prints.
`policy::Policy::ended` is tested with the trace off, and an integration test
runs `prepare_frame` with a stub engine and asserts the load comes back
timed.

The other two findings, both in `governor.rs`/`policy.rs`:

- The governor's window held frames from outside a fade, so a slow preset
  playing before a fade could step the picture down on that fade's first
  frame. The window now holds only frames of the fade in progress and is
  reset when a fade begins.
- A switch drawn while the user's own scale is already 4 was never recorded,
  because the step down cannot shrink it further and `full_detail()` still
  said no. The rule is now "the drawn scale equals the user's scale", which
  is the switch a person actually sees.
- The trace keeps at most 1000 switches instead of the whole session.

Two more things changed while measuring, both in the governor:

- A frame's cost is now the gap between frame starts, not the work inside
  the frame. On this GPU `projectm_opengl_render_frame` returns long before
  the picture is ready, so the work-based number was blind to exactly the
  GPU-bound fades the governor exists for. Before the change, a 40-preset
  run at 6 s a preset had the governor step down for 44 of 5874 fade frames;
  after, 476 of 5807, and only where the frame really missed.
- A fade that begins before the previous fade's 2 s hold ends now starts at
  full detail and decides for itself. With preset turns shorter than the fade
  plus the hold (3 s is a supported setting) the picture used to stay at half
  size for good after one heavy fade.

The tolerance is also the plan's own: a frame counts as late only when it
runs more than a quarter over the budget. At 640x480 a 60 fps frame lands
within a millisecond of 16.7 ms; counting those made nearly every fade step
down.

### Before and after, 200 presets, 640x480, soft cuts

The old path was run twice to show the driver cache: the first run compiled
these 200 shader pairs for the first time on this install, the second did
not. The new path is compared with the warm old run. `p50`/`p99` in ms.

| metric | old, cold | old, warm | new, warm |
|---|---:|---:|---:|
| preset load p50 / p99 / max | 142.1 / 264.7 / 343.4 | 107.6 / 204.0 / 217.3 | 109.5 / 204.8 / 376.5 |
| first draw p50 / p99 / max | 5.1 / 24.4 / 29.7 | 4.4 / 23.5 / 29.8 | 4.8 / 24.4 / 31.8 |
| switch cost p50 / p99 / max | 147.3 / 281.1 / 346.2 | 113.4 / 205.8 / 223.6 | 115.6 / 210.1 / 385.4 |
| worst frame per switch p50 / p99 | 162.6 / 293.9 | 127.0 / 228.1 | 132.0 / 244.2 |
| worst frame in the 1 s after, p99 | 293.9 | 228.1 | 225.5 |
| loads over 8 ms | 179 of 200 | 165 of 200 | 165 of 200 |
| switches over 50 ms | 165 of 200 | 161 of 200 | 163 of 200 |
| fade frame p50 / p99 | 16.7 / 39.1 | 16.7 / 35.1 | 16.7 / 36.7 |
| steady frame p50 / p99 | 16.7 / 24.4 | 16.6 / 18.2 | 16.7 / 19.1 |
| fade p99 / steady p99 | 1.61x | 1.93x | 1.92x |
| achieved frame rate | 54.9 fps | 56.7 fps | 56.4 fps |

Read the two warm columns against each other: the new path does not make the
switch cheaper, because the switch is projectM's CPU transpile (below), and
the prefetch only removes the file read. The 20 to 30% between the cold and
warm old runs is the NVIDIA shader cache: the second compile of the same
GLSL is nearly free, the first is 15 to 80 ms per program.

### 1920x1080

`--path new --size 1920x1080`, same 200 presets:

| metric | 640x480 | 1920x1080 |
|---|---:|---:|
| preset load p50 / p99 | 109.5 / 204.8 | 115.8 / 209.1 |
| first draw p50 / p99 | 4.8 / 24.4 | 4.6 / 20.0 |
| switch cost p50 / p99 | 115.6 / 210.1 | 119.8 / 216.4 |
| fade frame p50 / p99 | 16.7 / 36.7 | 16.7 / 35.5 |
| steady frame p50 / p99 | 16.7 / 19.1 | 16.7 / 18.6 |
| achieved frame rate | 56.4 fps | 56.4 fps |

The switch is the same at both sizes (it is the transpile, not the pixels).
Steady frames still hold 60 fps at 1080p on this GPU, which is why the
governor has so little to do here.

### Where the load goes: the per-phase numbers

These come from a **local, instrumented copy** of the pinned libprojectM
(`crmne/projectm-rs` 93852ff, whose `libprojectM` is `crmne/projectm` ffb1098
on the `spotifast` branch), built with a `[patch]` that is **not on the
branch** and not proposed for merging. Timers (a `steady_clock` read either
side of each step, printed when `SPOTIFAST_PM_PHASES` is set) were added to
`MilkdropPreset::Initialize`, `MilkdropShader::LoadTexturesAndCompile` and
`TranspileHLSLShader`, `Renderer::Shader::CompileProgram`, and the four
passes of `HLSLParser::ApplyPreprocessor` in the vendored hlslparser. The
harness's own `load=` for the same preset agrees with the instrumented
numbers to within a millisecond, so the instrumentation does not distort
them.

One preset with both shaders (`smooth abruptness.milk`), the same preset in
two processes, first compile and then a warm cache, in ms:

| phase | first load | second load |
|---|---:|---:|
| read + parse the `.milk` (text) | 1.6 | 1.0 |
| blur texture and generic shaders | 0.0 | 0.0 |
| ns-eel: per-frame/per-vertex/custom wave and shape code | 0.7 | 0.2 |
| framebuffer sizes | 0.4 | 0.5 |
| **warp shader**: HLSL preprocessor | 45.8 | 41.7 |
| warp: HLSL parse | 15.2 | 14.1 |
| warp: GLSL generate | 0.6 | 0.4 |
| warp: **driver compile + link** | **65.3** | **1.4** |
| **composite shader**: HLSL preprocessor | 55.6 | 41.3 |
| composite: HLSL parse | 17.4 | 14.7 |
| composite: GLSL generate | 0.7 | 0.4 |
| composite: **driver compile + link** | **74.4** | **1.3** |
| textures (`samples=`) | 0.0 | 0.0 |
| **whole load (`Engine::load`)** | **281.9** | **122.9** |

Across five presets with shaders: the preprocessor is 30 to 56 ms per shader
on 5.6 to 7.0 KB of input (424 to 987 tokens, 79 to 80 macros), the HLSL
parser 5.5 to 20.6 ms, the generator 0.2 to 0.7 ms, and the driver 15 to 80
ms cold or 0.7 to 1.8 ms warm per program. Presets without a warp or
composite shader skip all of it and load in 1.3 to 5 ms. Inside the
preprocessor, passes 1 (collect `#define`s) and 4 (replace identifiers) are
20 to 40 ms each; passes 2 and 3 are a few ms. That is about 50 microseconds
per token in vendored code, and it is the one number in this section worth
taking upstream as a bug of its own.

The consequence for the plan is blunt: **the floor of a switch on this GPU is
the CPU transpile, ~85 to 135 ms warm, before the driver is asked anything**.
A warm driver cache takes the compile from ~140 ms to ~3 ms and cannot touch
the rest. Moving the transpile off the render thread needs projectM to expose
a parse/compile step, or the second instance and texture handover in
`DESIGN-offthread-compile.md`; nothing in this repository can do it through
`projectm_load_preset_data`.

### The governor on this GPU

`--count 40 --seconds 6`, the same 40 presets, both paths, so the 2 s hold
fits inside the turn:

| metric | old | new |
|---|---:|---:|
| switch p50 / p99 | 109.2 / 226.9 | 122.9 / 267.1 |
| fade p50 / p99 / max | 16.7 / 36.5 / 75.2 | 16.7 / 38.5 / 249.2 |
| steady p50 / p99 | 16.7 / 18.9 | 16.7 / 20.1 |
| governor step-downs | n/a | 476 of 5807 fade frames |

The governor now sees the heavy fades and steps down (4.6% of fade frames in
the final 200-preset run, i.e. only the heavy ones; the rest are left alone).
The heavy fades stay 35 to 50 ms at a quarter of the pixels: **the lever does
not reduce a heavy fade's cost on this GPU**. The final 1920x1080 run had it
down for 1,063 of 30,207 fade frames, and the fade still cost p99 35.5 ms. The
cost is per-frame CPU/geometry work inside projectM (the per-vertex equations
on the 48x36 mesh, the wave and shape geometry, the eel code), not fill, and
`Engine::render`'s scale only divides pixels. The old test showed the lever
helping llvmpipe at 1080p; on this GPU it does not help at all, and the only
thing it changes is sharpness. Keeping it costs a coarser picture during
heavy fades and buys nothing measurable.

### The slow list, now that it records

`--count 40 --passes 2 --seconds 3 --path new` (80 switches, 4 minutes).
The two passes are in one process and one cache state, so the difference is
the list, not the driver:

| metric | pass 1 | pass 2 |
|---|---:|---:|
| load p50 / p99 | 117.4 / 247.5 | 2.5 / 129.2 |
| loads over 8 ms | 32 of 40 | 3 of 40 |
| loads over 50 ms | 31 of 40 | 3 of 40 |
| whole run, switch p50 / p99 | 11.2 / 212.0 | |
| whole run, worst frame per switch p50 / p99 | 34.7 / 225.8 | |
| whole run, frame rate | 57.1 fps | |

The p99 of the whole run is pass 1, where every slow preset is still met for
the first time. The medians are the list doing its job: after a preset has
been measured once, automatic play passes over it, and with 40 presets it
settles on the eight whose switches are 1.6 to 4.9 ms, cycling them at
57 fps. Three slow switches still slipped through in pass 2, because a switch
drawn at reduced detail is deliberately not recorded (it is not the switch a
person sees), so those presets get another chance and are learned then. On a
10,000-preset folder the list must still meet each preset once, so the hitch
is delayed, not removed, unless the list is fed by measurement ahead of time.

### Against the plan's success criteria on this GPU

| criterion | result |
|---|---|
| worst frame at a switch p99 <= 25 ms (200 presets) | **not met**: 244 ms (new, 640x480), 249 ms (1080p); warm floor ~200 ms |
| fade p99 <= 1.25x steady p99 | **not met**: 1.92x (640x480) and 1.91x (1080p); the governor steps down but its lever does not remove the heavy-fade cost |
| >= 58 fps average | **not met overall** (56.4); **met on what the slow list plays**: pass 2 of the 40-preset run has p50 loads of 2.5 ms and 57.1 fps over the whole run |
| no load blocks the render thread > 8 ms | **not met**: 165 of 200 loads are over 8 ms, p50 110 ms. The cost is inside projectM and cannot be moved off the thread through its API |
| fade runs on the wall clock | met (WP4a, unchanged) |
| tests, clippy, fmt | met (see the integration summary) |

The best achievable result with the current engine is the two-pass row: with
the slow list primed, automatic play switches in a median of 11 ms and holds
57 fps, and the remaining hitches are first encounters. Removing them needs
the second projectM instance, or a projectM change; both are outside this
branch.

## WP7: the HLSL transpile cache (RTX 3080 Laptop, 2026-11-04)

Branch `wp7-transpile-cache`. The transpile is now cached on disk, so a preset
that has been seen before skips libprojectM's HLSL preprocessor and parser.
`TRANSPILE-CACHE.md` is the full write-up: the cache key, the storage, the
configuration and the per-phase numbers. `PROJECTM-FORK.md` records the fork
chain that carries the change, which is the only way to get it without a
`projectM` release.

Where the time went: the warm load's 85 to 135 ms was 30 to 56 ms of HLSL
preprocessing and 5 to 20 ms of HLSL parsing *per shader*, while the driver
compiled the result in 0.7 to 2.7 ms. The generated GLSL is a pure function of
the preprocessed source and a handful of declarations, so it is stored per
shader and reused; a hit reads the file and goes straight to
`glCompileShader`.

Same protocol as WP6: 200 presets, 3 s each, 640x480, `--path new`, five runs
one after another, nothing else drawing.

| metric, 200 presets | cache off | cold | warm | warm again |
|---|---:|---:|---:|---:|
| preset load p50 / p99 / max (ms) | 127.6 / 402.7 / 849.1 | 115.6 / 254.5 / 361.5 | **5.1 / 111.6 / 155.7** | **5.2 / 120.4 / 319.7** |
| loads over 8 ms | 170 | 160 | 54 | 47 |
| first draw p50 / p99 (ms) | 5.2 / 37.7 | 5.5 / 29.1 | 4.8 / 31.9 | 4.9 / 34.5 |
| switch cost p50 / p99 (ms) | 135.1 / 427.8 | 122.1 / 276.6 | **11.3 / 115.7** | **11.0 / 126.8** |
| worst frame per switch p50 / p99 (ms) | 151.4 / 438.9 | 137.9 / 318.0 | **26.5 / 159.0** | **25.8 / 162.9** |
| fade frame p50 / p99 (ms) | 16.7 / 42.9 | 16.7 / 43.8 | 16.7 / 39.6 | 16.7 / 40.0 |
| steady frame p50 / p99 (ms) | 16.6 / 23.4 | 16.7 / 33.6 | 16.7 / 21.6 | 16.7 / 29.9 |
| fade p99 / steady p99 | 1.83x | 1.30x | 1.83x | 1.34x |
| frame rate | 53.3 fps | 53.9 fps | **57.1 fps** | **56.9 fps** |
| all 200 loads together | 26.5 s | 21.2 s | **2.35 s** | **2.56 s** |
| hit rate | - | 15.0% | 98.1% | 98.1% |

Per shader, median: miss 65.0 ms (preprocess 49.4, parse 13.8, generate 0.5,
strip 0.02) against a hit's 0.34 ms read and 0.8 ms driver compile. Over a run,
265 transpiles of 18.6 s became 313 reads of 0.31 s. 200 presets take 267
entries, 1.8 MB, 6.8 KB each. `PROJECTM_SHADER_CACHE_VERIFY=1` re-transpiled
all 313 hits and found 0 mismatches across processes.

Run to run, the warm load median was 5.1 to 7.2 ms, and one run taken right
after the cache directory had been rewritten read its entries at 6.6 ms each
instead of 0.3 and showed a 19.7 ms median: that is the file system, not the
code, and `TRANSPILE-CACHE.md` has the four runs side by side. The p99 does
not depend on the disk: it is the six shaders the driver rejects, below.

Against the plan's criteria on this GPU:

| criterion | result |
|---|---|
| worst frame at a switch p99 <= 25 ms | **not met**: 159.0 ms (warm), p50 26.5 ms is at the target. The harness counts the frame's wait for its own deadline (16.7 ms), and 10 of 200 presets cost 50 to 156 ms for reasons below |
| fade p99 <= 1.25x steady p99 | **not met**: 1.34x to 1.83x, unchanged by the cache: a fade draws two presets |
| >= 58 fps average | **not met**: 57.1 fps, from 53.3 |
| no load over 8 ms | **not met**: 54 of 200, p50 5.1 ms. 146 loads are under 8 ms, against 30 before |
| load p50 <= 8 ms | **met**: 5.1 ms |
| preset switch no longer the hitch | **met**: p50 135 -> 11 ms, p99 428 -> 116 ms |

What is left is not the transpile:

- 6 of the 265 distinct shader sources transpile and then fail to compile in
  the driver. projectM logs an error and drops the shader, and because only
  successful transpiles are cached they are transpiled again on every load:
  about 15 ms of transpile plus a failed driver compile of 30 to 60 ms each
  time. That is the difference between a 3 ms load and a 100 ms one in the
  10-preset sample in `TRANSPILE-CACHE.md`.
- The other 194 presets load in a median of 5.1 ms, which is the preset's own
  setup (text, expressions, textures, framebuffers, measured at 0.0 to 3 ms
  per step) plus GL calls that wait when the queue is full.
- The fade ratio is fill and geometry.

The cache is on by default, uses `%LOCALAPPDATA%\projectm\shader-cache`
(`$XDG_CACHE_HOME` or `~/.cache` elsewhere), and needs no Spotifast change.
`PROJECTM_SHADER_CACHE=0` turns it off, `PROJECTM_SHADER_CACHE_DIR` moves it,
`PROJECTM_SHADER_CACHE_VERIFY=1` and `PROJECTM_SHADER_CACHE_DEBUG=1` are for
testing it.

## WP8: the cache review fixes and the rest of a switch (RTX 3080 Laptop, 2026-11-04)

Branch `wp8-stutter-fixes`, `projectm-sys` pinned to Errantial/projectm-rs
`ea2d84e` (libprojectM Errantial/projectm `d3d81668e`). `TRANSPILE-CACHE.md`
is the full write-up, with the protocol, the per-phase split, the scaling
test and the robustness runs; `PROJECTM-FORK.md` lists the fork commits.

What changed:

- **The cache's disk work left the render thread.** One worker thread per
  process writes entries (temporary file and rename), reads each back once,
  refreshes the mtime of entries that were hit, scans at start and prunes.
  The render thread reads one file per shader and queues the rest.
- **A 256 MiB byte budget, least recently used first**, instead of 4096
  entries pruned by write time. The full 10,184-preset library takes 5,104
  entries and 35.1 MB (6,886 bytes an entry), so the budget holds about
  39,000 entries, 7.6 times the library. A second pass over the whole library
  in a new process transpiled nothing.
- **Reject entries**: a shader this driver refused (6 in the 200-preset
  sample, 179 of 5,104 distinct shaders in the library) is skipped straight
  to the usual fallback next time, keyed on the GL vendor, renderer and
  version strings. Those six presets load in 3.4 to 7.7 ms instead of 160 to
  218 ms.
- **The antivirus scan of a new entry** (about 7 ms on the first open of a
  new file here) is now paid by the worker's read-back, not by the render
  thread's first hit. Cache reads went from p99 24 ms to 0.9 ms per load.
- **Texture folder rescans** moved to the background. libprojectM threw its
  texture file list away at every preset load and rescanned every search
  path, including the 10,184-file preset folder Spotifast passes, on the
  render thread: p99 20 ms per load. Now one scan per process, then
  background scans between loads.
- **The governor** no longer counts the frame after a switch as a late fade
  frame (`src/milkdrop/policy.rs`, two new tests).

Same protocol as WP6 and WP7, 200 presets, 3 s each, 640x480, `--path new`,
each run after the machine's CPU stayed under 20% (13 to 16% measured):

| metric, 200 presets | off | cold | warm | warm again | WP7 warm |
|---|---:|---:|---:|---:|---:|
| preset load p50 / p99 / max (ms) | 138.1 / 226.9 / 280.8 | 118.3 / 238.5 / 262.4 | **4.9 / 17.6 / 35.6** | **4.9 / 22.4 / 37.7** | 5.1 / 111.6 / 155.7 |
| loads over 8 ms / over 25 ms | 165 / 165 | 158 / 158 | **11 / 2** | **14 / 2** | 54 / 16 |
| switch cost p50 / p99 (ms) | 147.1 / 240.2 | 123.6 / 243.6 | **10.0 / 40.9** | **9.9 / 40.3** | 11.3 / 115.7 |
| switches over 50 ms (slow list) | 165 | 158 | **1** | **1** | about 10 |
| switch frame p50 / p99 (ms) | 160.1 / 255.2 | 138.8 / 260.1 | 24.6 / 55.9 | 24.6 / 56.1 | 25.1 / 131.4 |
| worst frame per switch p50 / p99 (ms) | 160.1 / 283.2 | 141.6 / 260.1 | 25.0 / 63.2 | 25.0 / 70.2 | 26.5 / 159.0 |
| fade frame p50 / p99 (ms) | 16.7 / 35.5 | 16.7 / 35.5 | 16.7 / 35.3 | 16.7 / 35.1 | 16.7 / 39.6 |
| steady frame p50 / p99 (ms) | 16.6 / 18.2 | 16.7 / 18.2 | 16.6 / 18.1 | 16.7 / 18.5 | 16.7 / 21.6 |
| fade p99 / steady p99 | 1.95x | 1.94x | 1.95x | 1.90x | 1.83x |
| frame rate | 56.1 fps | 56.6 fps | **58.7 fps** | **58.7 fps** | 57.1 fps |
| all 200 loads together | 24.3 s | 21.0 s | **1.05 s** | **1.08 s** | 2.35 s |

Verify mode: 313 hits re-transpiled and 6 reject entries re-checked, 0 and 0
mismatches.

Where a warm load goes now (diagnostic timers, not committed): driver compile
and link 1.4 ms at the median and 3.7 at the p99 for both programs, ns-eel
compile and init code 0.6 and 11.3 ms (27.6 ms at worst), cache reads 0.5 and
0.9 ms, textures 0 apart from one 14.7 ms scan per process, no transpile.

The fade: during a fade the incoming and the outgoing preset's
`RenderFrame` cost what each costs on its own (the outgoing one 13.9 ms
against 14.0 before the fade, for example), and the transition draw 0.04 ms.
The fade's p99 of 35 ms is two heavy presets' CPU work added together; there
is no redundant work to remove without changing the picture, so WP8 does not
change it.

Against the criteria on this GPU:

| criterion | result |
|---|---|
| worst frame at a switch p99 <= 25 ms | **not met**: 63.2 / 70.2 ms over the whole turn, 55.9 / 56.1 ms for the switch frame itself; p50 25.0 ms. Bounded by the fade (two presets' CPU) and the first draw, which is a fade frame |
| fade p99 <= 1.25x steady p99 | **not met**: 1.90x to 1.95x; the transition costs 0.04 ms, the rest is two presets' per-frame CPU |
| >= 58 fps | **met**: 58.7 fps in both warm runs (WP7 57.1) |
| no load blocking the render thread > 8 ms | **not met**: 11 and 14 of 200 (WP7 54 and 47), p99 17.6 / 22.4 ms, all ns-eel compile, driver compile, the first texture scan and the initial-image draw; none is cache work |
| no render-thread prune stalls | **met**: no scan or prune runs there; worst render-thread cache read 0.80 ms while the worker pruned 1,488 entries in 14 scans |
| verify, 0 mismatches | **met** |

The slow list now excludes 1 of 200 presets with a warm cache. With a cold
cache it still learns from the uncached first switch: in a two-pass run, pass
1 put 32 of 40 presets on it, and pass 2 cycled 10 presets although every
load took under 7 ms. That is a WP6 policy question, left for the
maintainer (`TRANSPILE-CACHE.md`, "The slow list").

## WP9: expression compiling off the render thread, and the fade's shape draws (RTX 3080 Laptop, 2026-11-04)

Branch `wp9-fade-eel`, `projectm-sys` pinned to Errantial/projectm-rs
`08d2eb9` (libprojectM Errantial/projectm `5749d6a1d`, on `d3d81668e`).
`PROJECTM-FORK.md` lists the commits.

What changed:

- **A preset's expression code is compiled before the switch.**
  `projectm_prepare_preset_data()` parses the preset file and compiles its
  per-frame, per-pixel, custom wave and custom shape code on the prefetch
  thread (`src/milkdrop/prefetch.rs`, which already reads the next file
  while the current one plays). The render thread's load of the same bytes
  adopts the result. 197 of 197 loads in the instrumented run used one. A
  preset that does not parse is not kept and loads, and fails, as before.
- **The start of the per-frame init code runs there too.** WP8's biggest
  ns-eel cost on the render thread was not compiling but running init code:
  loops that clear `gmegabuf` a million entries at a time (27 ms at worst in
  the WP8 split). The leading top-level statements that name no built-in
  variable except q1 to q32 and do not call `rand()` run on the preparing
  thread, on the preset's own fresh memory. They see what they would see at
  the switch. The rest runs at the switch, in order, after the built-in
  variables were loaded. The split reads the code text (the build links
  vcpkg's projectm-eval 1.0.5, so the first version, which read the compiled
  tree, never ran anything ahead). 1,937 of the 10,184 presets have such
  statements, 278 of them loops. If either half does not compile alone, the
  whole block runs at the switch.
- **A custom shape's instances are uploaded once per frame.** Before, each
  of up to 1,024 instances did its own buffer upload and draw per frame. Now
  every instance's code runs first, all vertices go up in one upload per
  buffer, and each instance is drawn from its range with the same blend,
  shader, texture and attribute state as before.

Measured on the same protocol as WP8 (200 presets, 3 s each, 640x480, soft
cuts, `--path new`, warm cache in a new process, each run started after the
CPU stayed under 20% for 20 s). A new binary compiles shaders cold the first
time because the NVIDIA driver cache is per executable, so each binary had a
warm-up run first. Runs 3 (WP9) and 1 (WP8) were disturbed by something
else on the machine (53 and 38 loads over 8 ms, one load of 484 ms) and are
left out of the table; two more clean runs replaced them. Raw outputs:
`wp9/final/` on the laptop.

| metric, 200 presets | WP8 warm 2 | WP8 warm 3 | WP8 warm 4 | WP9 warm 1 | WP9 warm 2 | WP9 warm 4 |
|---|---:|---:|---:|---:|---:|---:|
| preset load p50 / p99 / max (ms) | 4.7 / 22.0 / 33.5 | 4.8 / 17.4 / 36.4 | 4.8 / 31.0 / 41.0 | 4.2 / 13.2 / 24.8 | 4.2 / 11.6 / 17.8 | 4.0 / 9.6 / 35.2 |
| loads over 8 ms / over 25 ms | 14 / 2 | 14 / 2 | 12 / 3 | **3 / 0** | **7 / 0** | **4 / 1** |
| switch cost p50 / p99 (ms) | 9.9 / 37.6 | 9.5 / 41.7 | 9.6 / 43.3 | 9.5 / 28.3 | 9.5 / 22.3 | 8.6 / 25.3 |
| switch frame p50 / p99 (ms) | 24.6 / 53.1 | 24.4 / 57.6 | 24.4 / 57.3 | 24.2 / 40.0 | 24.2 / 37.8 | 23.7 / 37.8 |
| worst frame in the 3 s p50 / p99 (ms) | 25.0 / 62.0 | 24.8 / 94.7 | 24.5 / 62.9 | 24.4 / 104.5 | 24.5 / 41.8 | 23.8 / 43.1 |
| fade frame p99 (ms) | 35.9 | 35.6 | 35.8 | 19.1 | 18.9 | 19.2 |
| steady frame p99 (ms) | 18.4 | 18.7 | 18.7 | 18.1 | 18.1 | 18.4 |
| fade p99 / steady p99 | 1.95x | 1.90x | 1.91x | **1.06x** | **1.04x** | **1.04x** |
| frame rate | 58.6 fps | 58.6 fps | 58.7 fps | **59.7 fps** | **59.8 fps** | **59.7 fps** |
| all 200 loads together | 1.06 s | 1.05 s | 1.07 s | 0.87 s | 0.84 s | 0.84 s |

The 104.5 ms worst frame in WP9 warm 1 is one 250 ms frame in the middle of
one preset's turn (292.milk, in its steady part), not a switch cost; WP8
warm 3 has the same kind of outlier (94.7 ms).

Cache off (every shader transpiled on the render thread, three runs each,
alternating, same day):

| metric | WP8 off 1 / 2 / 3 | WP9 off 1 / 2 / 3 |
|---|---|---|
| preset load p50 / p99 (ms) | 135.0 / 220.7, 130.6 / 227.9, 135.1 / 232.7 | 160.3 / 280.8, 138.1 / 222.5, 146.3 / 257.0 |
| switch frame p50 / p99 (ms) | 154.9 / 238.6, 150.9 / 244.6, 155.6 / 258.7 | 181.2 / 345.1, 158.3 / 242.2, 165.6 / 279.0 |
| fade p99 / steady p99 | 1.92x, 1.90x, 1.87x | 1.08x, 1.05x, 1.07x |
| frame rate | 56.3, 56.4, 56.3 fps | 56.8, 57.4, 57.3 fps |

With the cache off, WP9's loads are slower at the median (146.3 against 135.0
ms, the median of three runs, 8% more; the first run was 19% more). I did not
find the cause. The transpile dominates a cache-off load, nothing in WP9
touches it, and the default is the cache on, so this does not affect the
numbers above. It is an open question.

Verify mode (`PROJECTM_SHADER_CACHE_VERIFY=1`): 313 hits re-transpiled and 6
reject entries re-checked, 0 and 0 mismatches.

### Where the time goes (instrumented build, not committed)

Timers inside libprojectM, one instrumented run per build, warm cache, same
protocol (`wp9/final/diag.out` for WP9; the WP8 side is the earlier
instrumented WP8 run). The render thread's expression work on a load:

| per load, render thread (ms) | WP8 p50 / p99 / max | WP9 p50 / p99 / max |
|---|---:|---:|
| parse the preset file | 0.22 / 1.09 / 24.4 | 0.00 / 0.00 / 0.00 |
| ns-eel (rest of init, wave and shape init) | 0.54 / 29.07 / 76.3 | 0.00 / 0.43 / 1.32 |
| init code run (per-frame init, gmegabuf loops) | 0.00 / 10.4 / 27.1 | 0.00 / 0.00 / 0.00 |
| all of the load | 4.91 / 37.72 / 153.96 | 4.27 / 9.18 / 20.57 |

The same work on the prefetch thread, per preset (parse, compile and the
independent init statements): p50 1.68, p99 17.8, max 37.2 ms. It finishes
well inside the 3 s a preset plays.

What is left in a load is GL work: in the 6 loads over 8 ms of that run the
warp and composite program compile and link took 0.8 to 18.2 ms (one preset
spent 18.2 ms in the warp program's first compile in the process), the
framebuffers 0.5 ms and the initial image 0.4 to 3.3 ms.

A frame, CPU time in `RenderFrame` (ms):

| | WP8 p50 / p99 | WP9 p50 / p99 |
|---|---:|---:|
| steady frame | 1.43 / 15.16 | 1.58 / 9.17 |
| fade frame (both presets) | 3.07 / 19.84 | 3.31 / 13.19 |
| the transition draw | 0.04 / 0.12 | 0.04 / 0.27 |
| custom shape draw, one preset, p99 | 13.3 | 7.0 |

The slowest 1% of fade frames average 34.0 ms in the WP8 split (25.2 of it
in shape GL calls) and 14.6 ms in WP9 (shape GL calls 7.9, shape code 1.8,
per-frame code 1.0, per-vertex code 0.8, composite 0.6, blur 0.3, warp draw
0.3). During a fade the GPU takes 2.7 ms at the median and 10.1 at the p99
(steady: 1.2 and 5.9), so the GPU is not what limits a fade frame. WP9's
`waveform` timer (the main waveform draw) reads p99 2.26 ms against 0.09 in
WP8. The driver most likely charges queued work to whichever GL call comes
next; the frame total is lower in WP9, but I did not isolate it.

### Same output, checked

`examples/milkdrop_evalcheck.rs` draws a sample of presets with hard cuts,
a fixed frame clock and the same synthetic audio every run, and a build with
the hue offsets and random textures pinned. Each preset is prepared on a
spawned thread first, as the prefetch does. Old (WP8 plus the same hooks)
against new (WP9), 90 frames each:

- 60 presets sampled by stride from the 10,184 and 40 presets chosen for
  init loops that now run ahead: every per-frame variable (the q variables,
  built-in outputs, registers), every warp mesh vertex, every waveform point
  and every shape instance's vertices and colours are identical, run for run
  (3 old and 3 new runs on the first set, 2 and 2 on the second).
- Pixels: a preset draws slightly differently from run to run in WP8 too
  (undefined texture contents and similar), so a pixel hash only means
  something where WP8 repeats itself. 19 of the 60 and 15 of the 40 presets
  are pixel-stable across WP8 runs, and all 34 are identical in WP9. They
  include textured shapes, bordered shapes and shapes with 512 and 1,024
  instances. The shipped build without the hooks was also run once; it
  differs from the old build on 15 of the 19 stable presets. It has no
  pinned seeds, which would explain that, but I did not run it twice to
  confirm, so the comparison above is the pinned one only.
- A bug found on the way: a custom waveform whose `sep` is larger than the
  samples it leaves reads before the audio arrays (an existing out-of-bounds
  read in libprojectM, onto `frameQVariables` and `globalRegisters`).
  Moving the registers out of `PresetState` changed what such a wave drew
  (`shiter redi jedi- agar awkward seances qaent possibly mbattre.milk`).
  `PresetState` now keeps its original layout, and a prepared preset's
  registers are mirrored into it before waveform code reads audio.
- `InitCodeSplitTest.cpp`, 7 tests, passes against the projectm-eval 1.0.5
  sources (built and run on its own; `projectm-sys` builds without tests).

### Against the criteria (warm cache)

| criterion | result |
|---|---|
| worst frame at a switch p99 <= 25 ms | **not met**: the switch frame is 37.8 to 40.0 ms at the p99 and 23.7 to 24.2 at the p50 (WP8: 53.1 to 57.6 and 24.4 to 24.6); the worst frame in the 3 s is 41.8 to 104.5 ms at the p99 (WP8: 62.0 to 94.7). The frame is about 15 ms of frame pacing plus the load (p99 9.6 to 13.2 ms) plus the first draw, which is a fade frame of two presets with new programs (p50 5.3 ms, p99 17.1 in one run) |
| fade p99 <= 1.25x steady p99 | **met**: 1.04x to 1.06x (WP8 1.90x to 1.95x) |
| >= 58 fps | **met**: 59.7 to 59.8 fps (WP8 58.6 to 58.7) |
| no load blocking the render thread > 8 ms | **not met**: 3, 7 and 4 of 200 (WP8 14, 14, 12). ns-eel is no longer in it (p99 0.43 ms, max 1.32); what is left is the driver's compile of the warp and composite programs and the initial image |
| verify, 0 mismatches | **met** |

### Left, and what it would take

- The load over 8 ms is now the driver compiling GLSL, which libprojectM
  cannot do ahead of time on another thread without a shared GL context.
  Compiling on a second context is possible and would change how projectM
  owns its GL objects; I did not start it.
- The first draw of a fade is two presets' per-frame work. Shape GL calls
  are still the largest part (7.9 of 14.6 ms in the slowest 1%). Drawing a
  shape's instances with one multi-draw call where their state is equal
  would not change the picture; I did not do it.
- These would shorten a fade or a switch but show something different, so I
  did not do them: draw the outgoing preset every second frame during a
  fade (its `RenderFrame` is 8.9 ms of the 13.2 ms p99 fade frame; I did not
  measure the saving), skip the outgoing preset's custom shapes (shape GL is
  about half of the slowest fade frames), or render a fade at a lower
  resolution. The governor already does a version of the last one.
- Not compiled or run: macOS and Linux. The libprojectM changes use only the
  standard library (`std::mutex`, `std::deque`, `std::unordered_set`) and
  the Rust change has no `cfg`, but only Windows was built.
- Risks: the compile on another thread relies on projectm-eval's parser being
  re-entrant. That holds for 1.0.5, which this build links (I read its
  scanner and parser); a build against the older vendored projectm-eval was
  not checked. The init statements that run ahead exclude `rand()` because
  its generator state is global, and nothing else in projectm-eval is
  non-deterministic. The layout-keeping mirror exists only for the
  out-of-bounds read above.
- A person should look at: custom-shape presets during a fade, next to a WP8
  build (their instances now draw from one upload; the check says the same
  vertices and the same pixels where WP8 repeats itself), and a preset with
  a large `sep` custom waveform.
