# MilkDrop smooth-transition plan (Errantial/spotifast fork)

Goal: no visible hitch when MilkDrop changes presets, and a crossfade that looks continuous at the screen's refresh rate.
Branch base: `milkdrop-smooth-transitions` (from crmne/spotifast main @ 3563c57). Fork-only; keep upstream rules in AGENTS.md (no new settings, no general refactors, other platforms must keep compiling).

## What the code does today (read before changing anything)
- `src/milkdrop/child.rs` `prepare_frame()` runs on the GL/render thread every frame: `Presets::advance()` -> `Request::Load{path,smooth}` -> `Engine::load()` -> `projectm_load_preset_file(handle, path, smooth)`.
- That call is synchronous on the render thread. It does disk read + .milk parse + HLSL->GLSL translate + glCompile/Link of warp and composite shaders. For Cream-of-the-Crop presets this is tens to hundreds of ms -> one long frame at every scene change (the stutter).
- `src/milkdrop/engine.rs`: soft cut `CROSSFADE_SECONDS = 2.7`, hard cut off, mesh 48x36, `projectm_set_fps(60)`.
- During a soft cut projectM renders the old and the new preset every frame (about 2x GPU cost for 2.7 s), so a marginal GPU drops frames throughout the fade, not only at the start.
- `build()` sets `SwapInterval::DontWait`; pacing is a software timer in `Child::schedule_next` (`next_frame`), so frame delivery jitter is visible as judder.
- The ring buffer feeds audio per frame (`ring.since(cursor, LAG)`); a long load frame delivers a big PCM burst afterwards.

## Success criteria (measured, not eyeballed)
1. Frame-time telemetry shows, for a fixed 200-preset sample at 60 fps: worst frame within a 1 s window around a preset switch <= 25 ms on the reference machine (framework or laptop, Windows 11), and p99 frame time during a crossfade <= 1.25x the steady-state p99.
2. No preset load blocks the render thread longer than 8 ms (everything slower moves off-thread or is spread over frames), or the preset is skipped/flagged.
3. The crossfade progresses by wall-clock time, so a slow frame never makes the fade jump.
4. Existing tests pass: `cargo test --lib milkdrop`, `cargo clippy`, `cargo fmt --check`, and Windows/macOS/Linux `cfg` blocks still compile.

## Work packages
Agents work in separate git worktrees off `milkdrop-smooth-transitions` and report numbers with every claim.

- **WP1 Measure (first, everyone depends on it).** Add frame-time and load-time instrumentation to the child process: per-frame duration ring, `load()` duration, and a summary line on exit/preset switch, behind `cfg(debug_assertions)` or an env var read only in the child (`SPOTIFAST_MILKDROP_TRACE`), no new user setting. Add a repro harness under `examples/` (or a test binary) that cycles N presets from a folder at a short interval with a synthetic audio signal and prints stall stats. Output: `docs/` NOT touched; write results to `PERF_NOTES.md` at the repo root. Also record which presets have the slowest loads (top 20).
- **WP2 Take disk/parse off the render thread.** A worker thread reads the *next* preset's bytes ahead of time (decide next at switch time minus lead), and the render thread calls `projectm_load_preset_data` with the prefetched string if projectm-sys exposes it. Includes `Presets` changes so the next pick is known one turn ahead (history and Previous/Next must still behave exactly as the existing tests require). Measure the gain separately from WP3.
- **WP3 Shader-compile hitch.** Investigate with WP1 numbers how much of the load is shader work. Options in order of cost: (a) per-preset load-time budget with a skip/blocklist cache (presets that take > budget are skipped for auto-advance, still reachable manually); (b) spread the work: load only at a moment the app chooses (e.g. right after a vsync boundary with a short fade-in frames skipped) and hold audio feed continuity so no PCM burst; (c) research-only spike: a second projectM instance on a worker thread with a shared GL context, rendering the next preset into an FBO so compile happens off the main thread, then crossfading the two textures in our own blend pass (we already have `copy` FBO / offscreen path). (c) is a design note + prototype branch, not a merge, unless (a)+(b) fall short of the criteria.
- **WP4 Crossfade cost and pacing.** (a) Time-based fade verification: confirm projectM's soft cut uses the elapsed clock; make sure our `fps`/`next_frame` pacing cannot stall it. (b) Adaptive quality during the fade: if the frame time exceeds budget during a crossfade, step the internal render `scale` or mesh size down for the fade and restore after, with hysteresis (no pumping). (c) Frame pacing review: compare `DontWait`+timer against swap-interval 1 when the chosen fps equals the screen rate on Windows/macOS only; do not touch Wayland pacing (AGENTS.md, egui fork note) — report results, change only if clearly better. (d) Consider easing the fade length (2.7 s) only if the numbers say a different duration looks smoother; do not change without a measurement.
- **WP5 Integration + review.** One agent merges WP2-4 in order, reruns WP1's harness for before/after, runs fmt/clippy/tests, and writes a short summary. Review with the Interrogate skill before any PR upstream. Visual verification (does it *look* smooth) is a human step on a machine with a display; Linux server1 has no GPU, so llvmpipe numbers are relative only.

## Order
WP1 -> (WP2 || WP4a/b) -> WP3 -> WP4c/d -> WP5. WP1 lands first on the shared branch; the others rebase onto it.

## Environment notes
- Repo: `~/projects/spotifast` on server1 (Linux arm64, ~8 GB free disk at plan time: use `CARGO_TARGET_DIR` shared across worktrees e.g. `~/cargo-target/spotifast`, never one target dir per worktree, and run `cargo clean -p spotifast` rather than a full clean). `cargo build` compiles libprojectM via CMake: expect a long first build.
- Real GPU measurements need framework (DESKTOP-KC5CHJ8) or the laptop (Windows 11). Windows builds need vcpkg GLEW (see build.rs, docs build-from-source).
- Agents: OpenCode provider, model `opencode-go/deepseek-v4.1-flash`.
