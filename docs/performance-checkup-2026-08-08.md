# Performance checkup: 2026-08-08

## Scope and evidence standard

This checkup covers the current engine at commit `37b76cf` (`Render window shafts with matched shadow directions`). It is a source audit, not a new benchmark run. The only numeric results below are from the repository's earlier measured performance commits and are labeled historical. Candidate impact is therefore ranked as an expectation to validate, not as a measured claim.

Primary sources inspected:

- the current game loop, runtime, scene, renderer, audio, physics, and existing `runtime_scan` example;
- current repository history for earlier performance changes;
- the exact locally locked winit 0.30.13 and wgpu 29.0.4 source.

No implementation should be merged on source inspection alone. Each optimization needs an A/B release measurement and a behavior check at its real consumer.

## Executive verdict

The game is refresh-rate capped, not hard-coded to 60 FPS. It continuously requests redraws, but forces FIFO presentation, which waits for display vblank. A 60 Hz display therefore normally produces about 60 presented frames per second, while a 120 Hz display can produce about 120. There is no separate sleep or 60 Hz scheduler (`crates/openhp1-game/src/app.rs:87-89`, `108-120`, `700-710`; local wgpu source `wgpu-types-29.0.4/src/surface.rs:40-58`).

The most promising behavior-preserving work is CPU-side data movement, not a blind switch to uncapped presentation:

| Rank | Candidate | Expected impact | Risk | First measurement |
| --- | --- | --- | --- | --- |
| 1 | Stop cloning full runtime instances for per-frame UI, particle, and weapon snapshots | High when runtime state or particle actor count is large | Low-medium | Time Profiler allocation/copy samples; `runtime_scan` CPU and counters |
| 2 | Propagate dirty vertex ranges and stop repacking/uploading the entire scene for a local visual change | High; this extends an already measured full-upload hotspot | Medium | bytes uploaded/frame, `Renderer::update_vertices` samples, frame p95 |
| 3 | Avoid visiting every actor for physics and lifespan work when almost all are inactive | Potentially high on actor-heavy maps | Medium | runtime-only profile split by `tick_physics` and `tick_lifespans` |
| 4 | Cache or selectively redraw Modern point-shadow work only when its exact inputs change | Potentially very high when Modern volumetrics are GPU-bound | Medium-high | GPU capture and per-pass timestamps |
| 5 | Reduce transparent-sort recomputation and scratch allocations | Medium only on translucent-heavy maps | Low | count blended surfaces and CPU samples before coding |
| 6 | Add correct BSP/zone/frustum visibility | Potentially high on large maps, especially for shadow passes | High | GPU capture showing vertex/raster cost from off-screen geometry |

Candidates 1 and 2 are the best first implementation targets. Candidates 4 and 6 can outperform them on the GPU, but they are not low-risk, low-reading changes and should be attempted only after a GPU capture proves the bottleneck.

## Frame cap and scheduling

### Current behavior

- `Application::window_event` renders on `RedrawRequested` and immediately requests the next redraw (`crates/openhp1-game/src/app.rs:108-120`). The first redraw is requested after creating the window (`app.rs:87-89`).
- The event loop retains winit's default `ControlFlow::Wait`; there is no polling loop or explicit frame timer (`crates/openhp1-game/src/main.rs:50-57`; local winit source `winit-0.30.13/src/event_loop.rs:148-151`).
- The surface explicitly uses `wgpu::PresentMode::Fifo` (`crates/openhp1-game/src/app.rs:700-710`). Wgpu documents FIFO as vblank-queued and says `get_current_texture()` blocks when the queue is full (local wgpu source `wgpu-types-29.0.4/src/surface.rs:40-58`).
- No VSync or frame-limit field exists in `GraphicsSettings` (`crates/openhp1-game/src/app/graphics_settings.rs:50-64`).

### Why a one-line uncap is not behavior-preserving

`wgpu::PresentMode::AutoNoVsync` would choose Immediate, then Mailbox, then FIFO as a portable fallback (local wgpu source `wgpu-types-29.0.4/src/surface.rs:31-38`). That would remove the presentation ceiling on supported systems, but simulation is currently coupled to rendered frames:

- wall-clock delta is computed once per render and clamped to 100 ms (`crates/openhp1-game/src/app.rs:828-833`);
- animation, runtime Tick, particles, touch checks, and camera work run once per rendered frame (`app.rs:853-859`, `1321-1360`, `1364-1455`);
- physics subdivides by a maximum 20 ms step, so 250 FPS produces materially smaller integration and collision steps than 60 FPS (`crates/openhp1-runtime/src/world/physics.rs:13`, `238-253`);
- fast-forward runs 16 complete simulation ticks per rendered frame (`crates/openhp1-game/src/app.rs:834-859`).

Changing only present mode would therefore change Tick call counts, random-call opportunities, collision integration granularity, and 16x fast-forward speed per wall-clock second. Immediate mode can also tear (`wgpu-types-29.0.4/src/surface.rs:78-85`). Do not merge a one-line uncap under the behavior-preserving requirement.

The behavior-safe route is a fixed simulation cadence with presentation decoupled from it, followed by interpolation for smooth rendering. That is a larger change and should not precede the lower-risk hot-path work below. If an opt-in uncapped diagnostic mode is temporarily used for profiling, keep it out of the final behavior claim and compare runtime ticks/actions per wall-clock second.

## Ranked implementation candidates

### 1. Remove full runtime-state clones from per-frame snapshots

#### Evidence

The game asks the runtime for player UI state every displayed frame (`crates/openhp1-game/src/app.rs:878-881`). `player_ui_state` clones the player's entire `InstanceState`, then clones individual stored properties through `instance_property`; this includes walking the 25-slot `WizardCards` array (`crates/openhp1-runtime/src/world/actor/player.rs:99-117`, `134-160`). The returned state is a small copyable value consumed immediately by `GameUi::set_player_state` (`crates/openhp1-game/src/app/ui.rs:1029-1043`).

The same pattern is more expensive for effects:

- `particle_emitters` copies the entire actor-class list every simulation tick, resolves every actor's class, checks class ancestry, clones each ParticleFX instance, and clones many arrays/objects while rebuilding a large `ParticleEmitter` value (`crates/openhp1-runtime/src/world/actor.rs:386-420`, `421-535` and continuing through the constructor).
- `weapon_attachments` copies and scans every actor, clones each Pawn instance, and clones the weapon instance before returning a usually tiny attachment list (`crates/openhp1-runtime/src/world/actor.rs:737-795`).
- the game invokes both queries every runtime update (`crates/openhp1-game/src/app.rs:1405-1434`).

#### Minimal implementation sequence

Keep these as separate commits because their behavior surfaces and measurements differ:

1. Resolve the small set of player UI property IDs first, then borrow the player instance and read values by reference. Do not clone `InstanceState`, `WizardCards`, or card structs. Keep the exact missing/wrong-type errors.
2. Track registered ParticleFX actors at the same registration/state seam that already owns actor classes, iterate only that set, and read emitter properties without cloning the full instance. Preserve deterministic actor ordering if output order reaches particle synchronization.
3. Track Pawns with weapons or cache immutable attachment metadata, while continuing to sample the animated attachment transform every frame. Invalidation must cover actor spawn/destruction, `Weapon`, `ThirdPersonMesh`, and `ThirdPersonScale` changes.

The first commit is the safest and smallest. For particles and weapons, prefer cached membership plus borrowed reads over a general observer/event abstraction.

#### Required check

- Focused tests must prove returned `PlayerUiState`, emitter ordering/content, and attachment ordering/content are byte-for-byte or field-for-field identical before and after mutations.
- Run a release `runtime_scan` on the same map and duration; compare actions, state resumes, animations, transforms, spawns, destroys, and deferred diagnostics, not only time (`crates/openhp1-scene/examples/runtime_scan.rs:34-56`, `192-223`, `248-274`).
- Profile a particle-heavy authored scene; `Lev_Tut1` alone may underrepresent this path.

### 2. Upload only changed vertex ranges and separate dirty domains

#### Evidence

Any animation, transformed runtime action, particle update, weapon pose, or billboard update sets one global `vertices_dirty` flag (`crates/openhp1-game/src/app.rs:1321-1347`, `1412-1452`, `1500-1504`). The flag correctly coalesces multiple simulation iterations into one presentation update (`app.rs:853-883`), but that update still calls `Renderer::update_scene` for the complete scene (`app.rs:1513-1526`).

`Renderer::update_vertices` then:

1. walks every scene vertex;
2. re-resolves its surface material;
3. converts position, color, normal, and environment-map fields;
4. recomputes every blended-surface center;
5. uploads the whole packed vertex array.

See `crates/openhp1-render/src/renderer.rs:546-571` and `renderer/batch.rs:191-205`.

`update_scene` additionally rebuilds and uploads lighting GPU data and updates all Modern effect state after every vertex change (`crates/openhp1-render/src/renderer.rs:574-585`; `renderer/lighting.rs:157-165`; `renderer/modern.rs:346-352`). The shadow update repacks all scene positions into another vector and uploads them (`renderer/modern/volumetric/shadow.rs:328-340`, `537-546`).

This is a continuation of a proven hotspot. Historical commit `48436a1` changed repeated full uploads during 16x fast-forward to one per displayed frame. Its recorded forced-16x sample reduced vertex-upload leaf samples from 243 to 10, `_platform_memmove` samples from 1,019 to 237, and peak footprint from 1.7 GB to 677 MB. Those figures are historical, not re-measured on this commit; the current source still retains one whole-scene update when dirty.

#### Minimal implementation

- Have scene mutations accumulate merged vertex ranges. The owners already know the ranges: animated meshes store `vertices`, particle systems store `vertices`, and actor render records expose their ranges (`crates/openhp1-scene/src/loader.rs:1794-1815`, `1830-1840`; `loader/runtime_display.rs:348-363`).
- Add a renderer range update that repacks and writes only those ranges at byte offsets. Fall back to `reload_scene` only when topology or vertex count changes.
- Recompute blended centers only for blended surfaces whose indices intersect a changed range. Start with all blended centers if intersection bookkeeping costs more than it saves; measure before adding an index.
- Keep dirty domains distinct. Geometry changes must still update Modern shadow geometry; light/corona changes must still update lighting and volumetric sources. Do not simply replace `update_scene` with `update_vertices` and silently leave shadows or real-time lights stale.

#### Required check

- Synthetic renderer test: two disjoint actor ranges, mutate one, verify the untouched packed bytes stay identical and the changed byte offset/length are exact.
- Existing animation, particle, weapon, billboard, light, and Modern renderer tests must pass.
- Live authored replay must cover an animated actor, particles, a weapon, a moving light if present, and Classic plus Modern.
- Instrument converted vertices and uploaded bytes per frame. The success criterion is proportional work for small mutations, with identical screenshots and runtime counters.

### 3. Maintain exact active sets for physics and lifespan work

#### Evidence

Every runtime tick copies and sorts all actor IDs for physics (`crates/openhp1-runtime/src/world/physics.rs:196-203`), resolves each class, removes/reinserts each instance, reads `Physics`, and enters the stepping loop even though `PHYS_NONE` immediately performs no work (`physics.rs:203-234`, `238-253`).

Every tick also copies and sorts all non-destroyed actors for lifespans, resolves every class, looks up `LifeSpan`, and then usually finds no positive work (`crates/openhp1-runtime/src/world/actor/tick.rs:602-629`).

This is structurally O(all actors) work for two usually sparse concerns. Collision queries themselves already have a more appropriate cached broad phase: collision actors are built lazily per tick and sorted by minimum X (`crates/openhp1-runtime/src/world.rs:256-263`; `world/movement.rs:1130-1147`). Do not replace that existing seam.

#### Minimal implementation

- Maintain deterministic active-physics and active-lifespan actor collections.
- Update membership at the shared property-write seam for `Physics` and `LifeSpan`, plus registration, load/restore, spawn, destruction, and native paths. Direct UnrealScript assignment must not bypass invalidation.
- Iterate actors in the same ascending order as today; ordering can affect action and collision behavior.
- Keep `physics_ticked` semantics for `AutonomousPhysics` unchanged.

This candidate is high leverage only if profiling shows these loops are hot. The invalidation proof is the work; do not ship a partial cache that only watches `SetPhysics`.

### 4. Avoid redundant Modern point-shadow rendering when inputs are unchanged

#### Evidence

Modern volumetric lighting supports four point-shadow sources, six cube faces each, at 256x256 (`crates/openhp1-render/src/renderer/modern/volumetric/point_shadow.rs:12-14`, `60-72`). Every frame it:

- copies and sorts all point sources to choose the nearest four (`point_shadow.rs:189-214`, `294-303`);
- writes up to 24 face uniforms (`point_shadow.rs:194-204`);
- starts and draws up to 24 full shadow render passes over the shared shadow-caster geometry (`point_shadow.rs:216-245`).

The shared caster index includes every opaque, non-volumetric triangle (`renderer/modern/volumetric/shadow.rs:548-563`), and its vertex array mirrors all scene positions (`shadow.rs:537-546`). This can be the dominant GPU cost in Modern mode, but only a GPU capture can establish that.

#### Safe optimization boundary

Shadow maps may be reused only while all exact inputs for a slot are unchanged: selected source identity, source position/radius, caster vertex/index data, and relevant renderer resources. Camera movement alone does not change a point source's six view-projection matrices, although it can change which four sources are selected. Invalidate changed slots, not the whole atlas, when practical.

Animated opaque geometry currently participates in the caster data, so treating the world as globally static would change shadows. If ordinary animation invalidates all slots every frame, stop and measure; a static/dynamic caster split is a larger design, not a quick cache.

#### Required check

- GPU capture before and after with Modern volumetrics enabled and four or more eligible lights.
- Test exact invalidation for camera movement within the same selected set, selected-set changes, moving lights, animated opaque casters, topology reload, and resize.
- Screenshot comparison around moving actors and light transitions.

### 5. Reduce transparent sorting work only if it profiles hot

Every main render builds a vector of blended-surface references, sorts it by camera distance, concatenates indices, and rebuilds draw batches (`crates/openhp1-render/src/renderer/batch.rs:157-188`). With a sky zone, the renderer repeats that work for the sky camera (`renderer.rs:759-804`). The comparator recomputes two squared distances for every comparison (`batch.rs:161-167`).

The smallest safe improvement is to compute one distance key per surface before sorting and reuse scratch vectors in `Renderer`. Preserve `total_cmp`, the existing closest-first ordering, and deterministic ties. This is unlikely to move total FPS unless a map has many blended surfaces, so count surfaces and profile first.

### 6. Correct visibility culling is promising but not low-hanging

The shared renderer batches and draws the complete scene; `RenderScene` carries the combined mesh and materials but no render-time per-zone/per-actor visibility structure (`crates/openhp1-scene/src/render.rs:108-122`). The map data already exposes zone visibility and convex-leaf visibility masks (`crates/openhp1-map/src/bsp.rs:73-101`), while an existing renderer comment states that full node/zone visibility traversal is not implemented (`crates/openhp1-render/src/renderer/pipeline.rs:111-127`).

Correct BSP/zone/frustum visibility could reduce vertex and raster work in large maps and multiply the benefit across shadow passes. It also risks visible holes, broken portals, missing actors, and incorrect sky behavior. Treat it as a separate profiled project after the low-risk wins, with authored room-to-room traversal tests. Do not add a guessed distance cutoff or map-specific workaround.

## Explicitly deprioritized micro-optimizations

These are real costs but should not be separate optimization commits without profile evidence:

- `update_audio` allocates and fills all actor positions every displayed frame even though `AudioPlayer::update` uses positions only for active sounds (`crates/openhp1-game/src/app.rs:1459-1475`; `crates/openhp1-audio/src/playback.rs:143-170`). Reusing a scratch vector is easy but unlikely to be a noticeable FPS win.
- Tick and state actor lists allocate and sort each frame (`crates/openhp1-runtime/src/world/actor/tick.rs:25-42`, `91-93`). Replacing maps or caching order is not justified until runtime profiles attribute meaningful time there.
- Command encoder creation, small uniform writes, and the final presentation pass are normal per-frame wgpu work. Do not optimize them by inspection.
- Turning off Modern AO, bloom, SMAA, or volumetrics is a quality setting, not a behavior-preserving engine optimization. Use toggles to isolate GPU costs, not to claim an optimization.

## Measurement protocol

### Baseline matrix

Use one fixed authored replay per row and record the exact commit, map/save, camera path, internal resolution, window size, renderer mode, and settings:

1. Classic, normal play.
2. Classic, 16x fast-forward.
3. Modern defaults, normal play.
4. Modern defaults, 16x fast-forward.
5. Modern with AO/bloom/volumetrics individually disabled for diagnosis only.

Warm for 10 seconds, then capture at least 30 seconds. Report median and p95/p99 frame time rather than only average FPS. The current overlay stores one instantaneous wall-clock interval, including presentation wait (`crates/openhp1-game/src/app.rs:828-833`, `1263-1268`), so it cannot separate CPU, GPU, and vblank time.

### CPU and behavior counters

- Use a release Time Profiler or `/usr/bin/sample` capture on the real game.
- Run `target/release/examples/runtime_scan <same-map> 5` before and after. Its fixed 60 Hz loop and final counters provide a stable runtime comparison (`crates/openhp1-scene/examples/runtime_scan.rs:34-43`, `192-223`, `248-274`).
- Add temporary counters, not a permanent profiling framework: runtime ticks, actions, state resumes, vertices converted, bytes uploaded, particle emitters sampled, physics actors visited/active, and shadow passes.
- Require identical deterministic counters. Then perform an authored live replay because counters do not validate visuals, collision feel, audio, or input.

### GPU

- Capture Classic and Modern separately with the platform GPU debugger.
- Record pass count, duration, vertex count, bandwidth, and the cost of the 24 possible point-shadow passes.
- Use wgpu timestamp queries only if already supported cleanly by the chosen adapter; avoid adding a profiling dependency before native tools show a gap.

### Acceptance rule per commit

Merge only if the candidate shows a material improvement in its target workload with no regression outside noise in the other baseline rows. Keep each root cause in its own commit and revert candidates whose gains are not noticeable. Tests and counters are necessary; authored replay is still required for gameplay, rendering, particles, audio, and UI.

## Recommended stopping point

Implement and measure candidate 1 first, then candidate 2. Profile again from scratch. Proceed to active physics/lifespan sets only if runtime remains CPU-bound, and to shadow caching or visibility only if Modern remains GPU-bound. Stop when the next candidate is no longer prominent in the new profile; do not spend time polishing allocation counts that are absent from p95 frame time.
