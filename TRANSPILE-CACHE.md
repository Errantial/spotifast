# The preset shader transpile cache

What a preset switch costs, what the cache removes, and what is left.
Numbers are from the RTX 3080 laptop (`PERF_NOTES.md`, "WP7" and "WP8"),
with the protocol, the commands and the acceptance criteria stated so anyone
can repeat them.

WP7 added the cache. WP8 fixed what its review found (an entry cap far below a
real library, pruning by write time and on the render thread, a prune at start
that never ran, and six rejected shaders transpiled on every load), then
measured what was left of a switch and fixed the two largest pieces, neither
of them in the cache's original scope: an antivirus scan of every new entry
paid by the render thread, and a recursive rescan of the texture folders on
every preset load. The WP8 results come first; WP7's measurements follow as
history, with the design they describe replaced where this document says so.

## The problem

WP6 measured a warm preset load inside libprojectM at 85 to 135 ms of CPU
time, and none of it was the driver:

| phase, one preset with both shaders | first load | warm cache |
|---|---:|---:|
| warp: HLSL preprocessor | 45.8 ms | 41.7 ms |
| warp: HLSL parse | 15.2 ms | 14.1 ms |
| warp: GLSL generate | 0.6 ms | 0.4 ms |
| warp: driver compile and link | 65.3 ms | 1.4 ms |
| composite: HLSL preprocessor | 55.6 ms | 41.3 ms |
| composite: HLSL parse | 17.4 ms | 14.7 ms |
| composite: GLSL generate | 0.7 ms | 0.4 ms |
| composite: driver compile and link | 74.4 ms | 1.3 ms |
| whole load (`Engine::load`) | 281.9 ms | 122.9 ms |

The driver's shader cache already makes the compile nearly free on the second
load. What stayed expensive is libprojectM's own transpiler: 60 to 100 ms of
preprocessing and parsing per preset, every time, for source it has seen
before. That is a pure function of the input, so it can be stored.

## What the cache does

`ShaderTranspileCache` (in the libprojectM fork, see `PROJECTM-FORK.md`) keeps
the GLSL that `TranspileHLSLShader` generates, one file per entry in a
platform cache directory, and `MilkdropShader` asks it before transpiling.

### The key

The entry name is the SHA-256 of, in order, each part length-prefixed:

1. a fixed magic string, so the hash is not shared with anything else;
2. the cache format version (`ShaderTranspileCache::CacheFormatVersion`);
3. the shader type (`warp` or `composite`);
4. the GLSL generator version the shaders are generated for
   (`MilkdropStaticShaders::GetGlslGeneratorVersion()`), which covers the
   desktop and GLES shader variants;
5. the highest blur level the shader requires, which decides the `sampler_blur*`
   declarations;
6. the exact sampler and texsize declaration block inserted in front of the
   source. It is empty for textures that could not be loaded, so a preset that
   gains or loses a texture file gets a different entry;
7. the complete preprocessed shader source at the entry of the transpiler:
   libprojectM's preset shader header, the `rad`/`ang`/`uv`/`uv_orig` (and
   `hue_shader`) defines, and the preset's own code, already rewritten from
   `shader_body` into the `PS()` entry point.

Every input of `ApplyPreprocessor`, the declaration stripping, `Parse` and
`Generate` is in that list, so a changed preset, header, declaration or
generator cannot read another shader's GLSL. The vertex shader source is not
in the key: it is not an input of the generated fragment source, and it is
compiled fresh on every hit.

### The hit and miss paths

- Hit: read the file, validate it, and call `Shader::CompileProgram` with the
  stored text. The preprocessor, the declaration stripping, the parse and the
  generate are skipped; the driver compile still happens. The hit also asks
  the background thread to refresh the entry's recency (below); the render
  thread does no I/O for that.
- Miss: look for a reject entry for this key and this driver (below). If there
  is one, throw the same `Renderer::ShaderException` a failed compile throws,
  before transpiling, so the caller takes its usual fallback. Otherwise the
  normal path; after the program compiles, the GLSL is queued for writing.
  Only a successful transpile *and* compile is stored as GLSL.
- A transpile or a driver compile that fails is queued as a reject entry and
  the exception goes on to the caller exactly as before.
- A stored entry that fails to compile is dropped, the shader is transpiled
  again on a clean program object (`Shader::Reset()`), the entry is
  overwritten, and the first occurrence is logged.
- An unreadable, empty, oversized or obviously damaged entry, or an
  unwritable directory, is a miss. Nothing about the cache can stop a preset
  from loading.

### Reject entries

A shader the driver refused is refused again on every later load: the GLSL
is a pure function of the key, and so is the driver's verdict on it, as long
as the driver is the same one. WP7 measured six such shaders in 200 presets,
each re-transpiled (about 15 to 65 ms) and failing again in the driver (30 to
80 ms) on every load.

- The entry is `<hex>.reject`, where `<hex>` is the SHA-256 of a separate
  magic string, the transpile key above and the driver line.
- The driver line is `GL_VENDOR | GL_RENDERER | GL_VERSION |
  GL_SHADING_LANGUAGE_VERSION`, read with `glGetString` when the miss happens,
  for example `NVIDIA Corporation | NVIDIA GeForce RTX 3080 Laptop
  GPU/PCIe/SSE2 | 3.3.0 NVIDIA 617.14 | 3.30 NVIDIA via Cg compiler`. A driver
  update changes `GL_VERSION`, so every reject entry of the old driver stops
  matching and the shader is tried again.
- The file holds three lines: `projectm-shader-reject 1`, the driver line,
  and where it failed with the first line of the error
  (`driver: [Shader] Error compiling fragment shader: 0(131) : error C1068:
  array index out of bounds`). A lookup only counts if the first two lines
  match exactly; anything else is a miss and the shader is tried in full.
- A reject hit logs `[MilkdropShader] Skipping <type> shader this driver
  rejected before: <reason>` at error level, where the full compile used to
  log the driver's error, and throws. The warp shader is then dropped and the
  composite shader replaced by the default one, exactly as after a failed
  compile.
- Transpile failures (preprocessor, parser or generator) are remembered the
  same way, with `transpile:` as the stage.
- In verify mode a reject entry is not trusted: the shader is transpiled and
  compiled again, and if it compiles after all the entry is removed and
  counted as a `reject-verify-mismatch`.

### Storage

- Directory: `%LOCALAPPDATA%\projectm\shader-cache` on Windows,
  `$XDG_CACHE_HOME/projectm/shader-cache` or `~/.cache/projectm/shader-cache`
  elsewhere. Created on first use with `std::filesystem`.
- Entries: `<hex>.glsl` with nothing but the GLSL source, and `<hex>.reject`
  as above.
- **Threads.** The render thread only reads entries (`file_size` and one
  read) and hands work to a queue under a mutex. One worker thread per
  process does everything else: writes, renames, read-backs, mtime refreshes,
  discards, directory scans and pruning. It is started when the cache is
  first used, its first job is the startup scan, it stops after 5 s without
  work and is started again by the next job.
- **Writes** go to `<name>.tmp.<pid>` and are renamed into place (on a failed
  rename the target is removed and the rename tried once more), so a reader
  never sees a partial entry. The worker then reads the new entry back once:
  on Windows the first open of a new file waits for the antivirus scan, about
  7 ms here, and that wait now happens on the worker rather than on the
  render thread's first hit. Later opens take 0.1 ms.
- **A queued store is served from memory** until it is on disk, so a second
  preset with the same shader source hits even before the write.
- **Budget.** The entries (`.glsl` and `.reject`, by file size) may use
  256 MiB (`ShaderTranspileCache::DefaultMaxBytes`). Over it, the least
  recently used entries are deleted until the directory is at 90% of the
  budget, so a full cache is not pruned again on the next store. There is no
  entry count limit any more.
- **Least recently used.** An entry's modification time is the time of its
  last use. A store sets it; a hit asks the worker to set it to now
  (`std::filesystem::last_write_time`), at most once per entry every ten
  minutes per process. The render thread only does a map lookup for that.
  An mtime update does not make the antivirus scan the file again (measured:
  0.11 ms per read after a touch, the same as before it).
- **When pruning runs.** At start (the worker's first job, before any write),
  whenever the worker's running total (the startup scan plus this process's
  own writes) goes over the budget, and on a full rescan every 1024 stores.
  Every scan also deletes temporary files older than an hour.
- **Bounded memory.** The queue holds at most 32 MiB of entries and 65,536
  jobs; work beyond that is dropped and counted (a dropped store is only a
  later miss). The recency maps hold one small record per entry used.
- **Exit.** At process exit queued writes get 2 s; the rest is dropped. If
  the worker is still inside one slow write 2.5 s later, it is detached and
  its state left allocated, rather than hang the exit.
- **Two processes on one directory.** Writes stay atomic (temporary file and
  rename) and every name is unique per process. Two processes pruning at once
  only make some removals find nothing, which counts as removed. A process
  sees the other's new entries only at its own scans (start, over budget,
  every 1024 stores), so together they can overshoot the budget by what the
  other wrote since; the next scan of either brings it back.

### Configuration

Read once, on first use, with surrounding whitespace trimmed:

| variable | effect |
|---|---|
| `PROJECTM_SHADER_CACHE=0` | turns the cache off (also `false`, `no`, `off`) |
| `PROJECTM_SHADER_CACHE_DIR=<path>` | uses this directory instead of the platform one |
| `PROJECTM_SHADER_CACHE_MAX_MB=<n>` | sets the budget to n MiB instead of 256, for testing the pruning |
| `PROJECTM_SHADER_CACHE_VERIFY=1` | transpiles every hit again and logs `VERIFY MISMATCH` if the stored GLSL differs; re-checks every reject entry |
| `PROJECTM_SHADER_CACHE_DEBUG=1` | prints the directory and budget, one line per shader with the phase split, and a three-line summary on exit |

These are libprojectM environment variables, like the WP7 ones; Spotifast
has no setting for any of them and needs none. The cache is on by default and
the child process inherits the environment.

## How to measure

Same protocol as WP6 and WP7, same machine, same folder, same harness, so the
numbers are comparable. `examples/milkdrop_stutter.rs` drives the app's own
frame rules (`--path new`), 200 presets from the 10,184 `.milk` files of the
Cream of the Crop folder, 3 s each, soft cuts, 640x480,
`SwapInterval::DontWait` with the 60 fps software clock.

```bat
call C:\Users\emilg\sfwp5\env.cmd
cd /d C:\Users\emilg\sfwp5\src
cargo build --release --example milkdrop_stutter
C:\Users\emilg\sfwp5\target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 200 --seconds 3
```

(`env.cmd` sets `CARGO_TARGET_DIR=C:\Users\emilg\sfwp5\target`.)

Runs, one at a time, nothing else drawing. In WP8 every run first waited for
the machine's own CPU use to stay under 20% for 20 s (it idles at 13 to 17%
here: a WSL VM, Spotify, Defender); the actual reading is recorded with each
run. An earlier set of WP8 runs was thrown away because another agent's
Android build was using nine cores at the time.

```bat
set "PROJECTM_SHADER_CACHE_DIR=<scratch>\cache200"
set "PROJECTM_SHADER_CACHE_DEBUG=1"
:: (a) off, the baseline
set "PROJECTM_SHADER_CACHE=0"   &  milkdrop_stutter.exe ... --count 200 --seconds 3
:: (b) cold: empty cache
set "PROJECTM_SHADER_CACHE="    &  milkdrop_stutter.exe ... --count 200 --seconds 3
:: (c) warm: the same directory, a new process
                                    milkdrop_stutter.exe ... --count 200 --seconds 3
:: (d) warm again, to show it is stable
                                    milkdrop_stutter.exe ... --count 200 --seconds 3
:: (e) verify: every hit transpiled again and compared, every reject re-checked
set "PROJECTM_SHADER_CACHE_VERIFY=1" &  milkdrop_stutter.exe ... --count 200 --seconds 3
```

The scaling test uses the harness's old path with hard cuts, which loads one
preset after another without the 2.7 s fade in between:

```bat
milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --path old --hard --seconds 0.05 --count 20000
```

`--count 40 --seconds 3` is enough while iterating (about 2 minutes).

## WP8 results

Build: Spotifast `wp8-stutter-fixes` with `projectm-sys` pinned to
Errantial/projectm-rs `ea2d84e` (libprojectM Errantial/projectm `d3d81668e`),
built from that git pin, no diagnostic code. NVIDIA GeForce RTX 3080 Laptop
GPU, driver 617.14, Windows 11, release build, MSVC 19.51.

### Summary, per review finding

Branches and heads: Errantial/projectm `wp8-stutter-fixes` at `d3d81668e`
(five commits on WP7's `05178a4f9`), Errantial/projectm-rs
`wp8-stutter-fixes` at `ea2d84e` (one commit on WP7's `8ad31cf`),
Errantial/spotifast `wp8-stutter-fixes` (on WP7's `8dc3e9b`: the governor fix
`e9f4f9b`, the pin `369e91f`, then these notes). `Cargo.toml` pins
`projectm-sys` to `ea2d84ec529302f20e6cd456e162f894efe8c3df`.

- **A1, the cap.** A byte budget of 256 MiB replaces the 4096-entry cap. The
  whole 10,184-preset library takes 5,104 entries and 35.1 MB (6,886 bytes on
  average), so the budget holds about 39,000 entries, 7.6 times the library.
- **A2, LRU.** A hit queues a recency refresh (a map lookup on the render
  thread); the worker sets the entry's mtime, at most every ten minutes per
  entry, and the prune deletes the oldest mtime first. The LRU unit test
  fails when the refresh is removed.
- **A3, off the render thread.** Every write, read-back, mtime refresh,
  discard, scan and prune runs on one worker thread per process; the queue is
  bounded (32 MiB, 65,536 jobs); exit waits up to 2 s for queued writes.
- **A4, prune at start.** It is the worker's first job, before any write, and
  this document describes the code as it is ("Storage").
- **B, rejected shaders.** Reject entries keyed on the transpile key and the
  GL driver strings; the six presets load in 3.4 to 7.7 ms instead of 160 to
  218 ms and take the same fallback; verify re-checks them.
- **C1, the residual split.** After A and B the largest pieces were the
  antivirus scan of each new entry on the render thread's first hit (cache
  read p99 24 ms) and a recursive texture-folder rescan at every load
  (textures p99 20 ms). Both are fixed (read-back on the worker, background
  rescans). What is left: ns-eel compile and init code (p99 11.3 ms, worst
  27.6), driver compile and link (p99 3.7 ms), one texture scan per process
  (14.7 ms), the initial-image draw (7.3 ms once).
- **C2, driver compile.** Not significant (p50 1.4, p99 3.7 ms for both
  programs; the first draw is not compile-bound), so no program-binary cache
  and no pre-warming.
- **C3, the governor.** Fixed in `src/milkdrop/policy.rs` with two tests.
- **C4, the fade.** Measured: the outgoing preset does no work during a fade
  it did not do before it, and the transition draw is 0.04 ms. Not reducible
  without a visual change or running two presets in parallel; unchanged.
- **The slow list** is unchanged; it excludes 1 of 200 presets with a warm
  cache (WP7 about 10, WP6 163).

### The five runs, 200 presets, 640x480, soft cuts

Machine CPU before each run: 14.7%, 16.3%, 14.0%, 14.7%, 13.1%. Times in ms.

| metric | (a) off | (b) cold | (c) warm | (d) warm again | (e) verify |
|---|---:|---:|---:|---:|---:|
| preset load p50 / p99 / max | 138.1 / 226.9 / 280.8 | 118.3 / 238.5 / 262.4 | **4.9 / 17.6 / 35.6** | **4.9 / 22.4 / 37.7** | 135.7 / 236.0 / 245.4 |
| loads over 8 ms / over 25 ms | 165 / 165 | 158 / 158 | **11 / 2** | **14 / 2** | 165 / 165 |
| first draw p50 / p99 | 4.6 / 22.8 | 5.1 / 22.9 | 4.7 / 23.6 | 4.7 / 23.9 | 4.6 / 22.5 |
| switch cost (load + first draw) p50 / p99 / max | 147.1 / 240.2 / 284.2 | 123.6 / 243.6 / 270.7 | **10.0 / 40.9 / 50.9** | **9.9 / 40.3 / 67.2** | 141.0 / 243.4 / 253.8 |
| switches over 50 ms (what the slow list excludes) | 165 | 158 | **1** | **1** | 165 |
| the switch frame itself p50 / p99 | 160.1 / 255.2 | 138.8 / 260.1 | 24.6 / 55.9 | 24.6 / 56.1 | 155.4 / 258.7 |
| worst frame per switch (whole 3 s turn) p50 / p99 / max | 160.1 / 283.2 / 321.3 | 141.6 / 260.1 / 286.5 | 25.0 / 63.2 / 292.7 | 25.0 / 70.2 / 157.5 | 155.8 / 264.2 / 287.9 |
| worst frame in the 1 s after, p99 | 255.2 | 260.1 | 61.2 | 66.3 | 258.7 |
| frame during fade p50 / p99 | 16.7 / 35.5 | 16.7 / 35.5 | 16.7 / 35.3 | 16.7 / 35.1 | 16.7 / 35.3 |
| frame steady p50 / p99 | 16.6 / 18.2 | 16.7 / 18.2 | 16.6 / 18.1 | 16.7 / 18.5 | 16.7 / 18.2 |
| fade p99 / steady p99 | 1.95x | 1.94x | 1.95x | 1.90x | 1.94x |
| frame rate | 56.1 fps | 56.6 fps | **58.7 fps** | **58.7 fps** | 56.2 fps |
| all 200 loads together | 24.3 s | 21.0 s | **1.05 s** | **1.08 s** | 23.8 s |
| lookups / hits / reject hits | - | 319 / 48 / 0 | 319 / 313 / 6 | 319 / 313 / 6 | 319 / 313 / 6 |
| governor: fade frames drawn smaller | 1335 of 29999 | 1223 of 30261 | 1378 of 31566 | 1249 of 31584 | 1126 of 30044 |

The worst-frame maxima of 292.7 and 157.5 ms in (c) and (d) are single steady
frames in the middle of a preset's turn, not switches (the steady maximum of
those runs); everything else at the top of those columns is a fade frame.

### WP7 against WP8, warm cache in a new process

| metric | WP7 (c) | WP7 (d) | WP8 (c) | WP8 (d) |
|---|---:|---:|---:|---:|
| preset load p50 / p99 / max | 5.1 / 111.6 / 155.7 | 5.2 / 120.4 / 319.7 | **4.9 / 17.6 / 35.6** | **4.9 / 22.4 / 37.7** |
| loads over 8 ms | 54 | 47 | **11** | **14** |
| loads over 25 ms | 16 | 16 | **2** | **2** |
| first draw p50 / p99 | 4.8 / 31.9 | 4.9 / 34.5 | 4.7 / 23.6 | 4.7 / 23.9 |
| switch cost p50 / p99 | 11.3 / 115.7 | 11.0 / 126.8 | **10.0 / 40.9** | **9.9 / 40.3** |
| switch frame p50 / p99 | 25.1 / 131.4 | 25.1 / 141.5 | 24.6 / 55.9 | 24.6 / 56.1 |
| worst frame per switch p50 / p99 | 26.5 / 159.0 | 25.8 / 162.9 | 25.0 / 63.2 | 25.0 / 70.2 |
| frame during fade p99 | 39.6 | 40.0 | 35.3 | 35.1 |
| frame steady p99 | 21.6 | 29.9 | 18.1 | 18.5 |
| fade p99 / steady p99 | 1.83x | 1.34x | 1.95x | 1.90x |
| frame rate | 57.1 fps | 56.9 fps | **58.7 fps** | **58.7 fps** |
| all 200 loads together | 2.35 s | 2.56 s | **1.05 s** | **1.08 s** |
| hit rate | 98.1% | 98.1% | 98.1% plus 6 reject hits | 98.1% plus 6 reject hits |

The load p99 fell from 112 to 120 ms to 18 to 22 ms, and the switch p99 from
116 to 127 ms to 40 ms. The fade ratio looks worse only because the steady
p99 is lower on this quieter run (18 ms against 22 to 30); the fade p99 itself
went from 40 to 35 ms.

What did it, measured with per-phase timers in a diagnostic build of the same
code (not committed; below):

1. **The six rejected shaders** now cost a 3.4 to 7.7 ms load instead of 160 to
   218 ms (table in "Reject entries, measured").
2. **The texture rescan.** `TextureManager::PurgeTextures()` ran on every
   preset load and threw the scanned texture list away, so the next texture
   that was not loaded yet, and every `rand` texture, walked every texture
   search path recursively on the render thread. Spotifast passes the preset
   folder itself as a search path, so that is 10,184 files: p90 14.3 ms and
   p99 20.5 ms of texture time per load. It is now one scan at the first load
   and background scans after that: p90 0.01 ms, p99 0.02 ms, and the single
   first scan (14.7 ms) is the only texture cost left.
3. **The antivirus scan of a new entry.** The first open of a file Windows has
   not seen yet costs about 7 ms here (Defender), later opens 0.1 ms. WP7's
   warm run was always the first open of entries the cold run had written, so
   every hit paid it: cache read p90 12.3 ms, p99 24.0 ms per load. The
   writer thread now reads each entry back once after writing it: cache read
   p90 0.69 ms, p99 0.89 ms, max 1.14 ms.

Measured directly with a script that writes 265 cache-sized files and reads
them: first read after write p50 6.7 / p99 10.7 ms, second read p50 0.11 ms,
read after an mtime touch p50 0.11 ms, read a minute later p50 0.14 ms.

### Reject entries, measured

The six presets whose shader this driver rejects, one load each per run:

| preset | rejected shader | off | cold | warm (c) | warm (d) | first draw off / warm |
|---|---|---:|---:|---:|---:|---:|
| Krash + Geiss + martin - War Machine (Stahl's ultimate Mash Mix) | composite | 185.9 | 177.3 | 3.4 | 4.2 | 2.3 / 2.4 |
| ORB - Stahl - Fire Worm | composite | 179.7 | 162.6 | 4.4 | 5.2 | 2.2 / 2.2 |
| Rovastar & Zylot - Crystal Ball (Too Many Visions Mix) nz+ chef's table too good for you | warp | 199.6 | 193.0 | 7.7 | 6.4 | 4.5 / 3.5 |
| shifter + flexi - liquid circuitry from the fractalism hive --- Isosceles edit1 | composite | 198.8 | 89.7 | 4.5 | 5.3 | 2.4 / 2.7 |
| shiter redi jedi- agar awkward seances | warp | 160.5 | 182.7 | 5.1 | 5.4 | 16.8 / 14.9 |
| suksma - water + god roam3 | composite | 217.5 | 83.8 | 5.4 | 4.2 | 3.5 / 4.4 |

They render as before because the code path after the skip is the same one:
the reject hit throws the same `Renderer::ShaderException` at the same point,
so `PerPixelMesh::CompileWarpShader` drops the warp shader and
`FinalComposite::CompileCompositeShader` falls back to the default composite
shader, as after the failed compile. Their first draw, which is one frame of
the new preset, costs the same with and without the cache, which it would
not if a different shader were drawn. Verify mode (e) re-checked all six: the
shader still fails in the driver every time (0 reject mismatches). The reason
stored for one of them, for the record:
`driver: [Shader] Error compiling fragment shader: 0(131) : error C1068:
array index out of bounds`. That they look right in the window still wants a
human eye (see the end of this document).

### Where a warm load goes now

The diagnostic build added a `steady_clock` read around each step of
`ProjectM::LoadPresetData`, `MilkdropPreset::Initialize` and
`MilkdropShader::LoadTexturesAndCompile` (the patch is not on any branch).
Warm cache, 197 loads of the 200-preset run, ms:

| phase | p50 | p90 | p99 | max | sum of 197 |
|---|---:|---:|---:|---:|---:|
| whole load | 4.72 | 7.33 | 19.93 | 33.64 | 1046.8 |
| driver compile and link, both shaders | 1.36 | 2.55 | 3.74 | 5.64 | 272.7 |
| ns-eel compile and init code (per-frame, per-vertex, waves, shapes) | 0.55 | 1.52 | 11.33 | 27.65 | 203.3 |
| preset state, waves and shapes from the parsed text | 0.72 | 1.08 | 1.57 | 2.06 | 151.9 |
| cache reads, both shaders | 0.54 | 0.69 | 0.89 | 1.14 | 93.5 |
| framebuffer sizes | 0.44 | 0.59 | 0.79 | 0.88 | 90.9 |
| drawing the old picture into the new preset | 0.31 | 0.43 | 0.88 | 7.31 | 71.9 |
| reading the `.milk` text | 0.21 | 0.38 | 0.59 | 0.73 | 46.4 |
| textures | 0.00 | 0.01 | 0.02 | 14.70 | 15.5 |
| cache keys (SHA-256), both shaders | 0.07 | 0.09 | 0.13 | 0.17 | 11.7 |
| shader code preprocessing (string work) | 0.03 | 0.07 | 0.13 | 0.21 | 7.5 |
| transpile | 0 | 0 | 0 | 0 | 0 |

The same split before the texture and read-back fixes (A and B only), same
cache, same presets: whole load p50 / p90 / p99 5.53 / 23.50 / 43.08, cache
reads 0.59 / 12.32 / 23.97, textures 0.00 / 14.34 / 20.47, ns-eel 0.50 /
1.47 / 12.65, driver 1.31 / 2.59 / 4.07.

The top contributors to what is left, by name:

1. **ns-eel expression compile and init code**: the two slowest warm loads
   (27.6 and 27.0 ms of 33.6 and 31.6), and 11.3 ms at the p99. This is
   projectm-eval compiling the preset's per-frame and per-vertex code and
   running its init code, on the render thread, inside
   `MilkdropPreset::Initialize`. Nothing here caches it.
2. **The driver compile and link**: the largest sum (26% of all load time)
   but never large itself, 1.4 ms at the median and 3.7 ms at the p99 for
   both programs. NVIDIA's own shader cache is already warm.
3. **The first scan of the texture folders**, once per process (14.7 ms).
4. **Drawing the previous picture into the new preset** (`DrawInitialImage`),
   7.3 ms once.
5. Everything else is under 2 ms at the p99.

Misses contribute nothing: every lookup in a warm run is a hit or a reject
hit.

### The first draw and the fade

The first draw of a new preset is one fade frame: both presets render and
the transition shader blends them. Its p99 is the same with the cache off
(22.8 ms) and warm (23.6 ms), so the driver is not doing deferred compile work
there, and it matches the CPU cost of a heavy fade frame below.

A second diagnostic build timed `ProjectM::RenderFrame` during each fade: the
incoming preset's `RenderFrame`, the outgoing preset's `RenderFrame` and the
transition draw, CPU time, 198 fades:

| per fade, mean per frame | p50 | p90 | p99 | max |
|---|---:|---:|---:|---:|
| both presets and the transition | 3.06 | 12.58 | 19.29 | 22.68 |
| incoming preset | 1.39 | 5.33 | 13.82 | 15.63 |
| outgoing preset | 1.26 | 4.71 | 13.91 | 15.62 |
| transition draw | 0.04 | 0.05 | 0.16 | 0.17 |
| outgoing preset, the frames before the fade | 1.48 | 5.72 | 15.56 | 16.83 |

The worst single fade frame was 33.7 ms of CPU at the p99. 22 of 198 fades
averaged over 12 ms of CPU a frame, and in those the outgoing preset costs
what it cost before the fade (15.6 against 16.8 ms, 13.9 against 14.0, 13.9
against 15.6). The fade is the sum of two presets' own per-frame CPU work
(per-frame and per-vertex equations, custom waves and shapes): no work is
done for the outgoing preset that it did not do on its own, and the
transition costs 0.04 ms. In 12 of the 29 fades over 25 ms at the p99 in the first WP8
warm run, neither preset was over 20 ms on its own; only the two
together are.

So the fade cannot be made cheaper without a change of what is drawn (fewer
outgoing-preset updates, which is a visual change) or without running the two
presets' CPU work in parallel, which libprojectM does not allow today
(`EvalLibMutex.cpp` serialises projectm-eval) and which is not a cheap or
safe change. WP8 does not change the fade.

### Program binaries and warming (C2)

Not done, by the numbers above: the driver compile and link is 1.4 ms at the
median and 3.7 ms at the p99 for both programs of a preset, the first draw is
not compile-bound, and the NVIDIA driver's shader cache already turns a 15 to
80 ms cold compile into 0.7 to 1.8 ms (WP6). A `glProgramBinary` cache could
save at most part of those few milliseconds, on this driver, at the cost of a
second on-disk format keyed on the driver. Compiling the next preset's
programs ahead would need a second GL context (`DESIGN-offthread-compile.md`)
for the same few milliseconds.

### The governor (C3)

`Policy` fed the governor the gap from the previous frame's start, which is
the previous frame's duration, and passed `switched` for the current frame.
The frame after a switch therefore reported the switch's long frame as a late
fade frame, and a Next during a fade that was four frames late in eight tipped
it over the five-frame threshold. `Policy` now passes `switched` for both the
switch frame and the frame whose gap contains it. Two tests in
`src/milkdrop/policy.rs`:

- `the_frame_after_a_switch_does_not_judge_a_fade`: four late frames, a
  115 ms switch frame, then on-time frames; the picture stays at full detail.
  It failed before the fix (the governor stepped down) and passes after. The
  same frames without the switch frame step down, so the exclusion is the
  switch and not every long gap.
- `a_heavy_fade_after_a_switch_still_steps_down`: eight late frames after a
  switch still step down; only one frame is left out.

In the runs above the governor drew 1,126 to 1,378 of about 30,000 fade frames
smaller (3.7 to 4.4%), close to the 4.6% WP6 reported.

### The slow list

The WP6 slow list keeps a preset out of automatic play once one switch to it
cost more than 50 ms (`SWITCH_BUDGET`). How many of the 200 presets it would
exclude, by run:

| run | switches over 50 ms |
|---|---:|
| WP6, no cache | 163 |
| WP7, warm | about 10 (10 and 11 loads over 50 ms) |
| WP8, cache off | 165 |
| WP8, cold | 158 |
| WP8, warm | **1** in each run (`EVET + Flexi - Dreamsicles`, 50.9 and 67.2 ms) |

With a warm cache the list is close to empty, and the one preset on it is the
ns-eel compile above, which is a slow switch. The list itself is unchanged.

One interaction the cache creates, measured and not changed: the list learns
from a preset's first switch, and with an empty cache that switch is the full
transpile. `--count 40 --passes 2` on a cold cache directory, one process:

| | pass 1 (cold) | pass 2 (same process, cache now warm) |
|---|---:|---:|
| switches over 50 ms | 32 of 40 | 0 |
| distinct presets automatic play chose | 40 | **10** |
| load p50 / max | - | 3.0 / 6.9 ms |

Pass 2 loaded every preset in 7 ms at most, but automatic play only cycled 10 of the 40
presets, because the list keeps every preset whose first, uncached switch was
slow out of automatic play for as long as the window lives, although its next
switch would cost about 10 ms. The next session starts with an empty list
and a cache that is warm for what was seen, so this only lasts until the
window closes, and on a large
folder automatic play rarely comes back to a preset in one session anyway. On
a small folder it is visible. A possible change, for the maintainer to decide
(it changes WP6's policy): exclude a preset only after its second slow switch,
which a warm cache answers in about 10 ms for all but one preset of the 200.

### Scaling: the whole library

Every one of the 10,184 presets of the folder, loaded one after another
(`--path old --hard --seconds 0.05`), into an empty directory with the
default 256 MiB budget, then again in a new process. Machine CPU before the
runs: 15.3% and 14.6%.

| | fill (cold) | second pass, new process |
|---|---:|---:|
| presets loaded | 10,184 | 10,184 |
| shader lookups | 16,302 | 16,302 |
| hits / misses | 11,034 / 5,268 | **15,959 / 343** |
| of the misses, reject hits (no transpile) | 164 | **343 (all of them)** |
| transpiles | 5,268 (72.5 ms each) | **0** |
| entries written / pruned | 5,104 / 0 | 0 / 0 |
| render-thread cache read, total / max | 3.22 s / 265.3 ms | 3.87 s / **0.79 ms** |
| preset load p50 / p99 | 16.8 / 413.1 ms | 6.3 / 23.5 ms |
| worker: write time / scans (time) | 54.3 s / 5 (126 ms) | 0 / 1 (11.4 ms, the startup scan of 5,104 entries) |
| mtime refreshes | 443 | 5,178 |

- **What the library needs:** 5,104 entries, 4,925 of them GLSL (average
  7,127 bytes, 5,396 to 15,516) and 179 rejects (233 bytes), **35.1 MB**
  (45 MB on disk with NTFS's 4 KB clusters). WP7's estimate of ~13,600
  entries counted every shader of every preset; different presets share
  shader text, and many have none. The library is still over WP7's 4,096-entry
  cap, which pruned by write time, so a WP7 cache could never hold it.
- **Capacity:** at the measured 6,886 bytes an entry, 256 MiB holds about
  **39,000 entries, 7.6 times this library**.
- **No thrash:** the second pass transpiled nothing. Every one of its 16,302
  lookups was a hit (97.9%) or a reject hit (2.1%), and nothing was pruned.
- **The 265 ms read:** one read during the fill, while the worker was writing
  5,104 entries and reading each back on the same disk, took 265 ms. The
  second pass's worst read was 0.79 ms. A read on the render thread is still
  I/O and can wait behind other I/O on a busy disk; it is the one piece of the
  cache that stays on that thread.

The 179 reject entries are 179 distinct shader sources (in 343 lookups) that
this driver refuses, against 6 in the 200-preset sample: 3.5% of the
library's distinct shaders.

### Scaling: pruning under load

3,000 presets spread over the same folder, into an empty directory with
`PROJECTM_SHADER_CACHE_MAX_MB=8`, less than half of the roughly 19 MB they need, so the
worker has to prune all the way through (machine CPU 18.4% before the run):

| | value |
|---|---:|
| entries written / pruned | 2,656 / 1,488 |
| scans and prunes on the worker | 14, 507 ms together (36 ms each) |
| directory after the last scan / when the run ended | 1,116 entries and 7.55 MB of an 8 MiB budget / 1,168 entries |
| render-thread cache read, worst | **0.80 ms** |
| dropped jobs / store failures | 0 / 0 |

While the worker scanned 14 times and deleted 1,488 files, no read on the
render thread took more than 0.80 ms, and no scan or prune ran there at all
(the render thread has no code path that scans: `Lookup` reads one file,
`Store`, `Discard` and the recency refresh only queue). A budget smaller than
the working set does make a cyclic pass miss (44.8% hits); that is the budget,
and the default is 7.6 times the whole library.

### Verify mode

`PROJECTM_SHADER_CACHE_VERIFY=1`, run (e) above, entries written by the
cold run (b) in another process: **313 hits transpiled again, 0 mismatches;
6 reject entries re-checked by a full transpile and compile, all 6 still
rejected by the driver, 0 reject mismatches.** The load cost in that run is
the full transpile again (p50 135.7 ms), which is what verify mode is for.

### A damaged cache

A copy of the 200-preset directory (265 GLSL entries, 6 rejects) damaged the
WP7 way, plus the reject entries: 14 entries deleted, 14 emptied, 14
truncated to 200 bytes, 14 prefixed with a line that is not a declaration (so
they look like GLSL, are read as hits and then fail to compile), 3 reject
entries overwritten with garbage and 3 rewritten for another driver. Two runs
of 40 presets over it:

| | first run | second run |
|---|---:|---:|
| lookups / hits / misses | 62 / 55 / 7 | 62 / 61 / 1 |
| entries that could not be read | 3 | 0 |
| stored entries that failed to compile | 1 | 0 |
| reject hits / rejects stored again | 0 / 1 | 1 / 0 |
| store failures | 0 | 0 |
| presets that failed to load | 0 | 0 |
| load p50 / p99 | 20.8 / 150.6 | 4.6 / 20.5 |

Every damaged entry the 40 presets touched cost one transpile and was
rewritten by the worker; the damaged reject entry was not trusted, the shader
was tried in full, failed in the driver as before and was recorded again. The
second run reads the directory as healthy. A cache directory that cannot be
created still disables the cache with the WP7 message, and one that cannot be
written to still only counts store failures.

### Two processes on one directory

Two harness processes started together on one directory with a 2 MiB budget
(`--path old --hard --seconds 0.05`, 600 and 601 presets spread over the
folder, so they share most shaders and both prune all the time):

| | process A | process B |
|---|---:|---:|
| lookups / hits | 984 / 351 | 929 / 173 |
| entries written / pruned | 629 / 382 | 754 / 698 |
| scans | 14 | 18 |
| store failures / unreadable / dropped | 0 / 0 / 0 | 0 / 0 / 0 |

Both finished. Between them they pruned 1,080 entries while the other was
reading and writing the same files, without a failed write, a partial read or
a failed load, and the directory ended at 1.96 MB of its 2 MiB with 291
entries and no temporary files left behind.

### Against the acceptance criteria (warm cache, new process)

| criterion | WP8 (c) | WP8 (d) | result |
|---|---:|---:|---|
| worst frame at a switch, p99 <= 25 ms | 63.2 (whole turn), 55.9 (the switch frame) | 70.2, 56.1 | **not met** |
| worst frame at a switch, p50 (context) | 25.0, 24.6 | 25.0, 24.6 | at the target |
| fade p99 <= 1.25x steady p99 | 1.95x (35.3 / 18.1) | 1.90x (35.1 / 18.5) | **not met** |
| >= 58 fps | 58.7 | 58.7 | **met** |
| no preset load blocking the render thread > 8 ms | 11 of 200 over 8 ms, p99 17.6, max 35.6 | 14 of 200, p99 22.4, max 37.7 | **not met**; p50 4.9 |
| no cache disk I/O but one read per shader on the render thread | met by construction | | **met** (writes, touches, scans, prunes all on the worker) |
| no render-thread prune stall | 0 scans or prunes on the render thread | | **met** (scaling test below) |
| verify mode, 0 mismatches | 313 hits and 6 rejects re-checked, 0 and 0 | | **met** |

Why the three that are not met cannot be met by this work, with the evidence
above:

- **The worst frame at a switch** is bounded below by the fade, not the load.
  The worst frame of a 3 s turn is a fade frame for every preset whose fade
  frame exceeds its switch frame, and the fade p99 is 35 ms because two heavy
  presets' CPU work adds up (22 of 198 fades average over 12 ms of CPU a
  frame). The switch frame itself is load (p99 18 to 22 ms) plus the first
  draw, which is a fade frame (p99 24 ms). The harness also counts a frame up
  to its own 16.7 ms deadline, so 25 ms leaves about 8 ms for the work of the
  frame.
- **The fade ratio** is the same two presets' CPU, measured above: the
  transition is 0.04 ms, and the outgoing preset does nothing during the fade
  it did not do before it.
- **Loads over 8 ms**: 11 to 14 of 200 remain, and apart from the one
  texture scan per process none is cache, transpile or disk work. They are ns-eel compile and init code (up to
  27.6 ms), driver compile and link (up to 5.6 ms), the first texture scan of
  the process (14.7 ms, once) and the old picture drawn into the new preset
  (up to 7.3 ms). Moving the ns-eel compile off the render thread would need
  libprojectM to split preset construction into a part without GL calls and a
  part with them; `MilkdropPreset::Initialize` mixes both today.

## WP7 measurements (history)

These are the WP7 runs, kept for the comparison. The storage they describe
(4096 entries, 64 MB, pruning by write time on the render thread) was
replaced in WP8; the cache's key and hit path are unchanged.

### The four runs, 200 presets, 640x480, soft cuts

All five runs use the same sample of the same folder, one after another on
the same machine, nothing else drawing. `(a)` is the cache turned off, `(b)`
the first pass into an empty cache directory, `(c)` a new process with the
directory filled by `(b)`, `(d)` the same again to show it is stable, and
`(e)` the verify pass described below. Times in ms.

| metric | (a) off | (b) cold | (c) warm | (d) warm again |
|---|---:|---:|---:|---:|
| preset load p50 | 127.6 | 115.6 | **5.1** | **5.2** |
| preset load p99 | 402.7 | 254.5 | **111.6** | **120.4** |
| preset load max | 849.1 | 361.5 | 155.7 | 319.7 |
| loads over 8 ms | 170 of 200 | 160 of 200 | 54 of 200 | 47 of 200 |
| first draw p50 / p99 | 5.2 / 37.7 | 5.5 / 29.1 | 4.8 / 31.9 | 4.9 / 34.5 |
| switch cost p50 / p99 | 135.1 / 427.8 | 122.1 / 276.6 | **11.3 / 115.7** | **11.0 / 126.8** |
| switch cost max | 855.9 | 372.8 | 158.4 | 454.4 |
| worst frame per switch p50 / p99 | 151.4 / 438.9 | 137.9 / 318.0 | **26.5 / 159.0** | **25.8 / 162.9** |
| worst frame per switch max | 924.0 | 411.2 | 177.9 | 497.3 |
| worst frame in the 1 s after, p99 | 438.9 | 318.0 | 131.4 | 148.0 |
| stall frame p50 / p99 | 150.5 / 438.9 | 136.2 / 291.0 | 25.1 / 131.4 | 25.1 / 141.5 |
| frame during fade p50 / p99 | 16.7 / 42.9 | 16.7 / 43.8 | 16.7 / 39.6 | 16.7 / 40.0 |
| frame steady p50 / p99 | 16.6 / 23.4 | 16.7 / 33.6 | 16.7 / 21.6 | 16.7 / 29.9 |
| fade p99 / steady p99 | 1.83x | 1.30x | 1.83x | 1.34x |
| achieved frame rate | 53.3 fps | 53.9 fps | **57.1 fps** | **56.9 fps** |
| all 200 loads together | 26.5 s | 21.2 s | **2.35 s** | **2.56 s** |
| cache hit rate | - | 15.0% | 98.1% | 98.1% |

Read the load row first: 127.6 ms at the median became 5.1 ms, and the 200
loads together went from 26.5 s of CPU to 2.35 s. The p50 of a switch went
from 135 to 11 ms and its p99 from 428 to 116 ms. Nothing else moved much,
because nothing else is in the transpile's path.

The cold run's 15% hit rate is not a mistake: different presets often carry
identical shader source (mashups of the same base preset), so the 200 presets
need only 265 distinct entries and the later ones are hits even on the first
pass.

### Run to run

Four more warm runs of the same 200 presets, on the same cache directory, on
the build this branch pins, say how much of the spread is the cache and how
much is the machine:

| warm run | load p50 | load p99 | switch p99 | worst frame p99 | fps | cache read, mean |
|---|---:|---:|---:|---:|---:|---:|
| (c) | 5.1 | 111.6 | 115.7 | 159.0 | 57.1 | 0.34 ms |
| (d) | 5.2 | 120.4 | 126.8 | 162.9 | 56.9 | 0.33 ms |
| final build, quiet machine | 7.2 | 89.8 | 95.5 | 118.5 | 55.5 | 1.22 ms |
| final build, just after the directory was rewritten | 19.7 | 95.0 | 99.4 | 155.2 | 57.5 | 6.65 ms |

The last row is the honest worst case and it is I/O, not code: those 313
entries were rewritten minutes earlier and every read cost 6.6 ms, which
added 13 ms to every load. Ten minutes later, the same binary read the same
files at 0.3 ms. So the load median with a warm cache is **5.1 to 7.2 ms**,
and it can be worse on a busy disk. The p99 is 90 to 120 ms whatever the
disk does, because it is those six shaders below, not the cache.

The frame-level numbers move between runs too (55.5 to 57.5 fps, worst-frame
p99 118 to 163 ms); this laptop is not a quiet bench. The load and switch
columns are the ones that carry the change, and they are stable in direction
across every run.

### Where the time goes now: per-shader split

From `PROJECTM_SHADER_CACHE_DEBUG=1`, medians over the 200-preset runs. The
miss columns are `(b)`'s transpiles; the hit columns are `(c)`'s cache reads.
`(b)` had 271 miss lookups, of which 6 transpiled and then failed to compile
in the driver (see "Six shaders the driver rejects"), so 265 were stored and
logged.

| phase, one preset shader | warp, miss (ms) | composite, miss (ms) | warp, hit (ms) | composite, hit (ms) |
|---|---:|---:|---:|---:|
| HLSL preprocess | 49.4 | 52.2 | skipped | skipped |
| declaration strip and insert | 0.02 | 0.02 | skipped | skipped |
| HLSL parse | 13.8 | 13.6 | skipped | skipped |
| GLSL generate | 0.47 | 0.44 | skipped | skipped |
| **transpile, all of it** | **65.0** | **65.4** | **skipped** | **skipped** |
| cache entry read | - | - | 0.34 (all hits) | 0.34 (all hits) |
| driver compile and link | 0.9 | 0.7 | 0.84 | 0.64 |
| p99 of the transpile | 154.6 | 139.5 | - | - |
| p99 of read and compile | - | - | 7.2 | 5.6 |

Totals over each run: 271 transpiles cost 18.6 s of CPU (preprocess 14.3 s,
parse 4.1 s, generate 0.14 s, strip 0.01 s) and 265 entries were written; the
313 hits of `(c)` cost 0.31 s of cache reads and 0.33 s of driver compiles.
The driver was already cheap on this machine, which is exactly why the
transpile was worth caching: it was 85 to 135 ms of a load, and the driver
was 2 to 3 ms of it.

The cache costs the cold pass 1.9 ms per entry to write (514 ms for 271
entries), and 200 presets occupy 267 entries, 1.8 MB, 6.8 KB each (5.3 to
11.3 KB).

### Verify mode

`PROJECTM_SHADER_CACHE_VERIFY=1`, same 200 presets, entries written by the
three earlier processes: **313 hits, 313 fresh transpiles compared, 0
mismatches**. The load cost in that run is the full transpile again (p50
118.1 ms), which is what verify mode is for: it is a test of the key, not a
fast path. The same run shows the key is stable across processes: a fresh
transpile in a new process produces byte-identical GLSL to what an earlier
process stored, for every one of the 313 entries.

The build this branch pins repeated it: 61 hits over 40 presets, 0
mismatches. Every entry those runs read was written by a different process.

### A damaged cache

The cache is only a cache, so the test is what a damaged directory costs.
A copy of the 267 entries was damaged deliberately: 14 entries deleted, 14
emptied, 13 truncated to their first 200 bytes, and 13 prefixed with a line
of text that is not a declaration (so they still look like GLSL, are read as
hits and then fail to compile). Two runs of 40 presets over that directory:

| | first run | second run |
|---|---:|---:|
| lookups / hits / misses | 62 / 52 / 10 | 62 / 61 / 1 |
| entries that could not be read | 6 | 0 |
| stored entries that failed to compile | 2 | 0 |
| store failures | 0 | 0 |
| presets that failed to load | 0 | 0 |
| hit rate | 83.9% | 98.4% |

Every damaged entry was a miss, costing one transpile, and was rewritten by
the same load. The second run over the same directory reads it as healthy
again (the one remaining miss is one of the six shaders the driver rejects).
The two entries that were read as hits and then failed to compile took the
documented path: the entry was dropped, the shader was transpiled again on a
fresh program object, and the entry was overwritten.

A cache directory that cannot be created at all, for example
`PROJECTM_SHADER_CACHE_DIR=C:\Windows\System32\drivers\etc\hosts\wp7cache`
(a path under a file), prints

```
[projectm shader cache] disabled: cannot create C:\Windows\System32\drivers\etc\hosts\wp7cache: Cannot create a file when that file already exists.
```

and the run continues with every shader transpiled, exactly as with
`PROJECTM_SHADER_CACHE=0`: the same 5 presets took 12 to 217 ms to load, each
paying its full transpile.

A directory that exists but cannot be written to is the other half of the
same rule, and was hit by accident during development: a cache directory
whose name ended in a space made every write fail. The run was a normal one
with 33 counted store failures, 33 misses, no error surfaced to the app, and
no entry in the directory. Store failures never reach the render path.

### Checks

`cargo fmt --check`, `cargo clippy --lib -- -D warnings` and
`cargo test --lib milkdrop` pass on this branch. The Rust side of this work is
one `Cargo.toml` pin; the C++ has no unit-test target in this fork, so the
harness runs and the verify mode are its test.

### Against the acceptance criteria

| criterion | measured (c) | measured (d) | result |
|---|---:|---:|---|
| worst frame at a switch, p99 <= 25 ms | 159.0 | 162.9 | **not met** |
| worst frame at a switch, p50 (context) | 26.5 | 25.8 | at the target |
| fade p99 <= 1.25x steady p99 | 1.83x | 1.34x | **not met** |
| >= 58 fps average | 57.1 | 56.9 | **not met**, 1 fps short |
| no preset load over 8 ms | 54 of 200 | 47 of 200 | **not met**, p50 5.1 |
| preset load p50 <= 8 ms | 5.1 | 5.2 | met |
| no preset load over 25 ms | 16 of 200 | 16 of 200 | **not met** |
| no preset load over 50 ms | 10 of 200 | 11 of 200 | **not met** |
| worst single load | 155.7 | 319.7 | (d) had one 320 ms outlier |

The next section is why, with the measurements that show each residual is not
the transpile. The final build's own warm run, tabled above, reads 7.2 / 89.8
ms for the load p50 / p99, 95.5 ms for the switch p99, 118.5 ms for the worst
frame, 55.5 fps, and 1.26x for the fade ratio (the closest to 1.25x of any
run, and not a change in the fade's cost: the ratio moves with the steady p99,
which is noise on this laptop).

## What is left

- **ns-eel compile and init code** on the render thread: up to 28 ms for a few
  presets, 11 ms at the p99 of loads. Not cacheable from outside projectm-eval.
- **The fade**: two presets' per-frame CPU, 35 ms at the p99 frame. Only a
  parallel or a visually different fade changes it.
- **The first visit to a preset**: a cold load is still a full transpile
  (median 118 ms here), and that first switch is what the slow list learns
  from (below).
- **One texture-folder scan per process** on the render thread, at the first
  preset that asks for a texture (14.7 ms here with 10,184 presets in the
  folder).

## Checks

- `cargo fmt --check`, `cargo clippy --lib -- -D warnings` and
  `cargo test --lib milkdrop` (88 tests, two of them new) pass on
  `wp8-stutter-fixes` with the final pin.
- The cache's GoogleTest tests (`tests/libprojectM/ShaderTranspileCacheTest.cpp`
  in the fork, six tests) pass. `projectm-sys` builds libprojectM with
  `BUILD_TESTING=OFF`, so they were compiled on their own with MSVC (`/W4`, no
  warnings) against vcpkg's `gtest:x64-windows-static-md`. The LRU test was
  checked against a deliberately broken build (a hit no longer refreshing the
  entry): it fails there ("the entry that was just used was pruned") and
  passes on the real code.
- Verify mode, the damaged cache and two processes on one directory: above.
- `.github/scripts/check-dependency-sources.py` fails on this branch, as it
  already did on `wp7-transpile-cache`: it only accepts `github.com/crmne/`,
  and the fork chain lives under `github.com/Errantial/`, which this work may
  not push to crmne. That needs the maintainer.

## What was not compiled

Only Windows (MSVC 19.51, x64, Ninja, vcpkg) was compiled and run; macOS and
Linux were not compiled. The WP8 C++ is written to be portable:

- The cache's worker uses `std::thread`, `std::mutex`,
  `std::condition_variable`, `std::deque`, `std::unordered_map` and
  `std::filesystem` (`last_write_time` with a value, `directory_iterator` with
  `increment(error_code)`), all C++17; the reject entries call `glGetString`,
  available on GL 3.3 and GLES 3. `_getpid` stays behind `#if defined(_WIN32)`.
- The texture rescan uses `std::async(std::launch::async, ...)` and
  `std::future`, C++11, in a C++14 target; a failure to start the thread falls
  back to the old synchronous scan.
- The test sets the environment with `_putenv_s` on Windows and
  `setenv`/`unsetenv` elsewhere.
- The read-back after a write is done on every platform; it is there for
  Windows' antivirus and costs a read of a file that is in the page cache
  elsewhere.

## Risks

- **A libprojectM change without a version bump.** As in WP7: a change to the
  generator that leaves `GetGlslGeneratorVersion()` alone keeps stale entries.
  Bump `ShaderTranspileCache::CacheFormatVersion` in any release that changes
  the transpiler. The reject entries depend on the same key, plus the driver.
- **A reject entry for a shader that would compile now.** Only possible if the
  driver strings stay the same while the driver's verdict changes, or if the
  key misses an input. Verify mode re-checks every reject entry and found
  none; deleting `*.reject` (or the directory) clears them.
- **The hit read is still on the render thread.** 0.54 ms at the median,
  1.1 ms worst, once the entry has been opened once. An entry written by a
  process that died before its read-back, or copied in from elsewhere, still
  costs the first-open scan once (about 7 ms on this laptop).
- **The worker at exit.** Queued writes get 2 s; a worker stuck on a slow disk
  after that is detached and its state leaked, so the process can exit. A
  libprojectM built as a Windows DLL and unloaded from `DllMain` would find
  the worker already killed by `ExitProcess`; the wait then ends after 2.5 s.
  Spotifast links libprojectM statically.
- **Two processes and the budget.** Each prunes to its own view of the
  directory; together they can exceed the budget by what the other wrote
  since its last scan, until either scans again.
- **The texture list is one preset old.** A texture file copied into the
  preset folder is found from the preset load after the background scan that
  saw it, not at the very next load as before. Removing a file that a stale
  list still names falls back to the placeholder texture, as a missing file
  always did.
- **Disk use.** 256 MiB at most; the full library takes 35.1 MB for all 10,184 presets of this folder
  (scaling test).

## What a human should check in the app

The harness drives `Engine` and the child's frame rules, but a person should
look at the real window:

1. Open MilkDrop on a folder with presets the app has already shown and press
   Next a few times. A switch to a preset seen before should be instant;
   watch the first preset that uses a texture after a restart (one scan).
2. Find one of the six presets in "Reject entries, measured" (for example
   `ORB - Stahl - Fire Worm` or `suksma - water + god roam3`) and compare it
   with `PROJECTM_SHADER_CACHE=0`: the picture should be the same, because
   the same fallback shader is used. The app's log shows `Skipping composite
   shader this driver rejected before` where it used to show the driver's
   compile error.
3. Watch a few fades for any visible difference from `wp7-transpile-cache`,
   and press Next during a fade: the picture should not drop to half size
   because of that press.
4. Presets with `sampler_` textures and `rand` textures should show the same
   textures as before. Add a new image to the preset folder's `textures`
   directory while MilkDrop runs and check that a preset using it finds it
   from the second preset change on.
5. `%LOCALAPPDATA%\projectm\shader-cache` should hold `.glsl` and a few
   `.reject` files, and its size should stay under 256 MiB.
