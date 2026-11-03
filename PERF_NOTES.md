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
