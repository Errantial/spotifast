# MilkDrop preset transitions: integration summary (WP5)

Branch `wp5-integration`, merged from `wp4-crossfade` on top of WP1 to WP3.
All numbers in this file and in `PERF_NOTES.md` come from Mesa llvmpipe on the
shared Linux build host (Xvfb, or EGL's surfaceless device for the scale
probe). They are relative, never real-GPU numbers.

## What is on the branch

| package | commits | what it adds |
|---|---|---|
| WP1 measure | `65377de` `418a049` `47f3725` | `examples/milkdrop_stutter.rs` (load, first draw, stall, fade and steady frame times over N presets), `SPOTIFAST_MILKDROP_TRACE` for the child process, the baseline numbers in `PERF_NOTES.md`. |
| WP2 prefetch | `9d68a91` `0926ffc` | `Presets::lookahead` picks the next preset one turn early, `src/milkdrop/prefetch.rs` reads its file on a worker thread, `Engine::load_data` parses the bytes on the render thread instead of reading the file there. |
| WP3 switch budget | `4113f89` `91c2a44` `a225746` `ac4936d` | The switch is split into load and first draw in the trace and harness; `src/milkdrop/budget.rs` keeps a per-window list of presets whose switch cost more than 50 ms, and automatic play passes them over (keys and clicks still reach them); `DESIGN-offthread-compile.md` is the research note for a second projectM instance on a shared context. |
| WP4 fade detail and pacing | `fe14506` `ff6b768` `f5fad8f` | `Presets::fading` (the soft cut's wall-clock window), `src/milkdrop/governor.rs` (5 of the last 8 fade frames over budget steps the render scale to two for the rest of the fade, held 2 s after it), `examples/milkdrop_scale_probe.rs` and `examples/milkdrop_pacing.rs` with their measurements. No pacing code changed. |
| WP5 integration | `e9e1e8b` (merge) `e20ba2b` (fix) | The merge itself, the two-policy fix below, the integrated measurement and this summary. |

No new user setting and no interface change. The only user-visible changes
are the ones WP3 and WP4 already had: a preset whose first frame takes more
than 50 ms is passed over when presets change on their own, and a crossfade
on a machine that cannot hold the frame rate is drawn at half size for the
rest of the fade.

## The merge

Conflicts were in `Child::render` (WP4 rewrote it around the governor while
WP3 had added the switch timing), in `Presets` (both sides added methods and
fields), and in `PERF_NOTES.md` (two sets of notes). Resolution keeps every
work package: the child draws at the governed scale and still records each
switch for the trace and the budget, `Presets` keeps the lookahead pick and
the slow list beside the fade clock, and the notes keep one section per
package.

### Where WP3 and WP4 could fight, and what changed

Both judge a frame, so each could be fed the other's frame:

1. A switch drawn while the governor's step down was held costs about a
   quarter of its real first draw. WP3's budget would read that as a fast
   switch and trust a preset that is slow at full detail. `Child::render` now
   records a switch cost only while the governor is at full detail
   (`src/milkdrop/child.rs:421`).
2. The long frame that loads a preset is the switch's cost, not what a fade
   costs, but WP4's governor read every frame of a fade, including that one.
   Pressing Next through a fade could therefore shrink the picture for a fade
   that was keeping up. `Governor::observe` now ignores a frame that loaded a
   preset (`src/milkdrop/governor.rs:94`).

Both rules have unit tests in `governor.rs`. Neither changes what a switch
costs, so the harness numbers below are the same as before the fix.

## Measured effect (llvmpipe, relative only)

### The switch, same command and preset sample as WP1 and WP3

`sfb gl cargo run --release --example milkdrop_stutter -- --presets .perf-presets --count 200 --seconds 3`,
two runs back to back, the second (warm) one in the table. The preset folder
was rebuilt to the recipe in `PERF_NOTES.md` from the two packs (552 files of
MilkDrop's own plus every `i*9795/200`-th of Cream of the Crop, 749 files after
three name clashes). The build host was busier than on the WP3 day, so the
first run is only quoted in `PERF_NOTES.md` as noise.

| metric, warm, 200 presets | WP1 baseline | WP3 after | integrated |
|---|---:|---:|---:|
| load p50 / p99 / max (ms) | 10.2 / 24.8 / 26.4 | 9.4 / 26.3 / 29.2 | 11.2 / 28.4 / 35.6 |
| first draw p50 / p99 / max (ms) | not measured | 67.0 / 164.2 / 236.1 | 68.8 / 213.7 / 218.5 |
| switch p50 / p99 / max (ms) | not measured | 75.4 / 183.8 / 251.1 | 81.5 / 230.8 / 239.0 |
| stall frame p50 / p99 (ms) | 92.0 / 256.2 | 94.8 / 204.0 | 101.2 / 234.8 |
| worst frame per switch p50 / p99 / max (ms) | 97.8 / 267.9 / 345.7 | 98.6 / 204.0 / 271.1 | 103.3 / 234.8 / 259.4 |
| worst frame in the 1 s after, p99 (ms) | 256.2 | 239.4 | 234.8 |
| frame ms during fade, p50 / p99 | 71.1 / 136.8 | 76.1 / 142.7 | 76.1 / 134.4 |
| frame ms steady, p50 / p99 | 47.5 / 83.0 | 50.7 / 80.1 | 49.6 / 78.4 |
| fade p99 / steady p99 | 1.65x | 1.78x | 1.71x |
| achieved frame rate | 14.0 fps | 13.1 fps | 13.0 fps |
| loads over 8 ms | 107 of 200 | 101 of 200 | 112 of 200 |
| switches over 50 ms | not counted | 185 of 200 | 190 of 200 |

Read this as "nothing regressed", not as a gain. The harness drives `Engine`
directly, so the prefetch worker, the slow list and the governor are not on
its path at all. The medians agree with the WP3 run within 10%, and the
switch tail is worse in both of today's runs, which is host load: that path
(the engine, the crossfade constant) is unchanged by the merge.

### The governor's lever, same tree, same day

`sfb gl cargo run --release --example milkdrop_scale_probe -- .perf-presets <size>`,
12 presets, 8 frames per fade, the fade drawn at full size and then at half:

| window | steady | fade, full | fade, half | half / full |
|---|---:|---:|---:|---:|
| 640x480 | 27.4 ms | 56.7 ms | 50.9 ms | 90% |
| 1920x1080 | 42.3 ms | 90.8 ms | 59.8 ms | 66% |

WP4's own run of the same probe read 83% and 55%; the two runs disagree where
the measurement is weakest (640x480, where llvmpipe's per-pass overhead is
nearly resolution independent) and agree in direction. The lever quarters the
pixels the two presets and the transition shader touch; how much of a fade
frame that removes depends on the GPU.

## Against the plan's success criteria

State plainly, because the branch does not meet two of them on this host and
nothing here can show that a real GPU meets them:

1. **Worst frame within 1 s of a switch <= 25 ms, fade p99 <= 1.25x steady
   p99.** Not met and not measurable here: llvmpipe draws at 13 fps, so every
   frame is 40 to 80 ms. The fade ratio is 1.71x to 2.28x depending on the
   run, which is llvmpipe's double rendering, not a code property. On a real
   GPU, WP3's budget is the answer to the first half (a preset that cannot
   hide a switch is not chosen automatically) and WP4's step down to the
   second, but neither is verified on hardware.
2. **No preset load blocks the render thread longer than 8 ms.** Not met.
   112 of 200 loads are over 8 ms (p99 28.4 ms) and the first draw on the
   render thread is 68.8 ms at the median. The read is off the thread (WP2);
   the parse, the HLSL-to-GLSL translation and the driver's compile are not.
   Moving the compile needs the second projectM instance in
   `DESIGN-offthread-compile.md`, which is a design note, not code.
3. **The crossfade runs on the wall clock.** WP4a confirmed it from the
   pinned libprojectM source: `PresetTransition` and `TimeKeeper` use
   `high_resolution_clock`, and `projectm_set_fps` only feeds a preset's
   `fps` variable. Our frame limit cannot stretch or freeze a fade.
4. **Existing checks pass.** `cargo fmt --check`, `cargo clippy --lib -- -D
   warnings`, `cargo check --lib` and `cargo test --lib milkdrop` (85 passed)
   pass. Windows and macOS were not compiled here.

## What did not help

- **The slow list does not make a switch cheaper.** 190 of 200 switches are
  still over its 50 ms budget. It changes which preset automatic play picks,
  so the hitches a person sees on autoplay become rarer, not smaller. A key
  press still pays the full cost.
- **The prefetch does not show in the load number yet.** Warm loads were
  already about 10 ms; the read was never the expensive half. It removes the
  read from the frame, which is worth more on a cold page cache or a slow
  disk than on this host.
- **The half-size step is small at 640x480** (90% of the fade frame) and
  meaningful at 1080p (66%), because llvmpipe's per-pass overhead does not
  scale with resolution. A fill-bound GPU should sit near the 1080p figure.
- **Pacing was not changed.** The timer is accurate to a fraction of a
  millisecond; the display half needs a screen.
- **The fade length was not changed.** No run showed a different 2.7 s would
  look smoother.

## Known risks

- The 25 ms criterion is not demonstrated anywhere in this branch. The budget
  (50 ms) and the governor's trigger (5 of 8 frames over the frame limit) are
  policy choices from llvmpipe numbers, not from a GPU.
- The governor decides from the work inside a frame, not the gap between
  frames. On a GPU that drops frames without ever blocking in the driver the
  fade looks cheaper than it is, and on one that blocks in `swap_buffers` the
  switch frame can be counted at reduced detail. WP4b documents both; the
  interaction fix covers the second.
- The pane (skin) rendering path has the budget but not the governor:
  `src/milkdrop/offscreen.rs:144` records switch costs, nothing scales a fade
  down there.
- A preset first switched while a step down is held is not added to the slow
  list, by design of the fix, so the list learns that preset later or never.
- `Presets::lookahead` does not know the slow list, so near the start of a
  session automatic play may skip the preset whose file was read ahead and
  load the one it plays from disk (`src/milkdrop.rs:266`). A wasted read, not
  a wrong picture.
- The child's `Presets::request` stamps `started` before the synchronous load,
  so a load eats into the 2.7 s fade and into the governor's window
  (`src/milkdrop.rs:436`). Pre-existing.
- Absolute numbers here describe a 24-core Xeon drawing with llvmpipe at 13
  fps. Driver shader caches (Mesa on disk, Windows and macOS driver caches)
  decide the real switch cost.

## What a human must verify on a real GPU

Windows 11 (a 60 Hz desktop and a laptop, ideally a hybrid one) and macOS
(Apple silicon and, if available, an Intel Mac with graphics switching):

1. **Does a switch still hitch?** Set `SPOTIFAST_MILKDROP_TRACE=1` in the
   child's environment, open MilkDrop, let it run 20 or 30 preset changes and
   read the `milkdrop trace: summary` line on stderr: `switch_ms` should be a
   few milliseconds, and `first_draw_ms` should be small after the first time
   each preset plays. A preset still over ~25 ms after its first play is what
   the plan's criterion is about.
2. **Do the safe presets look right?** Automatic play should now step through
   presets smoothly; N and P must still show every preset, including the ones
   it skips.
3. **Does the step down show?** Force it by choosing a preset pair that
   overruns, or by lowering the frame limit: during a fade the picture should
   coarsen once and come back about two seconds after the fade, with no
   pumping. Check it does not trigger on a machine that holds the frame rate.
4. **Is the fade smooth?** Watch a 2.7 s crossfade at 60 Hz and at the
   screen's own rate; the fade must progress evenly even across a dropped
   frame (it is wall-clock timed).
5. **Pacing A/B, Windows 11 and macOS only.** With `fps` equal to the screen
   rate, compare the current `SwapInterval::DontWait` timer path against
   `SwapInterval::Wait(1)` by eye and with the child's frame histogram. Do
   not touch Linux: the egui fork paces Wayland frames by the compositor.
6. **Nothing else changed**: no new setting, no new menu, the same keys and
   the same default 10 s interval.

## Self-review findings

Fixed in this branch:

- `src/milkdrop/child.rs:421`: a switch drawn while the governor held a step
  down was recorded at a quarter of its cost, so the budget could trust a slow
  preset. Now only recorded at full detail.
- `src/milkdrop/governor.rs:94`: the frame that loads a preset was counted as
  a fade frame, so pressing Next through a fade could shrink the picture for a
  fade that was keeping up. Now ignored.

Listed, not fixed (deliberately out of scope, or harmless):

- `src/milkdrop/trace.rs:91` and `:177`: `Trace` keeps every switch for the
  life of the child, without a cap. It only runs when
  `SPOTIFAST_MILKDROP_TRACE` is set, and an entry is a name plus 30 floats, so
  a long traced session grows by a few hundred bytes a switch. A cap would
  lose the top-20 ranking the summary exists for.
- `src/milkdrop.rs:266`: the lookahead does not consult the slow list (see
  risks). Fixing it means deciding whether the child should read ahead the
  automatic pick or the key pick, which is a WP2/WP3 policy question and would
  need its own measurement.
- `src/milkdrop/offscreen.rs:123-146`: the pane child records switch costs but
  has no fade governor, so a skin's crossfade still pays double. Out of scope
  for WP5.
- `src/milkdrop.rs:170`: the slow list never forgets and is not cleared when
  the folder changes. It is one `PathBuf` plus a `Duration` per slow preset,
  so a 10,000-preset folder is around a megabyte at worst.
- `src/milkdrop/prefetch.rs:135`: `Drop` joins the read worker on the window
  thread when the window closes. `bytes` refuses anything but a regular file,
  so the wait is one preset file.
- `src/milkdrop/child.rs:381-397`: the early `self.live.is_none()` return and
  the later `let Some(live) = &mut self.live else` now both guard the same
  field. Harmless (it keeps the pre-borrow clock reads unconditional), left
  as the merge made it.
- Test gap: the wiring of both rules in `Child::render` has no test, because
  that function needs a GL context and a window. The policy on each side is
  unit tested.
- Not compiled here: the `#[cfg(windows)]` `glewInit` block in
  `src/milkdrop/engine.rs:76-96`, the `#[cfg(target_os = "macos")]` screen
  tracking in `src/milkdrop/child.rs:231` and `:455-472`, the
  `#[cfg(all(target_os = "linux", feature = "milkdrop"))]` body of
  `examples/milkdrop_scale_probe.rs` (the other targets get a message), and
  every example on Windows and macOS. None of them were changed by WP1 to
  WP4; they were read by eye.

## Proposed pull request description

> **Smoother MilkDrop preset changes**
>
> MilkDrop used to freeze for a moment every time the preset changed: the
> render thread read the file, parsed it and compiled its shaders inside one
> frame. This branch moves the read off the render thread, keeps presets whose
> first frame is too expensive out of automatic play, and lets a crossfade
> give up detail instead of the frame rate on a machine that cannot draw two
> presets at once.
>
> What changes for you:
>
> - Preset changes read the next preset's file while the current one plays.
> - Automatic play passes over a preset whose switch measured over 50 ms on
>   your machine. **N**, **P** and a click still play it. The list is kept in
>   memory for the window's lifetime; nothing is written to your config.
> - When a crossfade cannot hold your frame rate, it is drawn at half size for
>   the rest of the fade and comes back to full detail two seconds later. A
>   machine that keeps up never sees this.
>
> No new setting and no new interface.
>
> Measurements (Mesa llvmpipe on a Linux build host, so relative only, in
> `PERF_NOTES.md`): the switch is 81.5 ms at the median on a 200-preset
> sample, of which the load is 11.2 ms and the first draw 68.8 ms, and the
> fade window costs 1.71x a steady frame. Drawing a fade at half size removed
> 34% of a 1080p fade frame on the same host. The 25 ms goal in the plan is
> not shown on any machine yet: it needs a real GPU with a display, and the
> shader compile is still on the render thread.
>
> `cargo fmt --check`, `cargo clippy --lib -- -D warnings`, `cargo check
> --lib` and `cargo test --lib milkdrop` pass. Windows and macOS were not
> compiled or run here.
