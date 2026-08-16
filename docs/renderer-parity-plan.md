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
- [x] Complete the initial Classic coverage audit.
- [x] Complete the initial Modern coverage audit.
- [x] Reconcile all three passes into the feature matrix.
- [x] Triage the first confirmed gaps by shared impact and evidence strength.

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
      `Ghidra_Engine.c:156900-156911`; former divergence:
      `openhp1-scene/src/loader.rs:3950-3970,4091-4105`.
  - [x] Implemented in `08680a5`: each BSP vertex carries its node-plane normal
        and both node-zone rates; the active render-pass camera selects Zone0
        or Zone1, with an explicit `Level.Actors(0)` fallback.
  - [x] Focused opposed-normal and missing-zone regressions, combined
        map/scene/render nextest (139 passed, 4 skipped), touched-crate check,
        format, and diff check passed without modifying the copyrighted corpus.
  - [ ] Compare `Lev2_fire1` in retail, Classic, and Modern; keep the parent open
        until camera-side switching and authored rates are visually accepted.
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
  - [x] Decode the four authored properties, resolve chains by stable package
        object identity, and reproduce null fallback, cycles, clamp/range,
        long-delta single-step, accumulator cap, and priming semantics.
  - [x] Reuse incremental texture uploads for dimension-stable frames; first
        scan all shipped chains and add texture recreation/rebinding only if a
        referenced chain actually changes dimensions.
  - [x] Tick runtime textures independently of actor-mesh playback; the viewer
        now ticks and uploads textures before its actor-animation pause gate.
  - [ ] Acceptance: synthetic decode/scheduler/cycle/prime tests; full shipped
        chain/dimension scan; headless proof that the 16-link
        `Hub5_Devils.ground.devilfloor1_128` loop advances and wraps at its
        authored 20/20 rate; retail/Classic/Modern replay in `Lev5_Snare` and
        `Lev2_Inc_A`.
  - [x] Implemented in `ad0ca47`: authored fields, stable chain resolution,
        exact discrete scheduler behavior, actor-independent viewer ticking,
        and the existing changed-index GPU upload seam are shared by Classic
        and Modern.
  - [x] Synthetic decode/scheduler/zero-delta/cycle/prime/scene tests passed;
        focused nextest passed 110 tests with 2 skipped and focused check passed.
        A read-only full-map corpus scan loaded successfully and reported 15
        changed textures across 13 maps; all 172 regular roots were
        dimension-compatible and the Devil's Snare root is a 16-frame 128x128
        cycle.
  - [ ] Replay the animated floor in `Lev5_Snare` and `Lev2_Inc_A` in retail,
        Classic, and Modern; keep the parent open until visual acceptance.
- [ ] `BASE-009A` Implement exact shipped IceTexture animation through the
      changed-texture upload seam as a focused feature/commit before Fire.
      Direct `Fire.dll` evidence is complete for `MoveIcePosition`
      (`0x10505b40-0x10505e4a`), `BlitTexIce` (`0x10505e90-0x10506120`),
      `BlitIceTex` (`0x10506210-0x1050643b`), `ConstantTimeTick`
      (`0x1050a340-0x1050a478`), `Tick` (`0x1050a4b0-0x1050a5c3`),
      `RenderIce` (`0x1050a600-0x1050a6f5`), and `Lock`
      (`0x1050e560-0x1050e5fc`).
  - [ ] Preserve the recovered layout from `GlassTexture/SourceTexture` at
        `0xd8/0xdc` through `ForceRefresh` at `0x118`, including cached prior
        references/positions and the `LocalSource` blit guard.
  - [ ] Implement movement exactly: `MasterCount += 120*dt`,
        `UDisplace -= 2*signed(HorizPanSpeed-128)*dt`, and
        `VDisplace += 2*signed(VertPanSpeed-128)*dt`; apply Linear, Circular,
        Gestation, WavyX, and WavyY using `(Frequency+1)*MasterCount`, amplitude
        `Amplitude+1`, `.0012` sine/cosine frequency (`.0011` only for
        Gestation V), half amplitude for WavyX/Y, and nearest-integer positions.
        Frame-rate-sync uses Engine's base tick plus an exact `1/120` native
        step; the other time method consumes frame `dt`.
  - [ ] For power-of-two masks and rounded `u/v`, implement `MoveIce=0` as
        `D(x,y)=Source((x+u+Glass(x,y))&UMask,(y+v)&VMask)` and `MoveIce=1` as
        `D(x,y)=Source((x+Glass((x+u)&UMask,(y+v)&VMask))&UMask,y)`. Preserve
        unsigned glass samples, unchanged-state suppression, forced refresh,
        source/glass replacement, and the recovered dependency lock/unlock
        calls. Their virtual/device internals remain unresolved but do not
        change the proved pixel equations.
  - [ ] Acceptance: use an 8x8 `S(x,y)=8y+x`, `G(x,y)=x`, `u=1,v=2` fixture
        for both blits; prove a `1/120` step at speeds `129/127` produces
        master `+1` and U/V displacement `-1/60`; cover every panning/time
        mode, cache/force/local-source/lock behavior, and incremental upload.
        Compare `HP_Dungeon.doors.SlydoorICE` (linear 128/100, frequency 11,
        amplitude 44), `HP_FX.Snitch_Halo` (circular 128/128, frequency 20,
        amplitude 95), `BlueFog_01`, and `GreenFog`; replay halo users in
        `Lev2_Quid1`, `Lev3_Quid2`, and representative `Quid_*` maps.
- [ ] `BASE-009B` Implement exact shipped FireTexture animation as a separate
      feature/commit after Ice. Direct evidence covers `ConstantTimeTick`
      (`0x105082c0-0x105083d5`), `AddSpark` (`0x10501130-0x1050196f`),
      close/delete/line/paint/movement/flash helpers (`0x10501a00-0x105025db`),
      all of `RedrawSparks` (`0x105025e0-0x105058a3`), and `PostDrawSparks`
      (`0x10505960-0x105059e3`). Do not retain or promote the current unproved
      32-step warm-up.
  - [ ] Preserve the `0x50c` object layout and exact eight-byte spark records.
        Implement all 29 public `ESpark` values and all internal spawned types,
        covering the complete 44-case `RedrawSparks` switch (`0x00..0x2b`);
        shipped-asset subsets or placeholder cases are not acceptable.
  - [ ] Rebuild all 1,028 render-table entries as
        `clamp(round-to-nearest-even(i/4 + 1 - (255-RenderHeat)/16),0,255)`.
        Apply wrapped non-rising samples `(x,y),(x+1,y),(x-1,y+1),(x,y+1)`;
        for rising, shift the rows to `y+1,y+2`. Pentium/non-Pentium branches
        are optimized equivalents and need only one exact scalar result.
  - [ ] Reproduce RNG state rather than substituting a new generator: seed the
        512-byte table from low bytes of 512 Core `appRand()` results, read a
        little-endian word at `(index+0x80)&0xfc`, advance by four modulo
        `0x100`, XOR the returned source word into the new table slot, and
        retain the index/table across ticks. Permit injected initial state for
        tests because retail's first bytes depend on process-global Core RNG
        history.
  - [ ] Preserve redraw mutation order: reload `NumSparks` so appended sparks
        can execute in the same tick; swap removal causes the replacement to
        wait until the next tick. Preserve Manhattan proximity deletion,
        Bresenham's excluded final endpoint, and star restoration only when the
        saved destination value is below 38.
  - [ ] Acceptance: exhaustive render-table comparison; wrapped 8x8 filters in
        both rising modes; injected RNG table/index sequence; per-case state
        transitions for all 44 cases, including append/delete order; line and
        star boundary tests; changed-texture upload. Compare `FireEng.Fire1`
        and `Torch1`, `HP_FX.General.Furnace` and `Star`, and
        `GreatFire.ancflame1`; replay direct imports in `Lev5_Final`,
        `Lev5_fluffy`, `Lev3_DungeonB`, and `Lev_Tut1` in retail, Classic, and
        Modern. Remaining blockers are only the semantic name of the client
        `+0x54` tick-suppression field and retail-exact initial Core RNG state.
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
- [ ] `BASE-012` Implement original viewport screen flashes/fades through one
      local-player runtime-to-render path shared by Classic and Modern, after
      pinning the remaining native scheduling evidence. Direct evidence is the
      compiled `Engine.u` `PlayerPawn` bytecode (`ClientFlash` export 4319,
      `ClientInstantFlash` 3485, `ClientFadeIn` 4280, `ClientFadeOut` 3466,
      `SetViewFlash` 3402, and `ViewFlash` 4100), Engine draw/config handling at
      `Ghidra_Engine.c:117664-117768,121781-121788`, and shipped
      `D3DDrv.dll` `EndFlash` RVA `0x1087` -> VA `0x10008be0` (SHA-256
      `7683b11647dafe3926eff7d0d055abbe3d728648a19f5f8a613fd03efd151599`).
  - [ ] First pin the native Engine call site, frequency, and ordering for
        `APlayerPawn::eventViewFlash`; `Ghidra_Engine.c:103234-103250` proves
        the wrapper and delta parameter but not when Engine invokes it. Do not
        mark runtime cadence implementation-ready or simply assume placement
        beside `PlayerTick` until this xref is recovered.
  - [ ] Keep native `InterpolationManager` flash ownership unresolved. Its
        shipped `SetStartParameters`/`UpdateCamera` script exports 3646/3650
        contain only `Return; Nothing`; therefore embedded source mentioning
        `FlashScale`, `DesiredFlashScale`, `ScreenFlashScale`, or
        `ScreenFlashFog` is inactive and must not drive implementation. Resolve
        the native writes before adding interpolation-point flash behavior.
  - [ ] Reproduce the proved `PlayerPawn` plane state exactly: client writers
        scale authored RGB by `.001`; `ViewFlash` caps delta at `.1`, advances
        and clamps fade W to `[-1,0]`, combines desired/constant/zone fog with
        one added to W, decays desired by `2d`, interpolates by `10d`, clears
        instant fog each update, and applies the `.981` W and `.019` RGB snaps.
        Treat `FlashFog.W` as the effective scale; a separate active
        `FlashScale` property is not proved in this build.
  - [ ] At the narrow shared seam, expose the owning local player's resulting
        scale/fog through `PlayerView`, pass it to `Renderer::render`, and draw
        one fullscreen pass after Classic output or Modern final composite/AA
        but before game UI/egui. Match D3D's saturated
        `fog + scene*clamp(scale,0,1)` equation using source `ONE`, destination
        `SRC_ALPHA`, including its clamped 8-bit diffuse-color quantization.
        Do not add per-actor scene state or duplicate backend-specific effects.
  - [ ] Parse `WindowsClient.ScreenFlashes` with shipped default true. False
        supplies identity (`scale=1`, zero fog) at draw time while runtime flash
        state continues to advance; it must not reset the player properties.
  - [ ] Deterministic acceptance: synthetic tests for client writers, delta cap,
        desired decay, instant reset, fade clamps/rate clearing, snap thresholds,
        and one dispatch per proved native cadence; config-disabled identity
        without state destruction; 1x1/pure blend cases for identity, black,
        fractional scale plus fog and saturation; and structural coverage that
        both Classic and Modern use the same pass before UI. No copyrighted
        package is required by public tests.
  - [ ] Live acceptance: compare the two authored `ViewFlash` triggers in
        `Lev2_fire1`; the repeated red flashes and `fadeout 2.0` in `Lev5_Final`;
        HUD/console exclusion; `ScreenFlashes` true/false; and matching event
        timing in retail, Classic, and Modern, including a hitch that exercises
        the `.1` delta cap. `Lev3_Lumos` provides additional TriggeredViewFlash
        coverage.

### Evidence still required

- [ ] Audit the original render-device DLL before claiming exact sampler,
      `bNoSmooth`, mip-bias, 16-bit dithering, texture-cache, or fixed-function
      raster-precision parity; Render.dll delegates those operations.
- [x] Decompile shipped `Fire.dll` far enough to recover implementation-ready
      FireTexture and IceTexture simulation; the exact native addresses,
      formulas, state, ordering, and narrow unresolved hooks are recorded in
      `BASE-009A/B` and the renderer audit. This does not resolve unrelated
      special-lit actors, lens flares, Fatness/Wideness, specular glow, or
      runtime LOD bias.
- [ ] Recover the remaining unrelated native owners for special-lit actors,
      lens flares, Fatness/Wideness, specular glow, and runtime LOD bias before
      implementing those features.
- [ ] Establish exact translucent span ordering and mirror/warp recursion
      termination with targeted retail traces where static evidence is
      insufficient.
