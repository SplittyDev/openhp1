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

- [ ] `CLASSIC-003` Move Classic brightness correction to one final
      post-composite transform and use the shipped D3D exponent
      `1/(Brightness*2.5)`. Direct evidence: shipped `D3DDrv.dll` SHA-256
      `7683b11647dafe3926eff7d0d055abbe3d728648a19f5f8a613fd03efd151599`,
      [`Flush`](../res/Ghidra_D3DDrv.c#L3430), RVA `0x10aa` → VA `0x10002e00`.
      D3D installs a final hardware
      gamma ramp with `pow(i/255, exponent)`; OpenHP1 formerly used
      `1/(Brightness*2.0)` inside material shaders before blending
      (`renderer.rs:1715-1717`, `scene.wgsl:370-372`). This is the highest
      confidence independent device gap; preserve Modern's explicit HDR/tone
      pipeline.
  - [x] Deterministic acceptance: `.5 -> .8`, `.6 -> 2/3`, a case that
        distinguishes blend-then-gamma from gamma-then-blend, disabled/neutral
        behavior, and proof that Classic applies the transform once after
        scene blending/flash while Modern remains unchanged. The focused tests
        pass (70 passed, 4 skipped) without the copyrighted corpus.
  - [x] Implemented in this change: Classic scene materials now remain
        uncorrected through blending, then one final fullscreen display pass
        applies `1/(Brightness*2.5)`. The separate screen-flash change owns the
        reserved scene target immediately before this pass; Modern's HDR/tone
        path is unchanged.
  - [ ] Live acceptance: fixed-camera retail/Classic captures at identical
        brightness values, including opaque, translucent, and modulated pixels;
        record OS/display gamma conditions.

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
- [ ] `BASE-002` Reproduce the complete surface-attached decal lifecycle for
      both renderers. Direct evidence: DrawFrame submission
      `Ghidra_Render.c:2754-2855`, ClipDecal `4204-4598`, AttachDecal
      `Ghidra_Engine.c:82088-82834`, DetachDecal `44017-44103`, and coplanar
      node collection `316069-316282`. Active shipped use is actor shadows:
      714 non-null shadow owners across 40 maps. Compiled gameplay contains no
      active ecto/scorch spawn, so do not present those as authored live gates.
  - [x] `BASE-002A` Implement exact runtime BSP attach/detach: backward trace,
        `0x600` auto-pan rejection, USize-only square projection, SurfList and
        saved-node identities, `MultiDecalLevel` grid/upper clamp, unique
        neighbor projection, and strict `abs(dot(normal)) > 0.7` acceptance.
        Runtime surface records and backward detachment are implemented with
        synthetic adjacent coplanar/tilted fixtures. Scene consumption,
        renderer clipping/submission, shadow updates, and live acceptance stay
        in the following milestones; keep `BASE-002` open.
  - [ ] `BASE-002B` Implement renderer-driven ActorShadow updates: owner
        visibility and RendMap gates, stale-frame detach, translucent-owner
        suppression, one-unit move threshold, orientation changes, and exact
        8192-unit bounds/distance behavior. Keep this runtime commit separate.
  - [ ] `BASE-002C` Add the smallest shared scene decal record retaining owner
        surface/node identity and relative corners; keep package resolution and
        runtime policy outside `openhp1-render`.
  - [ ] `BASE-002D` Implement shared exact clipping: one-unit normal bias,
        saved-edge planes and node filtering, intersection/deletion order,
        24-vertex bound, projection clamp, and DrawScale UVs. Test inside,
        partial, outside, maximum, and empty/populated saved-ID cases.
  - [ ] `BASE-002E` Implement exact brightness/style/device state: grayscale
        RGB with zero alpha, ignored Actor.Opacity, normal depth write, Style 3
        `ONE/INVSRCCOLOR`, and Style 4 `DESTCOLOR/SRCCOLOR` with white diffuse.
  - [ ] `BASE-002F` Insert decals immediately after each owning BSP surface,
        preserve consecutive same-texture coalescing and animated replacement,
        and honor `Decals`/mirrored gates. This depends on the backend-neutral
        `BASE-016` submission plan; do not create a global pass.
  - [ ] `BASE-002G` Feed successful clipped submission time back to
        `LastRenderedTime` and verify compiled Scorch timers become sufficient;
        add policy only if shipped bytecode still leaves a proved gap.
  - [ ] Live acceptance: retail/Classic/Modern comparisons in `Lev2_HogFront`,
        `Lev_Tut3`, one Quidditch map, `Lev4_Sneak`, and `startup`; toggle
        `Decals`. Use a controlled spawn for ecto/scorch behavior.
- [ ] `BASE-003` Implement both original fog contracts independently of
      optional Modern volumetric/AO enhancements; do not replace them with one
      generic depth-fog shader.
  - [ ] `BASE-003A` Implement legacy BSP FogMap. Direct evidence:
        `FLightManager::LightAndFog` and exact accumulation
        `Ghidra_Render.c:17903-18228`, fog-light collection/color
        `22540-22568`, DrawFrame surface layout/lifetime `2550-2575,2862-2870`,
        and shipped D3D `DrawComplexSurface` VA `0x10003ac0-0x1000548b`.
    - [ ] Carry the optional light-manager-generated FogMap only on BSP
          surfaces. Preserve `PF_Unlit` suppression and the `0x40000000` gate;
          do not attach it to actor meshes without direct caller proof.
    - [ ] Reproduce `f2=2f`, premultiplied light-color accumulation and alpha
          clamps, FogMap texture-info UVs, and the one full-facet AlphaBlend
          pass after base/macro/light/detail in the shared Classic/Modern path.
    - [ ] Deterministic acceptance: one/two fog lights, clamp boundaries,
          Unlit/gate suppression, independent macro/detail/FogMap toggles, and
          pass-order trace. Modern volumetrics on/off must not change base fog.
  - [ ] `BASE-003B` Recover and implement camera-zone distance fog separately.
        Shipped properties and authored reachability are proved, including
        `bFogZone`, `FogColor`, and `FogDistance`; native FogColor replication
        is at `Ghidra_Engine.c:156927-156950,225743-225766`.
    - [ ] Decode/default-inherit the three inputs and select them from the
          camera's active ZoneInfo/LevelInfo, but do not render until the native
          enable handoff, curve/start/end derivation, distance coordinate,
          color space, actor/translucency/backdrop reach, and blend equation are
          direct evidence.
    - [ ] Live evidence targets: `Lev2_Fire2`, `Lev2_fire1`, `Lev_Tut1`,
          `Lev_Tut3b`, `Lev3_PreDungeon`, and `Lev3_PreTroll`, using fixed
          near/far views and a zone crossing in retail, Classic, and Modern.
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
- [x] `BASE-006` Implement `PF_SmallWavy` time-varying UV motion. Evidence:
      `Ghidra_Render.c:2520`; current shared UV path: `scene.wgsl:86`.
  - [x] Implemented for Classic and Modern in `840976c` with the exact retail
        U/V formula and raw-texel-to-normalized conversion.
  - [x] `openhp1-map` (12), `openhp1-scene` (57), and `openhp1-render` (67;
        4 skipped) nextest suites, focused checks, format, and diff checks passed
        without the copyrighted corpus.
  - [x] A complete 248-package/41-map scan found no BSP `PF_SmallWavy`
        surface, so no authored visual replay exists. Four FireTexture exports
        set the texture property, but DrawFrame tests the raw BSP surface bit;
        synthetic exact-formula coverage is the available acceptance gate.
- [ ] `BASE-007` Preserve, upload, and sample retail mip chains and LOD choices.
      Engine evidence: `Ghidra_Engine.c:28739-28755,65770-65777,121084-121092`.
      Direct D3D device evidence: `InitTextureStageState` RVA `0x10b9` → VA
      `0x1000adf0` selects NONE/POINT/LINEAR mip filtering and LOD bias `-0.5`;
      `SetTexture` RVA `0x10e6` → VA `0x10009a80` selects and uploads supported
      authored levels. Shipped D3D enables mipmapping and disables trilinear,
      so its default is linear min/mag with point selection between mip levels.
      Current single-level paths: `openhp1-scene/src/loader.rs:4246` and
      `openhp1-render/src/renderer.rs:495-502`.
  - [ ] Deterministic acceptance: retain every valid decoded level, upload the
        exact dimensions/rows, bind the complete view, select the shipped
        point-mip policy for Classic, and cover no-mip and trilinear settings
        independently. Live acceptance uses distant and oblique authored walls.
- [ ] `BASE-013` Carry effective `bNoSmooth`/polyflag `0x800` into shared
      material state and select point min/mag filtering for that draw. Direct
      evidence: D3D `SetBlending` RVA `0x104b` → VA `0x100092d0`; the state is
      independent of the mip-chain policy in `BASE-007`.
  - [x] The shared material state ORs the named BSP flag with decoded texture
        `bNoSmooth`; opaque, blended, backdrop, mirror, and portal batches bind
        point min/mag only when that effective flag is set. Classic and Modern
        use the same selection path, while Modern post-processing is unchanged.
  - [x] Synthetic surface-only, texture-only, neither, and both precedence
        checks pass; smooth and NoSmooth uses of one texture remain separate
        batches. Mip filtering remains independently nearest pending `BASE-007`.
  - [x] A read-only full-map scan found representatives in `Lev3_Intro`,
        `Lev3_PreDungeon`, `Lev3_PreTroll`, `Lev3_Troll`, `Lev_Tut1`, and
        `Lev_Tut3b`.
  - [ ] Compare a representative in retail, Classic, and Modern; keep the
        parent open until point min/mag is visually accepted.
- [ ] `BASE-014` Disable hardware back-face culling in both renderers after the
      renderer's CPU/BSP side-admission decision. Direct evidence: D3D `SetRes`
      RVA `0x1064` → VA `0x1000b360` sets `CULLMODE=NONE`, and the shipped DLL
      has no other render-state-22 write. Current shared pipelines back-cull
      non-TwoSided materials (`renderer/pipeline.rs:47-57`). The original
      side-admission behavior belongs in the shared scene/traversal path, not a
      mode-specific device state.
  - [x] Deterministic acceptance: both triangle windings submitted after CPU
        admission reach the rasterizer in Classic and Modern, including the
        reflected and screen-projected pipeline descriptors.
  - [ ] Live acceptance: select a shipped corpus/capture representative that
        actually submits a back-facing primitive and compare both modes.
  - [x] Implemented by setting cull mode to `None` for both ordinary/reflected
        scene pipelines and screen-projected surfaces; two-sided scene metadata
        is preserved for BSP admission.
  - [x] Focused render tests/check, formatting, and diff checks pass without the
        copyrighted corpus.
- [ ] `BASE-015` Implement exact shared mesh environment mapping without
      treating BSP/texture `bEnvironment` or `ShinySurfaces` as the same
      feature. Direct Render.dll evidence: actor flag mapping
      `42390-42426`, Actor → Zone → Level texture fallback `10501-10512`, and
      reflected UV/color equations plus Unlit precedence `23073-23137`.
  - [x] Preserve the retail BSP behavior: `bEnvironment` is a renderer no-op;
        `ShinySurfaces` separately gates translucent reflected child recursion.
  - [x] Resolve Actor → Region.Zone → LevelInfo environment textures. Decode
        texture `DrawScale`, which `UTexture::Lock` copies into base-mip
        `UMult`/`VMult`; normalized GPU sampling reduces the native texel-space
        equation to `(reflected_xy + 1) * 0.5 * DrawScale` without a `255`
        assumption. Initial scene assembly resolves the actor's loaded zone;
        draw-time refresh after runtime texture/flag changes or zone crossings
        remains assigned to `BASE-010`.
  - [x] Reflect normalized view position about the mesh normal, transform by
        the frame axes, apply the exact U/V scale, and replace vertex RGB with
        `pow(max(reflected_z, 0), 0.25)` before the existing Unlit override.
        Carry the native zero diffuse alpha through the D3D-proved
        texture/diffuse alpha modulation, fixed masked test, and opacity blend
        path while keeping opaque non-masked target-alpha coverage for
        Modern's geometry/AO mask.
  - [x] Deterministic acceptance covers non-default dimensions and multiplier,
        reflected frame basis, fourth-root color, blended zero alpha, masked
        discard, opaque non-masked coverage, Actor/Zone/Level fallback,
        environment plus Unlit precedence in Classic and Modern, and the BSP
        no-op regression. A read-only scan
        decoded 3,750 shipped textures, of which 37 have non-default
        `DrawScale`; ordinary mesh byte-UV scaling remains unchanged and is a
        separate parity gap.
  - [x] Reachability boundary: `HPBase.spellEcto` authors the actor-texture
        branch but was cut from the game; a full-package import scan finds no
        shipped user. The corpus also does not author the Zone/Level fallback,
        so `BASE-015` has no honest live acceptance target and remains covered
        by native equations plus synthetic tests.
- [ ] `BASE-016` Preserve retail BSP traversal and dynamic-actor ordering in
      one backend-neutral submission plan shared by Classic and Modern.
      Direct Render.dll evidence: effective-list selection `7140-7253`, mirror
      overlay/fallback save behavior `7662-7895`, list transforms and actor
      plane interleave `2345-2463`, material-key sorter `2115-2144`, and actor
      pass split `2874-2905`.
  - [x] Exact classification is pinned: list 0 mirror overlays are restored to
        traversal order; ordinary list 1 is sorted by the composite
        zone/base/auxiliary-texture object key; `flags & 0x47 != 0` list 2
        remains reverse traversal order. Retail never center-sorts list 2.
  - [ ] Retain BSP node/save order and per-actor geometry records instead of
        flattening those boundaries during scene assembly. Preserve the
        current shared mesh/material data rather than introducing a second
        renderer representation.
  - [ ] Build one camera-dependent command plan with the three surface lists,
        device-path actor insertion points, and traversal-time backdrop,
        portal, and mirror children; consume it unchanged in both modes.
        Decals must remain immediately after their owning surface, while
        coronas remain a distinct post-geometry pass.
  - [ ] Deterministic acceptance: exact flag/list matrix; traversal reversal
        and material-key order; alternating BSP-plane actor insertion for
        mesh, particle, and billboard records; style/opacity split; child-view
        ordering; and a command-trace/pixel fixture in both modes. Live gates:
        `Lev2_HogFront` WetWater, `Lev4_Sneak` masked/opacity actors,
        `Lev5_Chess` opaque baseline, and a corpus-identified Erised map.
- [ ] `BASE-017` Apply `FTextureInfo` dimensions and `DrawScale` to ordinary
      legacy mesh byte UVs without changing the environment-map equation.
      Direct Render.dll evidence: `DrawMesh` and `DrawLodMesh` multiply each
      serialized byte coordinate by `USize * UMult / 256` and
      `VSize * VMult / 256` (`Ghidra_Render.c:10617-10642,16114-16136`);
      `UTexture::Lock` supplies the dimensions and base-mip `DrawScale`
      multipliers (`Ghidra_Engine.c:96945-97058`). Both mesh decoders now use
      the native `/256` conversion and shared actor-mesh assembly applies the
      selected texture's decoded `DrawScale`; environment materials retain
      their separate reflection-coordinate equation.
  - [x] Implemented in the shared Classic/Modern mesh path without changing
        BSP texture coordinates or the environment-map shader equation.
  - [x] Deterministic acceptance: non-square dimensions and non-default
        `DrawScale` reproduce the native `/256` texel coordinates for both
        mesh formats while default-scale fixtures prove the intentional
        one-part-in-256 correction; environment UVs remain unchanged. The
        synthetic regressions cover scale `0.5`, `1`, and `24`, while the
        existing BSP texel-coordinate regression remains unchanged.
  - [ ] Corpus/live acceptance: a read-only scan found 37 non-default
        `DrawScale` textures, including 35 in `Detail.utx` and the reachable
        candidates `HP_Bentemp.benGrassCicle` (`24`) and
        `HP_Sneak.jellybeans01` (`0.5`). Trace their ordinary mesh bindings,
        then compare a confirmed representative in retail, Classic, and
        Modern.
- [ ] `BASE-018` Carry original macro/detail texture attachments through the
      shared BSP material path. Render supplies independent macro `+0x14` and
      detail `+0x18` texture infos; shipped D3D draws macro after base and
      detail after lighting (`Ghidra_D3DDrv.c:6496-7575`).
  - [x] Exact detail bands are recovered: three iterations start at eye Z
        `380`; after each band the threshold is multiplied by `0.23679848` and
        the texture-coordinate fade multiplier by `4.223`. Included vertices
        use alpha `clamp(round((threshold / eye_z - 1) * 100), 0, 255)`, with
        clipped boundary vertices inserted at each threshold
        ([D:7184-7326](../res/Ghidra_D3DDrv.c#L7184)).
  - [x] Detail is suppressed when the device detail capability is disabled or
        a FogMap is also attached; shipped D3D defaults disable
        `DetailTextures`, so its default visible path does not draw this pass.
  - [x] A read-only scan of 3,751 exports whose class name ends in `Texture`
        (including the one unsupported `WaveTexture`) found 24 non-null
        `DetailTexture` properties: 11 in `FractalFX.utx`, one self-reference
        in `HP_C.utx`, and 12 in `Liquids.utx`. It found no non-null
        `MacroTexture`, and no other shipped package or map imports any of the
        24 owning textures; there is therefore no shipped live representative.
  - [ ] Decode and preserve both object references, then implement the shared
        attachments without coupling either one to FogMap or Modern effects.
        Use synthetic pass-order/band checks because the shipped corpus cannot
        provide a live gate; retain the D3D default-disabled policy separately.
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
      Direct [`Fire.dll`](../res/Ghidra_Fire.c) evidence is complete for
      `MoveIcePosition`
      (`0x10505b40-0x10505e4a`), `BlitTexIce` (`0x10505e90-0x10506120`),
      `BlitIceTex` (`0x10506210-0x1050643b`), `ConstantTimeTick`
      (`0x1050a340-0x1050a478`), `Tick` (`0x1050a4b0-0x1050a5c3`),
      `RenderIce` (`0x1050a600-0x1050a6f5`), and `Lock`
      (`0x1050e560-0x1050e5fc`).
  - [x] Preserve the recovered layout from `GlassTexture/SourceTexture` at
        `0xd8/0xdc` through `ForceRefresh` at `0x118`, including cached prior
        references/positions and the `LocalSource` blit guard.
  - [x] Implement movement exactly: `MasterCount += 120*dt`,
        `UDisplace -= 2*signed(HorizPanSpeed-128)*dt`, and
        `VDisplace += 2*signed(VertPanSpeed-128)*dt`; apply Linear, Circular,
        Gestation, WavyX, and WavyY using `(Frequency+1)*MasterCount`, amplitude
        `Amplitude+1`, `.0012` sine/cosine frequency (`.0011` only for
        Gestation V), half amplitude for WavyX/Y, and nearest-integer positions.
        Frame-rate-sync uses Engine's base tick plus an exact `1/120` native
        step; the other time method consumes frame `dt`.
  - [x] For power-of-two masks and rounded `u/v`, implement `MoveIce=0` as
        `D(x,y)=Source((x+u+Glass(x,y))&UMask,(y+v)&VMask)` and `MoveIce=1` as
        `D(x,y)=Source((x+Glass((x+u)&UMask,(y+v)&VMask))&UMask,y)`. Preserve
        unsigned glass samples, unchanged-state suppression, forced refresh,
        source/glass replacement, and the recovered dependency lock/unlock
        calls. Their virtual/device internals remain unresolved but do not
        change the proved pixel equations.
  - [x] Deterministic acceptance: use an 8x8 `S(x,y)=8y+x`, `G(x,y)=x`, `u=1,v=2` fixture
        for both blits; prove a `1/120` step at speeds `129/127` produces
        master `+1` and U/V displacement `-1/60`; cover every panning/time
        mode, cache/force/local-source/lock behavior, and incremental upload.
        Focused texture/scene nextest passes 79 tests, and checks through the
        game/viewer pass. A read-only census decodes and animates all six
        shipped Ice textures with stable checksums. All twelve dependencies are
        ordinary static `Texture` objects with exact destination dimensions.
  - [ ] Live acceptance: compare `HP_Dungeon.doors.SlydoorICE` (linear 128/100,
        frequency 11,
        amplitude 44), `HP_FX.Snitch_Halo` (circular 128/128, frequency 20,
        amplitude 95), `BlueFog_01`, and `GreenFog`; replay halo users in
        `Lev2_Quid1`, `Lev3_Quid2`, and representative `Quid_*` maps.
  - [ ] Unproved engine-general boundary: native virtual locks structurally
        permit an Ice dependency that is itself Ice, but the complete shipped
        corpus has none and establishes no cycle/order semantics. OpenHP1 fails
        that case explicitly. A scan of 6,816 compiled Class/State/Function/
        Struct exports also found no `SourceTexture` or `GlassTexture` writes;
        runtime dependency-identity replacement therefore remains unimplemented
        rather than guessed. Fixed-identity pixel refresh, native smaller-source
        expansion/`LocalSource`, first-lock output, priming, signed time, and
        same-tick incremental upload are implemented.
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
      `bMeshEnviroMap` changes into shared scene materials, including draw-time
      Actor → current Region.Zone → LevelInfo environment fallback refresh.
      Evidence:
      `Ghidra_Engine.c:126909-126989` and
      `Ghidra_Render.c:10130-10134,13034-13266`.
- [ ] `BASE-011` Apply retail LodMesh collapse, morph, hysteresis, and
      distance-detail behavior instead of always rendering maximum detail.
      Evidence: `Ghidra_Render.c:10130-10134,31283-31361`; discarded fields:
      `openhp1-mesh/src/geometry.rs:71-125`.
- [ ] `BASE-012` Implement original viewport screen flashes/fades through one
      local-player runtime-to-render path shared by Classic and Modern. Direct
      evidence is the
      compiled `Engine.u` `PlayerPawn` bytecode (`ClientFlash` export 4319,
      `ClientInstantFlash` 3485, `ClientFadeIn` 4280, `ClientFadeOut` 3466,
      `SetViewFlash` 3402, and `ViewFlash` 4100), Engine draw/config handling at
      `Ghidra_Engine.c:117664-117768,121781-121788`, and shipped
      [`D3DDrv.dll` `EndFlash`](../res/Ghidra_D3DDrv.c#L3182), RVA `0x1087` -> VA `0x10008be0` (SHA-256
      `7683b11647dafe3926eff7d0d055abbe3d728648a19f5f8a613fd03efd151599`).
  - [x] Native cadence is pinned: `UGameEngine::Tick` dispatches `ViewFlash`
        exactly once to `Client.Viewports[0].Actor`, using the effective outer
        tick delta, after the complete level/world tick and before
        `UWindowsClient::Tick` and its eligible viewport repaint/`Draw` path.
        Direct Engine evidence is `Ghidra_Engine.c:252233-252238,252647-252653`
        and VAs `0x103a15f2-0x103a1647`; the exported wrapper at `0x103036bb`
        jumps to `0x103242a0` and has no internal caller or vtable pointer.
        Shipped `WinDrv.dll` completes the render-order proof through
        `UWindowsClient::Tick` `0x11102e00`, its repaint call
        `0x11102f3e-0x11102f44`, and `UWindowsViewport::Repaint`
        `0x11108a50` calling Engine `Draw` at `0x11108a8b`. The local actor
        order before the outer dispatch is `PlayerTick`, `ProcessState`,
        timers, then `performPhysics` (`Ghidra_Engine.c:264212-264253`).
  - [x] Native `AInterpolationManager::performPhysics` flash ownership is
        resolved negatively. Its vtable entry `0x1046ecac` selects the wrapper
        at `0x10301d02` and body `0x103f7880-0x103f9163`; the body has no access
        to rendered PlayerPawn flash offsets `0x4f8-0x504` and no `ViewFlash`
        reference. Its writes at `Ghidra_Engine.c:38397-38415,38481-38492` are
        manager-owned interpolation derivative/basis state at `this+0x298`
        through `this+0x2ac`, not player or viewport flash state. Do not add
        interpolation-manager flash behavior from inactive embedded source.
  - [x] Reproduce the proved `PlayerPawn` plane state exactly by dispatching
        the shipped compiled client writers and `ViewFlash` bytecode unchanged
        through the existing VM: client writers
        scale authored RGB by `.001`; `ViewFlash` caps delta at `.1`, advances
        and clamps fade W to `[-1,0]`, combines desired/constant/zone fog with
        one added to W, decays desired by `2d`, interpolates by `10d`, clears
        instant fog each update, and applies the `.981` W and `.019` RGB snaps.
        Treat `FlashFog.W` as the effective scale; a separate active
        `FlashScale` property is not proved in this build.
  - [x] At the narrow shared seam, expose the owning local player's resulting
        scale/fog through `PlayerView`, pass it to `Renderer::render`, and draw
        one fullscreen pass after Classic output or Modern final composite/AA
        but before game UI/egui. Match D3D's saturated
        `fog + scene*clamp(scale,0,1)` equation using source `ONE`, destination
        `SRC_ALPHA`, including its clamped 8-bit diffuse-color quantization.
        `EndFlash` multiplies each component by `256.0` at
        `0x10008cb3,0x10008cfa,0x10008d28,0x10008d56`, subtracts `0.5` at
        `0x10008cce,0x10008d06,0x10008d34,0x10008d62`, converts with `fistp`,
        and clamps to `0..255`. `D3DDrv.dll`, `Engine.dll`, and `WinDrv.dll`
        contain no x87 control-word writes, so the inherited default
        round-to-nearest-even mode makes the literal result
        `clamp(round_ties_even(component*256 - .5),0,255)`, including its
        odd/even half-integer boundary behavior. Classic applies the shared
        pass before its final gamma pass; Modern applies it after composite/AA.
        Do not add per-actor scene state or duplicate backend-specific effects.
  - [x] Parse `WindowsClient.ScreenFlashes` with shipped default true. False
        supplies identity (`scale=1`, zero fog) at draw time while runtime flash
        state continues to advance; it must not reset the player properties.
  - [x] Deterministic implementation acceptance: the shipped writer/`ViewFlash`
        bytecode remains single-owned by the VM and was exercised unchanged
        against the local corpus. Public synthetic tests prove exactly one
        post-world dispatch to the primary player, no physics-step or no-player
        dispatch, `FlashFog` projection through `PlayerView`, config-disabled
        draw-time identity without state destruction, D3D byte quantization,
        identity/black/fractional/saturated blends, RGB-only alpha preservation,
        valid WGSL, and one shared pass in the Classic pre-gamma and Modern
        post-composite/AA paths before UI. No copyrighted package is required
        by public tests.
  - [ ] Live acceptance: compare the two authored `ViewFlash` triggers in
        `Lev2_fire1`; the repeated red flashes and `fadeout 2.0` in `Lev5_Final`;
        HUD/console exclusion; `ScreenFlashes` true/false; and matching event
        timing in retail, Classic, and Modern, including a hitch that exercises
        the `.1` delta cap. `Lev3_Lumos` provides additional TriggeredViewFlash
        coverage.

### Evidence still required

- [x] Audit the shipped D3D render-device DLL for delegated visual/config
      behavior. The renderer audit records the 40-entry `UD3DRenderDevice`
      export block, direct blend/polyflag/depth/filter/stage/attachment/frame/
      gamma states, source and generated-artifact hashes, and a reproducible
      Ghidra 12.1.2 headless workflow. `SoftDrv.dll` was used only to separate
      software clear and `(Brightness+0.5)/128` shade-table behavior from D3D's
      final `2.5` gamma ramp; temporary analysis files are not repository
      dependencies.
- [ ] Trace Render.dll caller reachability and effective configuration before
      promoting D3D's RenderFog/specular branch, Invisible `ZERO/ONE`
      depth-writing branch, or AlphaBlend-only `ONE/INVSRCALPHA` branch to an
      OpenHP1 defect. Preserve the live-confirmed actor-opacity path meanwhile.
- [ ] Correlate the now-proved macro/detail texture-info slots with their
      serialized `UTexture` properties and find shipped non-null representatives;
      exact D3D pass math and capability gates are recorded in `BASE-018`.
- [ ] Use controlled retail captures before claiming spatial 16-bit dithering
      or fixed-function raster precision; D3D requests dithering, but driver
      output is implementation-dependent.
- [x] Decompile shipped [`Fire.dll`](../res/Ghidra_Fire.c) far enough to recover implementation-ready
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
