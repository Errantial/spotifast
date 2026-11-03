# MilkDrop performance notes

Built and run through `sfb` on the 24-core Linux build host. All OpenGL work
is llvmpipe: the pacing probe under Xvfb, the scale probe through EGL's
surfaceless device. Timings are relative, not real-GPU numbers.

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

