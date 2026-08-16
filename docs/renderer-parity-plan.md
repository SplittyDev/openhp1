# Original renderer parity plan

This document tracks the exhaustive review of the shipped HP1 renderer and the
work needed to reproduce it in OpenHP1. The feature-level evidence and status
matrix lives in [`original-renderer-parity-audit.md`](original-renderer-parity-audit.md).

## Target

- Classic reproduces every observable rendering behavior used by the shipped
  game, including output ordering, material semantics, lighting, animation,
  visibility, scene effects, camera-dependent rendering, and display behavior.
- Modern supports the same base feature set and semantics before its explicit
  HDR and post-processing layers. Those layers may improve output, but must not
  silently remove or reinterpret an original feature.
- A feature is complete only when its original behavior is established from a
  shipped primary source, both renderer paths are audited, an automated check
  protects non-trivial logic, and the relevant visual behavior is replayed.

## Evidence rules

Use evidence in this order:

1. Shipped `Render.dll`, represented locally by `res/Ghidra_Render.c`.
2. Shipped `Engine.dll`, represented locally by `res/Ghidra_Engine.c`, when the
   renderer consumes engine-owned state or calls an engine virtual/native.
3. Compiled properties, bytecode, assets, maps, and configuration from the
   legally obtained local installation.
4. Captured retail behavior for output that static evidence does not fully
   determine.
5. Licensed reference engines only after shipped evidence is exhausted.

Decompiler output is evidence of control flow and data use, not authoritative
names or types. Every behavioral claim must cite a function address/symbol and
line range, plus any package/config locator needed to reproduce it. No feature
is implemented from a guessed heuristic or a map-specific exception.

## Coverage method

- [ ] Enumerate every function and global in `Ghidra_Render.c`; classify each
      as behavior, setup/teardown, container/compiler artifact, or unresolved.
- [ ] Trace every imported engine call and renderer-facing engine structure
      needed to interpret observable behavior.
- [ ] Trace every render-device capability, flag, enum, and configuration
      branch used by the shipped renderer.
- [ ] Map each observable behavior to package/map/script inputs that exercise
      it, including negative and fallback paths.
- [ ] Map each behavior to its OpenHP1 decode, scene, runtime-update, batching,
      Classic pipeline, and Modern pipeline owners.
- [ ] Record unsupported, partial, divergent, and unverified behavior as
      unchecked tasks in the audit matrix.
- [ ] Reconcile the inventory against `docs/renderer.md`, texture/mesh/map
      documentation, existing tests, and prior renderer commits.
- [ ] Repeat the inventory pass until every decompiled function/global and
      every renderer-facing Engine.dll dependency is classified.

The audit is exhaustive by traceable inventory, not by claiming that a large
source file was merely read end to end.

## Status vocabulary

| Status | Meaning |
| --- | --- |
| `unknown` | Not audited yet. |
| `unresolved` | Evidence is insufficient to state the original semantics. |
| `missing` | Original behavior is established and OpenHP1 has no implementation. |
| `partial` | Some required paths or cases are absent. |
| `divergent` | Implemented behavior conflicts with shipped evidence. |
| `implemented` | Code appears to cover the established behavior. |
| `verified` | Focused automated and executable checks pass. |
| `live-confirmed` | Relevant retail/OpenHP1 visual comparison is accepted. |
| `not-applicable` | Internal mechanism or obsolete device/editor surface is inventory-only; its observable effect is tracked by another row. |

`implemented` is not interchangeable with `live-confirmed`.
`not-applicable` must name the observable replacement row; it cannot hide a
game-visible behavior.

## Audit phases

### 1. Establish the source inventory

- [ ] Record the `Ghidra_Render.c` function/global inventory and classification.
- [ ] Record renderer-facing Engine.dll dependencies.
- [ ] Record renderer/device configuration and capability branches.
- [ ] Record representative shipped assets/maps for each behavior.

### 2. Audit original feature families

- [ ] Frame lifecycle, viewport clearing, locking, flushing, and presentation.
- [ ] Camera projection, clipping, frustum, depth, precision, and screen bounds.
- [ ] BSP traversal, zones, portals, occlusion, sky zones, mirrors, and warps.
- [ ] Draw ordering, batching boundaries, depth writes/tests, and culling.
- [ ] Opaque, masked, translucent, modulated, additive, environment, two-sided,
      unlit, invisible, fake-backdrop, and other material/poly-flag semantics.
- [ ] Texture addressing, filtering, mip selection, palettes, animation,
      realtime procedural textures, detail/macro maps, and texture panning.
- [ ] Lightmaps, vertex lighting, dynamic lights, light effects, visibility
      masks, dark lights, fog, coronas, and other scene effects.
- [ ] Mesh, sprite, particle, decal, mover, skeletal, and vertex-animation
      rendering, including per-instance state updates.
- [ ] Color depth, gamma/brightness, flash/fade, selection/hit testing, debug
      modes, screenshots, and renderer statistics that affect behavior.
- [ ] Resource loss/recreation, resize, fullscreen/windowed transitions, and
      all hardware/device fallback branches used by the game.

### 3. Audit OpenHP1 implementations

- [ ] Complete the Classic column for every inventory row.
- [ ] Complete the Modern column for every inventory row.
- [ ] Confirm shared scene/batching code preserves semantics for both paths.
- [ ] Label every Modern-only HDR/post effect separately from base parity.
- [ ] Convert every confirmed gap into a checkbox with evidence, owner, test,
      representative map/asset, and acceptance criteria.

### 4. Implement confirmed gaps

- [ ] Prioritize shared data-loss and semantic gaps before backend-only fixes.
- [ ] Assign one independent feature to one branch/worktree.
- [ ] Require the worker to cite original evidence before editing engine logic.
- [ ] Add the smallest focused automated regression check.
- [ ] Run focused formatting, nextest, check, corpus, and executable validation.
- [ ] Review the diff against evidence and repository standards.
- [ ] Commit exactly one logical renderer feature or correction.
- [ ] Update the audit row with commit, checks, and remaining live status.

### 5. Close parity

- [ ] Every decompiled renderer function/global is classified.
- [ ] Every observable original feature has a closed audit row.
- [ ] Classic has no `missing`, `partial`, `divergent`, or `unknown` rows.
- [ ] Modern has no base-feature `missing`, `partial`, `divergent`, or
      `unknown` rows.
- [ ] Representative corpus maps cover every feature family.
- [ ] Automated checks pass without requiring the copyrighted corpus in CI.
- [ ] Local corpus scans and both real renderer modes pass on Metal/wgpu.
- [ ] Side-by-side retail/Classic and Classic/Modern base comparisons are
      recorded for every visually observable feature family.
- [ ] Remaining differences are explicitly approved Modern enhancements.

## Per-feature workflow

Each issue follows this sequence:

1. Pin original evidence and a representative shipped input.
2. Trace decode -> scene state -> runtime updates -> draw ordering -> shader or
   output consumer.
3. Identify the narrowest shared responsible seam.
4. Create a feature branch/worktree; do not combine unrelated findings.
5. Implement and leave one focused regression check for non-trivial behavior.
6. Run `cargo fmt --all`, focused `cargo nextest run`, and focused
   `cargo check`, followed by the relevant local corpus/executable check.
7. Review the complete diff for both source fidelity and repository standards.
8. Commit the single feature, then record its SHA and verification boundary.
9. Keep visual acceptance unchecked until the relevant scene is actually
   compared in Classic and Modern.

## Current work queue

- [ ] Complete the original Render.dll/Engine.dll behavior inventory.
- [ ] Complete the Classic coverage audit.
- [ ] Complete the Modern coverage audit.
- [ ] Reconcile all three passes into the feature matrix.
- [ ] Triage the first confirmed gaps by shared impact and evidence strength.

No parity fix should begin from this queue alone; the corresponding audit row
must first contain sufficient original evidence and concrete acceptance criteria.

## Confirmed issue backlog

This is the implementation queue discovered by the initial code/evidence pass.
The audit matrix owns the detailed semantics, code mapping, tests, corpus input,
commit, and live-verification fields for each item.

- [ ] `MOD-001` Stop decoding the already-linear Modern sky render target from
      sRGB a second time when projecting fake backdrops. Evidence:
      `Ghidra_Render.c:2323-3156`; current divergence: `scene.wgsl:204`.
  - [x] Implemented in `a746a30` with a focused shader invariant.
  - [x] `openhp1-render` nextest (65 passed, 4 skipped), focused check, format,
        and diff check passed without the copyrighted corpus.
  - [ ] Compare a representative fake-backdrop scene in Modern and retail;
        keep the parent open until visual acceptance.
- [ ] `BASE-001` Honor `PF_Unlit` for translucent, modulated, masked-blended,
      and actor-opacity materials in Classic and Modern. Evidence:
      `Ghidra_Render.c:17988`; current divergence: `pipeline.rs:164-194` and
      `scene.wgsl:130-154,188-201`.
  - [x] Implemented in `5ea6465` with fragment-selection coverage for all
        blended modes, masking states, and both renderer modes.
  - [x] `openhp1-render` nextest (66 passed, 4 skipped), focused check, format,
        and diff check passed without the copyrighted corpus.
  - [ ] Replay representative unlit blended content in Classic and Modern;
        keep the parent open until visual acceptance.
- [ ] `BASE-002` Decode, clip, update, and render decals, including shipped
      scorch, ecto-mark, and decal-shadow actors. Evidence:
      `Ghidra_Render.c:2219-2818,4204-4267` and
      `Ghidra_Engine.c:82088`.
- [ ] `BASE-003` Implement original surface fog independently of optional
      Modern volumetric enhancements. Evidence: `Ghidra_Render.c:18112-18256`.
- [ ] `CLASSIC-001` Render authored corona sprites and original volumetric
      lighting in Classic. Evidence: `Ghidra_Render.c:5819-5858,18256,
      21906-21928` and `Ghidra_Engine.c:153257-153281`.
- [ ] `MOD-002` Keep authored corona sprites when Modern volumetric enhancements
      are enabled. Evidence: independent retail sprite and volumetric paths at
      `Ghidra_Render.c:5819-5858,13034-13266`; current divergence:
      `renderer/modern.rs:381-390`.
  - [x] Implemented in `982f259` by removing the Modern-only suppression path.
  - [x] `openhp1-render` nextest (66 passed, 4 skipped), focused check, format,
        and diff check passed without the copyrighted corpus.
  - [ ] Replay an authored corona with Modern volumetrics enabled and disabled;
        keep the parent open until visual acceptance.
- [ ] `MOD-003` Replace Modern corona depth-test visibility with the original
      center-point BSP visibility rule. Evidence must be completed before
      implementation; current approximation: `renderer/modern.rs:599-600`.
- [ ] `BASE-004` Reproduce BSP node/zone/span visibility for world, sky,
      reflection, warp, blended, and dynamic submissions. Evidence:
      `Ghidra_Render.c:1938-2044,6452`; current approximation:
      `renderer/pipeline.rs:126-134`.
- [ ] `BASE-005` Select auto-pan rates from the camera-facing BSP node side and
      the actual Level actor fallback. Evidence: `Ghidra_Render.c:6452` and
      `Ghidra_Engine.c:156900-156911`; current divergence:
      `openhp1-scene/src/loader.rs:3950-3970,4091-4105`.
- [ ] `BASE-006` Implement `PF_SmallWavy` time-varying UV motion. Evidence:
      `Ghidra_Render.c:2520`; current shared UV path: `scene.wgsl:86`.
  - [x] Implemented for Classic and Modern in `840976c` with the exact retail
        U/V formula and raw-texel-to-normalized conversion.
  - [x] `openhp1-map` (12), `openhp1-scene` (57), and `openhp1-render` (67;
        4 skipped) nextest suites, focused checks, format, and diff checks passed
        without the copyrighted corpus.
  - [ ] Locate and replay representative shipped `PF_SmallWavy` surfaces in
        Classic, Modern, and retail; keep the parent open until visual acceptance.
- [ ] `BASE-007` Preserve, upload, and sample retail mip chains and LOD choices.
      Evidence: `Ghidra_Engine.c:28739-28755,65770-65777,121084-121092`;
      current single-level paths: `openhp1-scene/src/loader.rs:4152-4154` and
      `renderer/pipeline.rs:263-270`.
- [ ] `BASE-008` Advance generic `AnimNext` texture chains with their authored
      `PrimeCount`, `MinFrameRate`, and `MaxFrameRate` semantics through the
      shared texture-update path. Direct evidence:
      `Ghidra_Engine.c:36950-37024,69357-69419,97083-97088,124915-124929`,
      `Ghidra_Engine.c:151578-151595`, and `Ghidra_Render.c:13223-13236`.
  - [ ] Decode the four authored properties, resolve chains by stable package
        object identity, and reproduce null fallback, cycles, clamp/range,
        long-delta single-step, accumulator cap, and priming semantics.
  - [ ] Reuse incremental texture uploads for dimension-stable frames; first
        scan all shipped chains and add texture recreation/rebinding only if a
        referenced chain actually changes dimensions.
  - [ ] Tick runtime textures independently of actor-mesh playback; the viewer
        currently returns before `tick_water` when actor animation is idle at
        `openhp1-viewer/src/app.rs:802-810`.
  - [ ] Acceptance: synthetic decode/scheduler/cycle/prime tests; full shipped
        chain/dimension scan; headless proof that the 16-link
        `Hub5_Devils.ground.devilfloor1_128` loop advances and wraps at its
        authored 20/20 rate; retail/Classic/Modern replay in `Lev5_Snare` and
        `Lev2_Inc_A`.
- [ ] `BASE-009` Implement exact shipped FireTexture and IceTexture procedural
      animation through the same changed-texture upload seam. Direct ownership
      evidence is in shipped `Fire.dll`, not Engine.dll or Render.dll.
  - [ ] Decompile `UFireTexture::ConstantTimeTick` (RVA `0x82c0`) and its
        reachable spark movement/redraw helpers. Do not promote the current
        32-step static Fire snapshot to runtime behavior: its formula is not
        proven by the audited Engine/Render sources.
  - [ ] Decompile `UIceTexture::MoveIcePosition` (`0x5b40`), `BlitTexIce`
        (`0x5e90`), `BlitIceTex` (`0x6210`), `ConstantTimeTick` (`0xa340`),
        `Tick` (`0xa4b0`), `RenderIce` (`0xa600`), `Lock` (`0xe560`), and their
        reachable helpers before implementing source/glass compositing,
        panning, displacement, timing, or cache behavior.
  - [ ] Preserve direct/inferred boundaries: Fire's ConstantTimeTick override
        appears to use Engine's generic pacing, while Ice's Tick,
        ConstantTimeTick, and Lock overrides prove additional hook ownership
        but not the native formulas.
  - [ ] Acceptance: exact recovered-state unit tests and incremental-upload
        coverage; Fire comparisons using `Furnace` in `Lev5_Final`,
        `ancflame1` in `Lev5_fluffy`, `lumos1` in `Lev3_DungeonB`, and
        `owlstand1` in `Lev_Tut1`; Ice comparisons using the Snitch/Bludger
        halos selected by shipped class defaults in `Lev2_Quid1`,
        `Lev3_Quid2`, and representative `Quid_*` maps.
- [ ] `CLASSIC-002` Re-evaluate Classic actor and world lighting when actors or
      lights move and when live light properties change. Evidence:
      `Ghidra_Render.c:1938-2013,22100-22606`; current partial projection:
      `openhp1-scene/src/loader.rs:636-697` and
      `loader/runtime_light.rs:4-56`.
- [ ] `BASE-010` Project runtime `Texture`, `MultiSkins`, `bUnlit`, and
      `bMeshEnviroMap` changes into shared scene materials. Evidence:
      `Ghidra_Engine.c:126909-126989` and
      `Ghidra_Render.c:10130-10134,13034-13266`.
- [ ] `BASE-011` Apply retail LodMesh collapse, morph, hysteresis, and
      distance-detail behavior instead of always rendering maximum detail.
      Evidence: `Ghidra_Render.c:10130-10134,31283-31361`; discarded fields:
      `openhp1-mesh/src/geometry.rs:71-125`.

### Evidence still required

- [ ] Audit the original render-device DLL before claiming exact sampler,
      `bNoSmooth`, mip-bias, 16-bit dithering, texture-cache, or fixed-function
      raster-precision parity; Render.dll delegates those operations.
- [ ] Complete the cited shipped `Fire.dll` decompilation for exact
      FireTexture/IceTexture simulation,
      special-lit actors, lens flares, Fatness/Wideness, specular glow, and
      runtime LOD bias before implementing them.
- [ ] Establish exact translucent span ordering and mirror/warp recursion
      termination with targeted retail traces where static evidence is
      insufficient.
