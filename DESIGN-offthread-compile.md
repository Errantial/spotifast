# Design: compiling the next preset's shaders off the render thread

Status: research spike for WP3(c). **Not implemented and not prototyped.** It
is a design note, not a merge candidate. The cheap options (WP3 (a) switch
budget and skip list, (b) when the load happens) come first; this is what a
complete fix would look like if their numbers do not satisfy the plan.

Everything below was checked against the pinned versions in this checkout:

- `projectm-sys` 1.2.3, from `crmne/projectm-rs` revision
  `93852ff7299e5a63ed1315437593fc70915c0863`. Its vendored `libprojectM` is
  `crmne/projectm` revision `ffb109857b9d31d71eef7df44f6e34b5a788e556`, the
  `spotifast` branch, which reports itself as version 4.1.0 but carries the
  4.2 OpenGL sharing entry points (see below).
- `glutin` 0.32 and `winit` 0.30 (the crates the MilkDrop child already uses
  to make its window and context).
- The headers were read from the submodule, and the generated bindings use
  `bindgen` with `allowlist_function("projectm_.*")`, so every function named
  here has a Rust binding.

## The problem this would solve

A preset switch costs one long frame. WP1 split it into two parts and WP3's
harness now times them apart:

- `Engine::load` (read, parse, HLSL to GLSL translation) returns in a few
  milliseconds on a real machine and about 10 ms warm on llvmpipe.
- The **first draw of the new preset** is where the driver compiles and links
  the warp and composite shaders. On llvmpipe that is roughly 80 ms warm, and
  it dominates the stall.

OpenGL has no way to compile shaders for a context that is not current on the
calling thread. `projectm_load_preset_data` (WP2) only moves the file read;
the compile still happens inside the load call on the render thread. Neither
of those can remove the first-draw cost while projectM's only instance lives
on the window's thread. The only way to do the compile somewhere else is to
give projectM a second instance on a second thread with a context that shares
objects with the window's context.

## Shape of the change

Two projectM instances, ping-ponged, each rendering into its own texture:

```
             main thread (window context)          worker thread (shared context)
 frame N:    render instance A -> texture A
             blend A and B -> window
 frame N+k:                                        load next preset into B
                                                   (parse + compile here)
 switch:     render A -> texture A
             signal B to render its first frame     render B -> texture B
             wait for B's fence (one frame at most)
             blend A and B by wall-clock alpha
 ... until fade ends ...
             A and B swap roles; the idle one becomes the compile target
```

Components:

1. **Second context.** `glutin::context::ContextAttributesBuilder::with_sharing`
   takes any `AsRawContext` (both `NotCurrentContext` and
   `PossiblyCurrentContext` implement it). The child creates the shared
   context on the main thread, where the window context is known, and sends
   the `NotCurrentContext` to the worker. `Display` is documented `Send +
   Sync`, and `NotCurrentContext` is `Send`, so the worker can create its own
   pbuffer surface from the display and config and make the context current
   there. (Surfaces themselves are not clearly `Send`, so the worker should
   create its own from the `Display`, not borrow the window's. On WGL this is
   a `WGL_ARB_pbuffer` surface; on EGL and GLX it is a native pbuffer; if a
   backend cannot make one, a second, never-shown winit window is the
   fallback.)
2. **Second engine.** `Engine::new` becomes a function that can build an
   engine on either thread. projectM initializes one process-wide OpenGL
   resolver on the first `projectm_create()`
   (`projectm_create_with_opengl_load_proc` is documented to ignore its
   loader after that), and both contexts are in the same share group, so the
   second instance can simply call `projectm_create()` with the worker
   context current. Both instances get the same texture paths, mesh, fps,
   aspect and preset duration.
3. **Render target.** Each instance owns a texture and an FBO to render into.
   `projectm_opengl_render_frame_fbo(handle, fbo)` already exists in the
   pinned headers and is already used by the windowless pane mode. The
   worker's FBO is created in the worker's context (FBO container objects are
   **not** shared between contexts; the texture they attach to is), and only
   the texture name is handed to the main thread.
4. **Handover and synchronisation.** The worker emits a `glFenceSync` after
   its frame; the main thread waits with `glWaitSync` (server-side, no CPU
   block) before sampling the texture, or falls back to the previous frame if
   the fence is not signalled yet. A mutex/condvar carries "texture k is
   ready" and "texture k may be written again", so the main thread never
   samples a texture the worker is writing. Two textures per instance, or
   one each plus the two-frame handshake above, are both enough.
5. **Our own fade.** projectM's soft cut is turned off
   (`projectm_set_soft_cut_duration(handle, 0.0)`) and the blend is a small
   glow pass: one fullscreen triangle, two samplers, `mix(a, b, alpha)` with
   `alpha` from wall-clock time. That satisfies the plan's criterion 3 (a
   slow frame cannot make the fade jump) and removes projectM's own
   double-render during a cut only to replace it with ours, so the GPU cost
   during the fade stays about 2x as it is today. Outside a fade it is one
   textured draw, replacing the current copy-and-stretch blits.
6. **Audio.** Both instances are fed the same PCM each frame (two
   `projectm_pcm_add_float` calls instead of one). The incoming instance
   needs the audio of the fade to have its beat detection ready when it takes
   over. This is cheap and stays on the render thread.
7. **Which preset is preloaded.** WP2's `Presets::lookahead` already names
   the next preset a turn ahead. The worker starts loading it when the
   lookahead changes, which gives it the whole preset turn (seconds) to
   compile. When the switch is due, the main thread only has to wait for a
   render, not a compile. A manual Next or Previous cannot be preloaded; for
   those the worker has to compile on demand and the hitch returns. The skip
   list from WP3(a) can also apply to manual choices if the maintainer
   accepts that, but the plan asks for them to stay reachable.

## What sharing means on each platform

`glutin`'s `with_sharing` is the useful part: no raw WGL/GLX/EGL/CGL code is
needed to create the second context. What it does underneath:

- **EGL (Linux, X11 or Wayland):** passes the shared context to
  `eglCreateContext`. Textures, buffers, shaders and programs are shared;
  FBO names, VAOs and all context state are not. Cross-thread use needs
  `eglMakeCurrent` on the worker. Wayland and X11 behave the same here; the
  child's EGL display already comes from glutin-winit.
- **GLX (Linux, X11):** passes the share list to
  `glXCreateContextAttribsARB` (or the legacy `glXCreateContext`). Same
  object rules. Direct rendering is per-context; a pbuffer surface on the
  worker is enough.
- **WGL (Windows):** glutin calls `wglShareLists` between the two contexts.
  Share lists must be established before either context has objects, which
  glutin does at creation time. A context can be current on each thread with
  the same window DC, but the worker renders only to an FBO, so a pbuffer
  surface created with `WGL_ARB_pbuffer` (glutin exposes
  `create_pbuffer_surface`) avoids touching the window's DC at all.
- **CGL (macOS):** glutin uses `NSOpenGLContext initWithFormat:shareContext:`.
  Sharing requires the same pixel format and virtual screen, which glutin
  gets from the same config; all contexts in a share group must stay on one
  GPU, and Apple's automatic graphics switching can move a window between
  GPUs. The child already handles a screen change by resizing the context
  (`follow_screen`); the shared pair has to be rebuilt if the virtual screen
  changes. macOS OpenGL is deprecated, so this path needs the most real
  hardware testing.

## Driver and product risks

- **The share group may serialise compiles anyway.** Several drivers take a
  global lock for a share group, so the worker's compile can still block the
  main thread's next GL call. The hitch would shrink (the main thread is
  blocked at a boundary, not doing the work) but not disappear. Only a
  per-driver measurement answers this. llvmpipe almost certainly serialises,
  so the build host cannot prove the win; it can only prove the plumbing.
- **Two instances cost about 2x video memory and 2x GPU work during a
  fade.** projectM's soft cut already renders two presets during a fade, but
  a second instance also keeps its last preset's textures while idle, and two
  instances each carry their own feedback textures at full size. On an
  integrated GPU or a small window this could matter. Freeing them would mean
  destroying and recreating the idle instance, which reintroduces a compile.
- **projectM is not documented as thread safe.** Two handles are intended to
  be independent, but the shared GL resolver and any global in the preset
  loader have to be verified with a real two-thread prototype. This is the
  first go/no-go test.
- **GLEW on Windows** is initialised once, on the first context; the function
  pointers are then used by both contexts. That works for a share group in
  practice but must be checked on hybrid-GPU machines, where contexts can end
  up on different adapters and sharing fails outright.
- **Manual switches still hitch** unless their target was preloaded (Next
  can be, Previous usually cannot).
- **Fade behaviour changes.** projectM's soft cut is replaced by our blend;
  WP4's fade governor and pacing work would need to be redone against the new
  path, and the "look" of projectM's own transition (which also blends the
  two presets' feedback, not just their final textures) would change
  slightly. Our blend is a cross-fade of final images, which is what the
  current soft cut looks like in practice, but it is not bit-identical.

## Alternatives that do not need a second context

- **Warm the shader cache ahead of time.** Not possible: GL only accepts
  compile/link calls on the thread the context is current on, and there is no
  projectM API to compile a preset without switching to it. Switching to it
  early would put it on screen.
- **Compile during a fade on the same thread.** The 2.7 s fade has frames to
  spare on a fast GPU, but the compile is synchronous, so it would land in
  one of those frames and the hitch would just move inside the fade. It could
  be spread over several frames only by splitting the preset into parts,
  which projectM does not expose.
- **Skip/flag (WP3 a).** The plan's sanctioned cheap answer: the preset never
  becomes a stall because automatic play stops choosing it. It does not help
  a manual choice and it does not make a slow preset playable. It is the
  recommended path unless criterion 1 must also hold for manual switches.

## Effort estimate

For a production-quality change behind the existing engine API:

| Piece | Estimate |
|---|---:|
| Shared context, worker thread, pbuffer surface, one platform (Linux EGL/GLX) | 3 to 5 days |
| Second engine, FBO handover, fences, blend pass, ping-pong, audio to both | 3 to 5 days |
| Replace projectM's soft cut, integrate with WP4 pacing and fade budget | 2 to 3 days |
| Windows WGL port and GLEW/hybrid-GPU testing | 2 to 4 days |
| macOS CGL port, deprecated-GL and screen-switch testing | 2 to 4 days |
| Real-GPU QA across NVIDIA, AMD, Intel, Apple, and a fallback path | 1 to 2 weeks, unpredictable |
| **Total** | **about 4 to 6 focused weeks** |

The uncertainty is concentrated in driver behaviour, not in the Rust work.
The glutin sharing API removes the largest expected chunk (raw per-platform
context creation), which is why this is a plausible project rather than a
research dead end.

## If it is prototyped

Do it on Linux first, in this order, and stop at the first failed gate:

1. Create the shared context and a second `Engine` on a worker; load a preset
   there while the main thread renders. Gate: no GL error, both pictures
   correct.
2. Measure the main thread's longest call while the worker compiles. Gate:
   p99 under 8 ms on a real GPU, and the switch frame under 25 ms.
3. Render both into textures, fence and blend. Gate: fade p99 no worse than
   the current soft cut's fade p99.
4. Run the WP1 harness with the same 200-preset sample and compare switch
   p50/p99 and worst frame. Gate: the plan's criteria 1 and 2.

Only then port to Windows and macOS. Nothing here should be merged on the
strength of llvmpipe numbers: Mesa's software GL is the one driver where
sharing is least likely to help, and its compile times are not a GPU's.
