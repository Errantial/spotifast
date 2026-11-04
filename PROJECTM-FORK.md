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
| 1 | [Errantial/projectm](https://github.com/Errantial/projectm) (fork of crmne/projectm) | `wp7-shader-cache` | `686dfb969` | `crmne/projectm` `spotifast` plus the preset shader transpile cache |
| 2 | [Errantial/projectm-rs](https://github.com/Errantial/projectm-rs) (fork of crmne/projectm-rs) | `wp7-shader-cache` | `0b1e4ef` | `crmne/projectm-rs` `respect-cmake-generator` (`93852ff`) with `projectm-sys/libprojectM` pointed at link 1 |
| 3 | `Errantial/spotifast` | `wp7-transpile-cache` | see `git log` | `Cargo.toml` pins `projectm-sys` to link 2 |

`Cargo.toml`'s `[patch]` section is what ties Spotifast to the chain:

```toml
projectm-sys = { git = "https://github.com/Errantial/projectm-rs", rev = "0b1e4efe009033edbb41e5b3b26c1af5f1c7e551" }
```

### What link 1 adds on top of the `spotifast` branch

`crmne/projectm`'s `spotifast` branch (`ffb109857`) is upstream `master` plus
a length check on HLSL identifiers, kept until projectM releases it. On top of
that, `wp7-shader-cache` adds:

- `src/libprojectM/MilkdropPreset/ShaderTranspileCache.{hpp,cpp}`: the
  persistent cache, its SHA-256 key and its bounded, atomically written
  directory.
- `src/libprojectM/MilkdropPreset/MilkdropShader.cpp`: the transpile is split
  into a function that takes the declaration block and the preprocessed
  source and returns GLSL, so the result can be looked up and stored; the
  declaration block is built before the transpile so it can be part of the
  key; and `PROJECTM_SHADER_CACHE_DEBUG=1` reports the per-phase split.
- `src/libprojectM/Renderer/Shader.{hpp,cpp}`: `Shader::Reset()`, so a cached
  GLSL that fails to compile can be retried on a clean program object.
- `src/libprojectM/MilkdropPreset/CMakeLists.txt`: the new sources, and
  `cxx_std_17` for this target only (the cache uses `std::filesystem`; the
  rest of libprojectM stays on C++14).

### What link 2 adds

Only the submodule: `.gitmodules` points at `Errantial/projectm` on
`wp7-shader-cache`, and the recorded gitlink is link 1's commit. Its own base
is `crmne/projectm-rs` `respect-cmake-generator` (`93852ff`), which Spotifast
needed before this branch for the CMake generator and the vcpkg triplet the
`projectm-sys` build script chooses.

## Updating the chain

Always work from the oldest link up, and never rewrite a published commit.

1. In a checkout of `Errantial/projectm` on `wp7-shader-cache`, make the
   libprojectM change, build it through Spotifast, commit and push.
2. In a checkout of `Errantial/projectm-rs` on `wp7-shader-cache`:

   ```sh
   git -C projectm-sys/libprojectM fetch origin wp7-shader-cache
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

## Dropping it

The fork exists only until projectM carries the cache itself. Then:

1. Send link 1's commit upstream (or wait for an equivalent), and drop the
   submodule override in link 2 by pointing `projectm-sys/libprojectM` back at
   `crmne/projectm` `spotifast`.
2. When a `projectm-sys` release also has the generator and triplet fixes,
   replace the `Cargo.toml` override with the crates.io version and delete
   this file.

Nothing outside `Cargo.toml`, this file and `TRANSPILE-CACHE.md` refers to
these repositories, and Spotifast itself needs no code change to use or stop
using them.
