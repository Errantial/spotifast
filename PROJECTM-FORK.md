# The libprojectM fork chain

Spotifast builds MilkDrop with libprojectM. Its preset loader is where a
preset switch spends its time, so this fork carries changes to it that do not
exist upstream yet. The chain has three links, and every link names a branch
and a commit rather than a floating revision.

## Why

`PERF_NOTES.md` (WP6) measured a warm preset load inside libprojectM at 85 to
135 ms of CPU time on this laptop, of which the HLSL preprocessor is 30 to 56
ms per shader and the HLSL parse another 5 to 20 ms. The driver compiles the
resulting GLSL in about 2.7 ms once its own shader cache is warm. The
transpile is therefore the whole cost of switching to a preset the app has
already seen, and skipping it is what the fork's cache does
(`TRANSPILE-CACHE.md`).

## The chain

| link | repository | branch | commit | holds |
|---|---|---|---|---|
| 1 | [Errantial/projectm](https://github.com/Errantial/projectm) (fork of crmne/projectm) | `wp9-fade-eel` | `5749d6a1d` | `crmne/projectm` `spotifast` plus the preset shader transpile cache, the WP8 stutter fixes and the WP9 fade and expression changes |
| 2 | [Errantial/projectm-rs](https://github.com/Errantial/projectm-rs) (fork of crmne/projectm-rs) | `wp9-fade-eel` | `08d2eb9` | `crmne/projectm-rs` `respect-cmake-generator` (`93852ff`) with `projectm-sys/libprojectM` pointed at link 1 |
| 3 | `Errantial/spotifast` | `wp9-fade-eel` | see `git log` | `Cargo.toml` pins `projectm-sys` to link 2 |

The `wp7-shader-cache` branches (link 1 `05178a4f9`, link 2 `8ad31cf`) and
the `wp8-stutter-fixes` branches (link 1 `d3d81668e`, link 2 `ea2d84e`) are
unchanged; each later branch starts from the one before it.

`Cargo.toml`'s `[patch]` section is what ties Spotifast to the chain:

```toml
projectm-sys = { git = "https://github.com/Errantial/projectm-rs", rev = "08d2eb9a46429b056852d9902c513ee99b66865a" }
```

Link 1 carries three WP7 commits on the `spotifast` branch:

| commit | what |
|---|---|
| `686dfb969` | the cache: `ShaderTranspileCache`, the split transpile in `MilkdropShader`, `Shader::Reset`, and `cxx_std_17` for the `MilkdropPreset` target |
| `7bb27c4e5` | keep the cache lock over the whole write, create the directory with one call, read the debug switches before the disable switch |
| `05178a4f9` | log why the cache is disabled, and label the per-hit timing for the driver compile only |

and five WP8 commits on top of them:

| commit | what |
|---|---|
| `3a4cdd4d2` | every cache write, mtime refresh, scan and prune on one background thread per process; a 256 MiB byte budget pruned least recently used first; the prune at start; reject entries in the store |
| `b076327f7` | `MilkdropShader` skips a shader this driver rejected before, straight to the caller's fallback |
| `68a95d7dc` | `tests/libprojectM/ShaderTranspileCacheTest.cpp`: the queue, reject entries, damaged entries and the LRU prune, with no GL context |
| `63a7ba68b` | the writer thread reads each new entry back once, so the antivirus scan of a new file is not paid by the render thread |
| `d3d81668e` | `TextureManager` rescans the texture folders in the background between preset loads instead of on the render thread at every load |

and two WP9 commits on top of those:

| commit | what |
|---|---|
| `a832e3501` | `CustomShape` runs every instance's code first and then uploads all instances' vertices in one buffer upload per frame, drawing each instance from its range with the same state as before |
| `5749d6a1d` | `projectm_prepare_preset_data()`: parse a preset, compile its expression code and run the independent start of its per-frame init code on any thread; `projectm_load_preset_data()` with the same bytes adopts the result (`PreparedPreset`, `InitCodeSplit`, adopt constructors on the five expression contexts, `tests/libprojectM/InitCodeSplitTest.cpp`) |

Link 2 carries five commits: `0b1e4ef` (the submodule change), `ba70411`
(pointer to `7bb27c4e5`), `8ad31cf` (pointer to `05178a4f9`), on
`wp8-stutter-fixes` `ea2d84e` (pointer to `d3d81668e`), and on
`wp9-fade-eel` `08d2eb9` (pointer to `5749d6a1d`); each of the last two also
makes `.gitmodules` follow its branch.

### What link 1 adds on top of the `spotifast` branch

`crmne/projectm`'s `spotifast` branch (`ffb109857`) is upstream `master` plus
a length check on HLSL identifiers, kept until projectM releases it. On top of
that, `wp9-fade-eel` adds (the cache itself is WP7's; WP8 changed it and
added the texture rescans; WP9 added the last three items):

- `src/libprojectM/MilkdropPreset/ShaderTranspileCache.{hpp,cpp}`: the
  persistent cache, its SHA-256 key, its reject entries and its bounded,
  atomically written directory, all written by a background thread.
- `src/libprojectM/MilkdropPreset/MilkdropShader.cpp`: the transpile is split
  into a function that takes the declaration block and the preprocessed
  source and returns GLSL, so the result can be looked up and stored; the
  declaration block is built before the transpile so it can be part of the
  key; `PROJECTM_SHADER_CACHE_DEBUG=1` reports the per-phase split; and a
  shader with a reject entry for this driver throws the usual
  `ShaderException` before it is transpiled.
- `src/libprojectM/Renderer/Shader.{hpp,cpp}`: `Shader::Reset()`, so a cached
  GLSL that fails to compile can be retried on a clean program object.
- `src/libprojectM/MilkdropPreset/CMakeLists.txt`: the new sources, and
  `cxx_std_17` for this target only (the cache uses `std::filesystem`; the
  rest of libprojectM stays on C++14).
- `src/libprojectM/Renderer/TextureManager.{hpp,cpp}`: the texture file list
  survives a preset load and is refreshed by a background scan (`std::async`)
  started at the previous load.
- `tests/libprojectM/ShaderTranspileCacheTest.cpp` and its line in
  `tests/libprojectM/CMakeLists.txt`: GoogleTest tests of the cache.
- `src/libprojectM/MilkdropPreset/CustomShape.{hpp,cpp}`: one vertex upload
  per buffer per frame for all of a shape's instances, instead of one per
  instance.
- `src/libprojectM/MilkdropPreset/PreparedPreset.{hpp,cpp}`,
  `InitCodeSplit.{hpp,cpp}`, the expression contexts, `CustomWaveform`,
  `CustomShape`, `MilkdropPreset`, `Factory`, `PresetState`, `ProjectM` and
  `ProjectMCWrapper.cpp`, and `projectm_prepare_preset_data()` in
  `src/api/include/projectM-4/core.h`: a preset's expression code compiled
  ahead of its switch on the caller's thread. `PresetState` keeps every
  original member at its offset (a custom wave with a large `sep` reads
  before the audio arrays, onto the q variables and registers).
- `tests/libprojectM/InitCodeSplitTest.cpp`: the init code split, run whole
  and split against projectm-eval.

### What link 2 adds

Only the submodule: `.gitmodules` points at `Errantial/projectm` on
`wp9-fade-eel`, and the recorded gitlink is link 1's commit. Its own base
is `crmne/projectm-rs` `respect-cmake-generator` (`93852ff`), which Spotifast
needed before this branch for the CMake generator and the vcpkg triplet the
`projectm-sys` build script chooses.

## Updating the chain

Always work from the oldest link up, and never rewrite a published commit.

1. In a checkout of `Errantial/projectm` on `wp9-fade-eel`, make the
   libprojectM change, build it through Spotifast, commit and push.
2. In a checkout of `Errantial/projectm-rs` on `wp9-fade-eel`:

   ```sh
   git -C projectm-sys/libprojectM fetch origin wp9-fade-eel
   git -C projectm-sys/libprojectM checkout <new projectm commit>
   git add projectm-sys/libprojectM && git commit && git push
   ```

3. In Spotifast, set `projectm-sys`'s `rev` in `Cargo.toml` to the new
   `projectm-rs` commit, then `cargo update -p projectm-sys` (or delete the
   entry in `Cargo.lock`) so Cargo fetches it, and build.

Cargo keeps a git checkout per revision under `~/.cargo/git/checkouts`, so a
new `rev` is a new directory and a from-scratch CMake build; a build that
finishes in seconds built the old revision. When testing a change inside the
existing checkout instead, delete the matching
`target/<profile>/build/projectm-sys-*` directory: Cargo does not notice
edits to a git dependency's C++ sources and will not rebuild them otherwise.

## Verification

`PROJECTM_SHADER_CACHE_DEBUG=1` prints the cache's configuration and one line
per preset shader, and `PROJECTM_SHADER_CACHE_VERIFY=1` transpiles every hit
again and prints `VERIFY MISMATCH` if the cached source differs. Both are off
by default. `TRANSPILE-CACHE.md` records what they showed on this laptop.
The cache's own tests (`ShaderTranspileCacheTest.cpp`) build in libprojectM's
`projectM-unittest` target when `BUILD_TESTING=ON`; `projectm-sys` builds with
it off, so on this laptop they were built and run on their own against
vcpkg's GoogleTest (`TRANSPILE-CACHE.md`, "Checks"). `InitCodeSplitTest.cpp`
was built and run the same way, against the projectm-eval 1.0.5 sources
`projectm-sys` links.

`examples/milkdrop_evalcheck.rs` draws a fixed sample of presets from fixed
audio and a fixed clock and prints a hash of every frame; two builds that
print the same lines drew the same frames. `PERF_NOTES.md` (WP9) records how
the WP9 changes were checked with it.

## Dropping it

The fork exists only until projectM carries the cache and the WP9 changes
itself. Then:

1. Send link 1's commits upstream (or wait for equivalents), and drop the
   submodule override in link 2 by pointing `projectm-sys/libprojectM` back at
   `crmne/projectm` `spotifast`.
2. When a `projectm-sys` release also has the generator and triplet fixes,
   replace the `Cargo.toml` override with the crates.io version and delete
   this file.

Nothing outside `Cargo.toml`, this file and `TRANSPILE-CACHE.md` refers to
these repositories. Spotifast calls one function only the fork has,
`projectm_prepare_preset_data()` (`Engine::prepare_data`, used by
`src/milkdrop/prefetch.rs`); stopping the fork means removing that call.
