# Shipped D3D render-device inventory

Status: read-only binary/decompilation inventory. This document classifies the
complete shipped `UD3DRenderDevice` implementation and turns every observable
gap found during that pass into implementation-ready checklists. It does not
require OpenHP1 to reproduce DirectDraw COM topology, cache data structures, or
compiler scaffolding when their observable result is already specified.

Primary evidence is the legally obtained shipped `D3DDrv.dll`, SHA-256
`7683b11647dafe3926eff7d0d055abbe3d728648a19f5f8a613fd03efd151599`, and
[`res/Ghidra_D3DDrv.c`](../res/Ghidra_D3DDrv.c). Engine-owned screenshot
commands are proved by [`res/Ghidra_Engine.c`](../res/Ghidra_Engine.c).

## Reproducible coverage reconciliation

The inventory used:

```sh
objdump -p res/System/D3DDrv.dll
objdump -d --start-address=0x10001000 --stop-address=0x10001240 \
  res/System/D3DDrv.dll
```

A structural pass then counted every top-level decompiler body whose opening
brace is on a line by itself. Four control-flow braces were removed as false
positives, and `UD3DRenderDevice::operator=` was added because a naive
assignment-signature filter misses it.

| Body class | Count | Reconciliation |
|---|---:|---|
| Named device/object bodies | 45 | 39 code entries in ordinals 295-334, plus six object lifecycle/operator bodies |
| Unnamed `FUN_100...` bodies | 125 | Product implementations, generated support, and compiler/module bodies classified below |
| Duplicate/forwarding thunks | 57 | No independent behavior |
| Catch handlers | 153 | Classified with their owner; no independent behavior |
| Unwind handlers | 136 | Classified with their owner; no independent behavior |
| Module/compiler bodies | 7 | No renderer behavior |
| **Total top-level bodies** | **523** | Exact structural count after the five corrections above |

The exported `UD3DRenderDevice` block is the contiguous ordinal range
295-334: 39 code entries plus `PrivateStaticClass` data. Every code export is
a direct jump to the implementation VA below. The six bodies outside a simple
direct-call closure are also closed: `FUN_100014b0` is `DllMain`,
`FUN_1000a870/1000a8b0` are deleting destructors, `FUN_10011beb` is a teardown
continuation, and `FUN_10014ef0/10014fe0` are generated container cleanup
reached through thunks/catches.

## Complete named/exported mapping

`L...` is the opening line of the actual address-bearing implementation body,
not the duplicated friendly-name decompilation near the start of the file.

| Method | Actual body | Classification / observable owner |
|---|---|---|
| `AdjustPolyFlags` | `0x10009280`, L14434 | Material flag normalization |
| `ClearZ` | `0x10008450`, L13873 | Depth clear |
| `CreateVideoTexture` | `0x10009670`, L14593 | Texture allocation; only resulting format/size/stalls are observable |
| `D3DError` | `0x1000f030`, L17912 | HRESULT diagnostic |
| `Draw2DLine` | `0x100069c0`, L12976 | Canvas/debug line pixels |
| `Draw2DPoint` | `0x10007020`, L13174 | Canvas/debug point pixels |
| `DrawComplexSurface` | `0x10003ac0`, L11549 | BSP base/macro/light/detail/FogMap passes |
| `DrawTile` | `0x100060b0`, L12646 | Canvas textured tile pixels |
| `DrawTriangles` | `0x10005b60`, L12484 | Gouraud/mesh triangle submission |
| `EndFlash` | `0x10008be0`, L14181 | Screen flash/fade pass |
| `EnumDevicesCallback` | `0x1000ac70`, L15477 | Capability/setup |
| `EnumDirectDrawsCallback` | `0x1000a8f0`, L15293 | Capability/setup |
| `EnumModesCallback` | `0x10006560`, L12791 | Mode capability/setup |
| `EnumPixelFormatsCallback` | `0x10009200`, L14409 | Texture format capability/setup |
| `EnumZBufferCallback` | `0x1000abe0`, L15449 | Depth format capability/setup |
| `Exec` | `0x100065e0`, L12816 | `GETRES`, `LODBIAS`, `SHOWPOOLS` |
| `Exit` | `0x10002c90`, L10852 | Lifecycle |
| `Flush` | `0x10002e00`, L10934 | Gamma/resource refresh |
| `GetOsSurface` | `0x100090c0`, L14347 | Platform/editor surface interop |
| `GetStats` | `0x10008230`, L13809 | Device statistics text |
| `Init` | `0x10002ac0`, L10770 | Lifecycle/setup |
| `InitTextureStageState` | `0x1000adf0`, L15554 | Sampler/stage defaults |
| `InternalConstructor` | `0x10001990`, L10375 | Object setup |
| `Lock` | `0x100030c0`, L11076 | Clear, recovery, begin-frame |
| `MaxVertices` | `0x10005b40`, L12474 | Returns 256; batching/API constraint |
| `PopHit` | `0x10007d00`, L13602 | Editor hit testing |
| `PrecacheTexture` | `0x100037f0`, L11417 | Cache warming |
| `PushHit` | `0x10007880`, L13426 | Editor hit testing |
| `ReadPixels` | `0x10008510`, L13916 | Framebuffer readback/screenshots |
| `RecognizePixelFormat` | `0x1000ad30`, L15509 | Format/capability setup |
| `SetBlending` | `0x100092d0`, L14451 | Blend/depth/alpha-test/filter state |
| `SetRes` | `0x1000b360`, L15691 | Mode, target, depth, caps, recreation |
| `SetTexture` | `0x10009a80`, L14774 | Conversion, mip upload, binding |
| `ShutdownAfterError` | `0x10002d30`, L10892 | Error teardown |
| `StaticClass` | `0x10001580`, L10224 | UObject scaffolding |
| `StaticConstructor` | `0x100019b0`, L10387 | Config registration/defaults |
| `UnSetRes` | `0x10011ab0`, L19853 | Device teardown |
| `Unlock` | `0x100038b0`, L11458 | End/present |
| `UpdateModulation` | `0x10012350`, L20759 | Complex-surface pass color |
| Default constructor | `0x10002070`, L10553 | Setup |
| Copy constructor | `0x100124d0`, L20811 | Setup |
| Assignment | `0x10013050`, L21276 | Setup |
| Destructor | `0x10001600`, L10251 | Teardown |
| `operator new(UObject)` | `0x100015a0`, L10233 | Allocator |
| `operator new(EInternal)` | `0x100015e0`, L10243 | Allocator |

Exact ordinal-to-body reconciliation for the contiguous device block:

```text
295 AdjustPolyFlags       0x10009280   315 Init                    0x10002ac0
296 ClearZ                0x10008450   316 InitTextureStageState   0x1000adf0
297 CreateVideoTexture    0x10009670   317 InternalConstructor     0x10001990
298 D3DError              0x1000f030   318 Lock                    0x100030c0
299 Draw2DLine            0x100069c0   319 MaxVertices             0x10005b40
300 Draw2DPoint           0x10007020   320 PopHit                  0x10007d00
301 DrawComplexSurface    0x10003ac0   321 PrecacheTexture         0x100037f0
302 DrawTile              0x100060b0   322 PrivateStaticClass      data 0x10030f98
303 DrawTriangles         0x10005b60   323 PushHit                 0x10007880
304 EndFlash              0x10008be0   324 ReadPixels              0x10008510
305 EnumDevicesCallback   0x1000ac70   325 RecognizePixelFormat    0x1000ad30
306 EnumDirectDraws       0x1000a8f0   326 SetBlending             0x100092d0
307 EnumModesCallback     0x10006560   327 SetRes                  0x1000b360
308 EnumPixelFormats      0x10009200   328 SetTexture              0x10009a80
309 EnumZBufferCallback   0x1000abe0   329 ShutdownAfterError      0x10002d30
310 Exec                  0x100065e0   330 StaticClass             0x10001580
311 Exit                  0x10002c90   331 StaticConstructor       0x100019b0
312 Flush                 0x10002e00   332 UnSetRes                0x10011ab0
313 GetOsSurface          0x100090c0   333 Unlock                  0x100038b0
314 GetStats              0x10008230   334 UpdateModulation        0x10012350
```

## Complete unnamed/support closure

The 125 address-bearing bodies divide without remainder:

- Module glue: `014b0`.
- UObject/class/allocation: `01510, 01580, 015a0, 015e0, 01600`.
- Construction/defaults/pool support:
  `01990, 019b0, 02070, 02250, 02490, 027f0`.
- Frame lifecycle/clear/present/flush:
  `02ac0, 02c90, 02d30, 02e00, 030c0, 037f0, 038b0`.
- Complex surfaces: `03ac0`.
- Drawing, commands, and mode enumeration:
  `05b40, 05b60, 060b0, 06560, 065e0, 069c0, 07020`.
- Hit/stats/readback/flash/OS surface:
  `07880, 07d00, 08230, 08450, 08510, 08be0, 090c0`.
- Flags/blending/textures/formats/caps:
  `09200, 09280, 092d0, 09670, 09a80, 0a870, 0a8b0, 0a8f0, 0aa60,
  0abe0, 0ac70, 0ad30, 0adf0`.
- Resolution/device setup: `0b360`.
- Error diagnostic: `0f030`.
- Teardown/COM/EH continuation:
  `11ab0, 11beb, 11eb9, 11f3f, 11fce, 12051, 120df`.
- Modulation/copy/assignment: `12350, 124d0, 13050`.
- Generated container/string/destructor support:
  `13bd0, 13ca0, 13e10, 13f80, 140f0, 14170, 142f0, 14370, 14460,
  144b0, 144e0, 145f0, 14630, 14720, 14750, 14860, 148f0, 14a30,
  14b70, 14bcf, 14c04, 14c90, 14dd0, 14e2f, 14e64, 14ef0, 14fe0,
  150f0, 15170, 152a0, 153d0, 15410, 154f0, 15520, 15600, 15710,
  15840, 158c0, 15960, 15a70, 15aa0, 15bc0, 15bf0, 15d30, 15d60,
  15ed0, 16040, 16100, 161e0, 162c0, 163f0, 164c0, 16510, 165f0,
  16640, 16720, 16800, 168e0, 169c0, 16a10`.
- RTTI/compiler runtime:
  `16d6b, 16dc0, 16def, 16e57, 16e6f, 16f21`.

## Direct evidence versus inference

Direct proof includes command literals and branches, pixel equations, state
calls, export VAs, surface-lock rectangles, file headers/order, and body/caller
relationships. Friendly meanings for unknown structure offsets, which COM
interface a decompiler typed imprecisely, and whether a diagnostic is useful to
a player are inferences. Compatibility requirements below are limited to the
directly observable result.

## Implementation checklist

### `SHOT` framebuffer capture

- [ ] Keep this a feature commit separate from `SNAP` and `SAVESNAP`.
- [ ] Route the command through viewport ownership. `UViewport::Exec` owns the
      branch ([E:49170-49265](../res/Ghidra_Engine.c#L49170)); Actor,
      PlayerPawn, and Console natives reach the owning viewport/player
      interface ([E:23122-23249](../res/Ghidra_Engine.c#L23122),
      [E:78608-78724](../res/Ghidra_Engine.c#L78608),
      [E:117505-117553](../res/Ghidra_Engine.c#L117505)). The exact indirect
      vtable member name is inferred.
- [ ] Capture the retained previous completed logical viewport, including
      screen flash and Canvas/HUD/console layers, before Classic display gamma.
      Engine draws those layers before `Unlock`/present
      ([E:117743-117793](../res/Ghidra_Engine.c#L117743)). Which transient
      console frame is visible remains command-timing dependent.
- [ ] For Classic, reproduce D3D `ReadPixels`: lock the windowed viewport
      rectangle or full fullscreen primary surface, decode 16/24/32-bit RGB
      masks, and when gamma control is active apply
      `round(pow(c/255,1/(Brightness*1.5))*255)`
      ([D:13914-14150](../res/Ghidra_D3DDrv.c#L13914)). This is distinct from
      the `Brightness*2.5` display gamma ramp.
- [ ] Write the first free `Shot%04i.bmp` under `appUserDir` for indices
      `0..255`; when all are occupied, handle the command without writing
      ([E:49779-49846](../res/Ghidra_Engine.c#L49779)).
- [ ] Emit a 54-byte header, positive dimensions, bottom-up 24-bit BGR, and
      discard alpha.
- [ ] Deterministic acceptance: 16/24/32 mask extraction, gamma-table values,
      exact BMP bytes, `0000/0255/all occupied`, and a layered
      world/flash/HUD/console capture at the pre-display-gamma logical-output
      seam.

OpenHP1 currently captures the newly rendered internal presentation texture
before egui/HUD/console and before Classic gamma/RGB565/scaling
([app.rs:976-1037](../crates/openhp1-game/src/app.rs#L976)), then writes a
top-down 32-bit BMP. The narrow host seam is a retained previous completed
logical viewport updated after UI but before Classic display gamma; Classic
then applies the separate `Brightness*1.5` readback LUT. Modern retains its
final tone-mapped logical output rather than raw HDR. Exact Classic 16-bit
framebuffer pixels depend on `CLASSIC-004`; correct cropping/readback when the
logical viewport and window differ depends on `HOST-001`.

### `SNAP N` persistent downsample

- [ ] Keep this a feature commit separate from SHOT/SAVESNAP.
- [ ] Compute `scale=ftol(2^N)` and replace a persistent viewport snap of size
      `floor(width/scale) x floor(height/scale)`
      ([E:49417-49510](../res/Ghidra_Engine.c#L49417)).
- [ ] Read the same completed viewport pixels as SHOT, crop right/bottom
      remainders, and compute non-overlapping top-left-origin `scale^2` box
      averages of all four FColor bytes using integer sums/division.
- [ ] Write no file. Retain the buffer until the next SNAP or viewport teardown;
      `UTexture::LoadFromSnap` consumes it
      ([E:136651-136703](../res/Ghidra_Engine.c#L136651)).
- [ ] Deterministic acceptance: a 5x3 source with `SNAP 1` yields an exact 2x1
      result, channel truncation and cropping are fixed, and a second SNAP
      replaces dimensions and contents.

OpenHP1 currently aliases `SNAP N` to a newly named screenshot
([console.rs:270-278](../crates/openhp1-runtime/src/console.rs#L270)); that is
not original behavior.

### `SAVESNAP token` and snap-backed texture

- [ ] Keep this a feature commit separate from SHOT/SNAP.
- [ ] Require a token and nonempty persistent snap; write exactly
      `appUserDir()+token`, without forcing an extension or numbered prefix
      ([E:49512-49777](../res/Ghidra_Engine.c#L49512)).
- [ ] Pad each positive dimension to
      `2^(floor(log2(dimension))+1)`, strictly the next power of two even when
      already a power of two. Repeat the first snap pixel through padded rows
      and columns.
- [ ] Emit positive-height, bottom-up 24-bit BGR and retain the snap afterward.
- [ ] Implement `CreateTextureFromScreenShot` as a transient texture loaded
      from the persistent snap at 128x64
      ([E:105789-105818](../res/Ghidra_Engine.c#L105789)). Package text has only
      a commented HPMenu source lead; active shipped bytecode reachability is
      not proved and must not be claimed.
- [ ] Deterministic acceptance: a 2x2 snap produces an exact 4x4 padded file,
      first-pixel fill, bottom-up rows, unchanged token path, persistent state,
      and 128x64 texture crop/centering behavior.

### Device commands and statistics

- [ ] Implement `LODBIAS <float>` through the shared sampler state; it updates
      device bias plus stages 0 and 1 when multitexture is active
      ([D:12814-12942](../res/Ghidra_D3DDrv.c#L12814)). Do not emulate pools.
- [ ] Implement `GETRES` compatibility output: unique 16-bit modes, at most 16,
      within configured maximum dimensions.
- [ ] Implement `SHOWPOOLS` as compatible diagnostics or explicitly retain a
      truthful modern equivalent; it has no pixel contract.
- [ ] Extend statistics with surface/polygon/tile counts and times, vertex-buffer
      wraps, per-format residency/allocation/set/upload data, upload time, and
      thrash count ([D:13807-13845](../res/Ghidra_D3DDrv.c#L13807)). Obsolete
      pool values are diagnostics, not required allocator architecture.

### Tile, Gouraud, line, point, and texture-format acceptance

- [ ] `DrawTile`: strip internal `0x01000000`; for a palette whose entry zero
      alpha is not 255, force AlphaBlend-only `0x10000000` unless Translucent is
      already set ([D:12668-12674](../res/Ghidra_D3DDrv.c#L12668)). This directly
      disproves the earlier claim that FogMap is the only AlphaBlend-only user.
- [ ] `DrawTile`: preserve half-pixel placement, reciprocal Z/RHW, bound
      max-color multiplication, UV scaling, four-vertex fan, and ties-even
      `component*256-.5` byte packing.
- [ ] `DrawTriangles`: preserve half-pixel/RHW coordinates, texture-dimension UV
      scaling, RGB times bound max color, vertex alpha, Unlit white, optional
      RenderFog RGB in specular, indexed/list submission, and the 256-vertex
      batching limit ([D:12484-12645](../res/Ghidra_D3DDrv.c#L12484)).
- [ ] `Draw2DLine`: preserve endpoint quantization and flag-bit-2 attenuation
      `clamp(1-.001/Z,.2,1)` independently at each endpoint, while disabling and
      restoring texture color/alpha ops ([D:12976-13147](../res/Ghidra_D3DDrv.c#L12976)).
- [ ] `Draw2DPoint`: preserve its five-vertex rectangle fan and `-1.5/-1.0`
      offsets while disabling/restoring texture ops
      ([D:13174-13403](../res/Ghidra_D3DDrv.c#L13174)).
- [ ] Add synthetic palette-alpha, masked-index, ARGB1555, ARGB8888, and
      max-color modulation checks. Physical cache buckets, hardware palettes,
      AGP placement, and COM surfaces remain implementation details.

## Already mapped without a new feature

`ClearZ`, `Lock`, `Unlock`, `SetRes`, `UnSetRes`, `Flush`, cooperative loss,
pixel/depth-format selection, texture stages/cache-visible pixels,
`DrawComplexSurface`, `UpdateModulation`, `EndFlash`, and editor hit testing are
already tracked by their existing audit rows. Enumeration callbacks,
constructors, allocators, generated containers, HRESULT stringification,
GetOsSurface COM identity, RTTI, catch handlers, and unwind handlers add no
independent stock-game rendering behavior after those visible results are
mapped.
