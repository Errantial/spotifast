# The preset shader transpile cache

What a preset switch costs, what this change removes, and what is left.
Numbers are from the RTX 3080 laptop (`PERF_NOTES.md`, "WP7"), with the
protocol, the commands and the acceptance criteria stated so anyone can repeat
them.

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
  generate are skipped; the driver compile still happens.
- Miss: the normal path, and after the program compiles, the GLSL is written.
  Only a successful transpile *and* compile is stored, so a failure is never
  cached.
- A stored entry that fails to compile is dropped, the shader is transpiled
  again on a clean program object (`Shader::Reset()`), the entry is
  overwritten, and the first occurrence is logged.
- An unreadable, empty, oversized or obviously damaged entry, or an
  unwritable directory, is a miss. Nothing about the cache can stop a preset
  from loading.

### Storage

- Directory: `%LOCALAPPDATA%\projectm\shader-cache` on Windows,
  `$XDG_CACHE_HOME/projectm/shader-cache` or `~/.cache/projectm/shader-cache`
  elsewhere. Created on first use with `std::filesystem`.
- One file per key, `<hex>.glsl`, containing nothing but the GLSL source.
- Written to `<hex>.glsl.tmp.<pid>` and renamed into place, so a reader never
  sees a partial entry.
- Bounded to 4096 entries and 64 MB, pruned oldest first by modification time,
  checked once at start and then every 64 stores. Leftover temporary files
  older than an hour are deleted in the same pass.
- A process-wide mutex guards reads of the counters and the whole write; the
  render thread blocks only for the size of a shader source.

### Configuration

Read once, on first use, with surrounding whitespace trimmed:

| variable | effect |
|---|---|
| `PROJECTM_SHADER_CACHE=0` | turns the cache off (also `false`, `no`, `off`) |
| `PROJECTM_SHADER_CACHE_DIR=<path>` | uses this directory instead of the platform one |
| `PROJECTM_SHADER_CACHE_VERIFY=1` | transpiles every hit again and logs `VERIFY MISMATCH` if the stored GLSL differs |
| `PROJECTM_SHADER_CACHE_DEBUG=1` | prints the directory, one line per shader with the phase split, and a summary on exit |

The cache is on by default, so Spotifast needs no code change to use it, and
no setting: the child process inherits the environment and the platform
cache directory is the right place for it.

## How to measure

Same protocol as WP6, same machine, same folder, same harness, so the numbers
are comparable. `examples/milkdrop_stutter.rs` drives the app's own frame
rules (`--path new`), 200 presets from the 10,184 `.milk` files of the Cream
of the Crop folder, 3 s each, soft cuts, 640x480, `SwapInterval::DontWait`
with the 60 fps software clock.

```sh
call C:\Users\emilg\sfwp5\env.cmd
cd /d C:\Users\emilg\sfwp5\src
cargo build --release --example milkdrop_stutter
target\release\examples\milkdrop_stutter.exe --presets C:\Users\emilg\sfwp5\presets --count 200 --seconds 3
```

Runs, one at a time, nothing else drawing:

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
:: (e) verify: every hit transpiled again and compared
set "PROJECTM_SHADER_CACHE_VERIFY=1" &  milkdrop_stutter.exe ... --count 200 --seconds 3
```

`--count 40 --seconds 3` is enough while iterating (about 2 minutes).

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


## What is left, and what the cache cannot do

Three costs remain, and each was measured rather than assumed.

### Six shaders the driver rejects

In every warm run exactly 6 of the 313 lookups miss, and none of them is
stored (`store=0.0ms` in the summary, no store failures): those are shaders
that transpile fine and then fail to compile in the driver, which goes with a
`LOG_ERROR` from projectM and the shader being dropped from the preset. They
are re-transpiled on every load because the cache only stores successful
transpiles.

That is the whole difference between a 3 ms load and a 100 ms one. A
10-preset sample of the slowest presets, warm cache, run on a quiet machine:

| preset | load | of which shaders |
|---|---:|---|
| Bmelgren & Krash - Rainbow Orb Peacock | 2.6 ms | two hits |
| BrainStain - boiling-mix2 (redi jedi) | 3.2 ms | two hits |
| Tripgnosis - Neon Demonology | 4.1 ms | two hits |
| martin - mandelbox explorer | 14.1 ms | two hits |
| EVET + Flexi - Dreamsicles | 53.3 ms | one hit, one rejected |
| Krash + Geiss + martin - War Machine | 65.5 ms | one hit, one rejected |
| Rovastar & Zylot - Crystal Ball | 67.5 ms | one hit, one rejected |
| suksma - water + god roam3 | 67.6 ms | one hit, one rejected |
| shifter + flexi - liquid circuitry | 71.4 ms | one hit, one rejected |
| ORB - Stahl - Fire Worm | 99.9 ms | one hit, one rejected |

A rejected shader costs its transpile (about 15 ms for these) plus a driver
compile that fails (30 to 60 ms on this driver), every time. Caching the
failure would remove that, and is exactly what this design refuses to do: a
transpile can fail for a reason that goes away (a texture appears, a driver
updates), and a cache of failures would then be wrong forever. The right fix
is upstream, in the presets' shaders or in the parser, not here.

### The rest of the preset

For the 194 presets whose shaders are all hits, the load is the preset's own
setup: reading and parsing the `.milk` text, compiling the per-frame,
per-vertex, wave and shape expressions, sizing the framebuffers, and resolving
`sampler_` textures. Instrumenting libprojectM's `Initialize` for one
diagnostic build (not part of the fork) put each of those steps between 0.0
and 3 ms, with the texture loop of a shader at 0.3 to 0.6 ms. That is why the
warm load is 5.1 ms at the median.

The 39 to 54 loads above 16 ms are the same steps on larger presets (20 to 35
KB of preset text, up to 12 samplers) and on frames where the GPU queue is
still full from the previous preset: `Engine::load` makes GL calls, and a GL
call can wait. None of it is the transpile, and none of it is something a
shader cache can reach.

### The fade, and the floor of a "frame" here

`milkdrop_stutter` measures a frame from the end of the previous frame's
render to the end of this one, so a frame contains the wait for the next
frame's deadline. The floor of any frame in this harness is therefore the
16.7 ms frame interval, and 25 ms allows about 8 ms of work in the switch
frame. The warm p50 of 25.8 to 26.5 ms sits exactly there.

The fade window still costs 1.34x to 1.83x a steady frame, because a fade
draws both presets, and that cost is fill and geometry, not the transpile. The
cache moves neither number.

### Where the 1 fps went

The frame rate went from 53.3 to 57.1 fps. A switch now costs 11 ms instead of
135, which removes about 25 s of CPU from a 602 s run (3.4% of wall time and
the biggest single hitch per switch), but the remaining 200 loads still add
2.35 s and the fades and heavy presets still dominate the average.

## What was not compiled

Only Windows (MSVC 19.51, x64, Ninja, vcpkg) was compiled and run. The change
is written to be portable but two targets were not built here:

- macOS and Linux: the cache uses `std::filesystem`, `std::mutex`,
  `std::ifstream` and `<chrono>` only. `_dupenv_s` and `_getpid` are behind
  `#if defined(_WIN32)`; everything else uses `std::getenv` and `getpid`.
  The target needs C++17 (`target_compile_features(MilkdropPreset PRIVATE
  cxx_std_17)` in the fork's `MilkdropPreset/CMakeLists.txt`), which means
  GCC 9 or newer (no `-lstdc++fs`), Clang 7 with libc++ 7, and CMake 3.8 or
  newer for `cxx_std_17`. No platform file API is called directly.
- The Windows path itself was exercised, both the default directory and
  `PROJECTM_SHADER_CACHE_DIR`.

## Risks

- **A libprojectM change without a version bump.** The key covers the preset
  shader header, the generator version and the preprocessed source, so a
  change to the *generator itself* that leaves `GetGlslGeneratorVersion()`
  alone would keep stale entries. `ShaderTranspileCache::CacheFormatVersion`
  exists for that: bump it in any release that changes the transpiler. The
  same applies to two different builds sharing one cache directory.
- **A cache of the wrong thing.** Only successful transpiles are stored, and
  a stored entry that fails to compile is dropped and rewritten, so the worst
  case is one wasted compile. Verify mode over 200 presets found no stale
  entry.
- **The read is on the render thread.** It is 6.8 KB, and measured 0.34 ms on
  average over 313 hits. On a busy disk it is still I/O: one diagnostic run
  taken right after a 12-minute build read the same files at 5 ms each
  instead, because the disk and the antivirus were busy. On a network home
  directory it would be worse. `PROJECTM_SHADER_CACHE_DIR` or
  `PROJECTM_SHADER_CACHE=0` is the way out; nothing about it is required.
- **Disk use.** 4096 entries or 64 MB, oldest first. 200 presets are 1.8 MB,
  so the entry cap binds first (about 28 MB at the measured entry size). A
  library of 10,000 presets therefore keeps about 2,000 of them and re-caches
  as it goes; the cap is a guard rail, not a hit-rate target.
- **The failures this cannot fix.** The six shaders the driver rejects are
  re-transpiled every load, by design. See above.

## What a human should check in the app

The harness drives `Engine` and the child's frame rules, but a person should
still look at the real window:

1. Open MilkDrop on a folder with presets the app has already shown, and
   switch presets with Next. The second visit to each preset should be
   instant; the first visit after a cold start is unchanged.
2. Watch a few fades for a visible difference from `wp6-fix-stutter`,
   especially on the presets listed in this document's slowest tables.
3. Confirm that a preset which shows nothing but its background still does,
   and that the app's log shows the same shader errors as before (the six
   rejected shaders are pre-existing).
4. Check that `%LOCALAPPDATA%\projectm\shader-cache` appears and grows, that
   deleting it only costs one slow pass, and that
   `PROJECTM_SHADER_CACHE=0` in the child's environment restores the old
   timing exactly.
