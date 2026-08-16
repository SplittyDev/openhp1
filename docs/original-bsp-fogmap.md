# Original BSP FogMap behavior

This note records how the shipped HP1 renderer produces and draws the
per-surface BSP `FogMap`. It is intentionally separate from actor vertex fog
and from any modern renderer's screen-space or volumetric-froxel effects.

Evidence labels used below:

- **Direct proof**: behavior visible in the shipped native binaries or shipped
  packages/configuration.
- **Inference**: a semantic name or OpenHP1 integration conclusion supported by
  the direct evidence, but not named by the decompiler at that point.
- **Unresolved**: a fact that the available primary evidence does not establish.

## Result

**Direct proof.** `FLightManager::SetupForSurf` constructs a transient 32-bit
FogMap on the same grid and in the same projection domain as a BSP light map.
It admits only volumetric lights reaching that surface, evaluates a scalar
volume integral at every texel, maps the scalar through a cached 256-entry
color/alpha ramp, and combines multiple lights in a 7-bit channel domain. The
D3D device draws the result as the final full-facet pass with
`ONE, INV_SRC_ALPHA` blending. A surface cannot receive both detail texture and
FogMap in the shipped D3D path; FogMap wins.

The smallest OpenHP1 seam is therefore an optional, real per-surface attachment
next to the existing light/macro/detail attachments, generated from shared BSP
surface/light data and consumed by the existing shared attachment pass planner.
It is not a material flag, actor-opacity effect, or Modern-only fog subsystem.

## Native call chain and gates

### Frame and surface entry

**Direct proof.** `URender` initializes and exits the global light manager via
`FLightManager::Init` (`0x10b023a0`) and `FLightManager::Exit`
(`0x10b028e0`) at `res/Ghidra_Render.c:30295,30346`. The BSP draw path calls
the manager's surface setup virtual at `res/Ghidra_Render.c:2604-2608` before
the render-device surface draw. That outer call requires:

- a valid surface light-map index;
- viewport render mode value `5`;
- the relevant model/render field at `+0xac` to be nonzero; and
- level/state field `+0x48` to be zero.

It calls `FLightManager::FinishSurf` (`0x10b096c0`) after the immediate draw
when either the LightMap or FogMap output is non-null
(`res/Ghidra_Render.c:2868-2870`).

### FogMap eligibility

**Direct proof.** Inside `FLightManager::SetupForSurf` (`0x10b077f0`), the
FogMap output is installed only when all of these conditions hold
(`res/Ghidra_Render.c:21025-21047`):

| Condition | Native evidence | Meaning |
|---|---|---|
| current zone pointer is non-null | zone pointer test | a zone actor is available |
| zone byte `+0x2cc` has bit `0x02` | byte/bit test | **Inference:** `ZoneInfo.bFogZone` |
| surface chain at `+0x40` is non-null | linked-list test | at least one volumetric light reaches the surface |
| surface polyflags exclude `0x4` | bit test | `PF_Translucent` surfaces are excluded |

On success, the output points at the reusable global FogMap `FTextureInfo` at
`0x10b4bb90`. The chain's actors are deduplicated into the surface's
`FLightInfo` array and tagged as volumetric. Newly admitted entries receive
type `3` (`res/Ghidra_Render.c:21034-21047`).

The volumetric surface chain is not a list of every volume light in the map.
The renderer builds it geometrically at `res/Ghidra_Render.c:7935-7969`: it
deduplicates candidates, evaluates the light sphere with radius
`(VolumeRadius + 1) * 25` against the facet geometry/plane terms, and prepends
only reaching actors. Dynamic reachability also flows through `URender`'s
leaf-light lists (`res/Ghidra_Render.c:35176-35235`) and the sphere-to-leaf and
compatible-surface assignment in `FUN_10b31c70`
(`res/Ghidra_Render.c:41272-41468`).

The common add-light helper `FUN_10b097b0` (`0x10b097b0`) rejects a light when
the working array is full, `LightType == 0`, `LightBrightness == 0`, or it is
the self light; it then classifies static, dynamic, and special-lit cases
(`res/Ghidra_Render.c:22100-22148`). `res/Ghidra_Engine.c:127102-127150`
independently identifies actor offsets `+0x1e0` through `+0x1e4` as
`LightType`, `LightEffect`, `LightBrightness`, `LightHue`, and
`LightSaturation`.

**Direct proof.** The shipped D3D configuration enables the feature with
`VolumetricLighting=True`, enables multitexture, and disables detail textures
at `res/System/0/Default.ini:255-268`. The configuration is supporting policy
evidence; it is not one of the four local `SetupForSurf` tests above.

## Texture descriptor, allocation, and projection

**Direct proof.** `SetupForSurf` obtains the surface's light-map descriptor,
uses its actual U/V sizes, rounds each storage dimension up to a power of two,
and copies its projection/pan/scale fields into the FogMap descriptor
(`res/Ghidra_Render.c:20943-20959,21048-21085`). It then:

- allocates `pow2_width * pow2_height * 4 + 4` bytes from frame `GMem`;
- places a `-1` sentinel word before the pixel pointer;
- installs the pixel pointer in the global FogMap mip;
- marks the descriptor with flag `0x10`; and
- constructs a device-cache identity from the owning model, surface light-map
  index, zone number, and suffix `0x26`.

Only the actual width by actual height rectangle is evaluated; the power-of-two
width is the row stride. This is direct 32-bit texel data, not an indexed image:
Setup supplies no texture palette, while the D3D `SetTexture` format dispatch
uses a palette only for format `1` (`res/Ghidra_D3DDrv.c:4116-4142,4289-4303,
4338-4343`).

**Direct proof.** The D3D FogMap pass computes each vertex coordinate as

```text
u = (projected_surface_u - PanU + 0.5 * UMult) * device_u_scale
v = (projected_surface_v - PanV + 0.5 * VMult) * device_v_scale
```

and draws every polygon in the facet
(`res/Ghidra_D3DDrv.c:7153-7180`). Thus the FogMap uses its own copied
light-map projection and samples texel centers; it does not reuse base-texture
UVs.

## Per-light parameters and ramp

`FLightInfo::ComputeFromActor` (`0x10b06920`) prepares each admitted light
(`res/Ghidra_Render.c:20246-20791`).

**Direct proof.** For a volumetric entry it:

- derives RGB from the normal light brightness/color computation
  (`res/Ghidra_Render.c:20314-20373,20652-20670`);
- takes the fourth ramp component from actor byte `+0x1ec`, divided by 255;
- uses actor byte `+0x1ea` in the volume brightness scale;
- uses `(actor[+0x1eb] + 1) * 25` as the volume radius; and
- caches the ramp under the actor object identity with suffix `0x20`.

`res/Ghidra_Engine.c:127212-127234` names `+0x1ea` and `+0x1eb` as
`VolumeBrightness` and `VolumeRadius`. **Inference:** `+0x1ec` is
`VolumeFog`; the native code directly proves that it supplies fog-ramp alpha,
but the reviewed decompilation does not attach that property name to the
offset.

The cached block is `0x410` bytes: 16 bytes of validation metadata followed by
256 four-byte ramp entries (`res/Ghidra_Render.c:20667-20753`). Four fixed-point
accumulators linearly ramp the computed channels over indices 0 through 255,
and every stored byte is capped at 127. The decompiler's memory order is
B/G/R/A. **Inference:** naming this Unreal `FColor`/BGRA layout relies on the
surrounding UE/D3D structure convention; the independently proven contract is
four ramped bytes with the fourth byte used as blend alpha.

The cache entry is validated against the four ramp parameters. It is rebuilt
on a mismatch or the actor change flag `0x1000`, and its lock is retained until
surface finish (`res/Ghidra_Render.c:20674-20709`).

## Texel influence and multi-light composition

**Direct proof.** Volumetric entries are sorted with `FUN_10b077c0`
(`0x10b077c0`) by `FLightInfo+0x10` before rasterization
(`res/Ghidra_Render.c:20798-20805,21090-21103`). The comparator orders the
numeric key deterministically. **Unresolved:** the semantic name of that key
and a source-level reason for the selected order are not present in the
decompilation.

For every light, the producer walks the actual FogMap grid with a projected
start point and U/V increments. The scalar kernel is at
`res/Ghidra_Render.c:21135-21325`:

- one actor-mode branch uses an inverse-distance form from the prepared volume
  brightness/radius coefficients;
- the general branch intersects the sampling ray with the light sphere,
  clips the interval using direction, sidedness, and radius terms, evaluates
  the analytic volume falloff integral, and clamps the result to `[0, 1]`.

The first light writes

```text
texel = ramp[round(clamp(influence, 0, 1) * 255)]
```

directly (`res/Ghidra_Render.c:21135-21232`). For every later light, let `src`
be the selected ramp entry and `dst` the accumulated texel. The exact integer
combination is:

```text
dst.rgb[c] = min(127, src.rgb[c] + floor((127 - src.a) * dst.rgb[c] / 127))
dst.a      = min(127, dst.a + src.a)
```

for each of the three color bytes (`res/Ghidra_Render.c:21325-21354`).
`FLightManager::Init` precomputes the 128 by 128 product table
`floor((127 - alpha) * old / 127)` used here
(`res/Ghidra_Render.c:17537-17569`). This is an ordered source-over-like
accumulation in a 7-bit domain. Calling the RGB values strictly premultiplied
would go beyond the directly established arithmetic, so this note preserves
the equation instead.

**Unresolved:** although the kernel's control flow and operations are present,
the decompiler does not name every prepared `FLightInfo` coefficient. A clean
source transcription should first assign those fields from
`ComputeFromActor`, then preserve this native arithmetic and boundary behavior;
it should not replace the integral with a visually tuned radial falloff.

## Device blend and pass order

`UD3DRenderDevice::DrawComplexSurface` (`0x10003ac0`) identifies the
`FSurfaceInfo` attachments at offsets base `+0x0c`, LightMap `+0x10`, macro
`+0x14`, detail `+0x18`, and FogMap `+0x1c`
(`res/Ghidra_D3DDrv.c:6496-7181`).

**Direct proof.** If detail and FogMap are both present, the device nulls the
detail attachment (`res/Ghidra_D3DDrv.c:6573-6576`). The effective FogMap order
is therefore:

```text
base -> macro -> LightMap -> FogMap
```

The base and LightMap may use the existing multitexture shortcut, but FogMap is
still the final full-facet pass. Detail is an alternative before that point,
not an additional pass beneath FogMap.

The FogMap pass calls `SetBlending(0x10000000)` and binds the FogMap as texture
stage zero (`res/Ghidra_D3DDrv.c:7077-7082`). `SetBlending` (`0x100092d0`)
enables alpha blending and sets D3D source blend `2` and destination blend `6`,
which are `ONE` and `INVSRCALPHA`
(`res/Ghidra_D3DDrv.c:1651-1723`):

```text
framebuffer_out = fog_rgb + framebuffer_in * (1 - fog_alpha)
```

The blend mode also follows the non-depth-writing path. Existing surface depth
continues to select the facet; the fog pass does not author new depth.

## Lifetime and caching

**Direct proof.** `SetupForSurf` records a `GMem` mark and reuses global
descriptor/mip storage (`res/Ghidra_Render.c:20886-20905`). The renderer draws
the surface immediately. `FinishSurf` restores that mark, releasing the
transient FogMap pixels and intermediate allocations, then releases every
per-surface cache lock by subtracting `0x01000000` from its lock count
(`res/Ghidra_Render.c:22041-22067`).

The ownership split is therefore:

- transient per-surface FogMap grid: frame `GMem`, valid through the immediate
  `DrawComplexSurface` call;
- persistent per-light 256-entry ramp: global cache, validated and locked while
  the surface is being built;
- reusable FogMap descriptor and mip shell: global light-manager storage; and
- device texture identity: model/lightmap/zone-derived cache ID with suffix
  `0x26` and the realtime-changed descriptor bit.

This lifecycle matters for OpenHP1: retaining a generated attachment is valid,
but its invalidation inputs must include the contributing lights and relevant
zone/surface identity. Treating the global native descriptor as proof that the
image is immutable would be incorrect.

## Shipped-map reachability

**Direct proof from shipped packages.** A read-only local probe used
OpenHP1's existing decoded BSP nodes, zones, light-map indices, light chains,
and raw actor properties; it did not modify the packages. The following
authored conjunction exists:

| Package | `bFogZone` exports | Light maps with a linked light whose `VolumeBrightness`, `VolumeFog`, and `VolumeRadius` are all nonzero | Those light maps also referenced by a BSP node in a fog zone |
|---|---:|---:|---:|
| `res/Maps/Lev2_Fire2.unr` | 1 (`ZoneInfo7`, export 66) | 54 | 54 |
| `res/Maps/Lev2_fire1.unr` | 5 (`ZoneInfo8`, `ZoneInfo5`, `ZoneInfo16`, `ZoneInfo9`, `ZoneInfo1`; exports 1, 944, 1222, 1412, 2230) | 183 | 183 |

Representative `Lev2_Fire2` light export 60 has volume bytes `(45,25,10)`,
HSB `(140,100,50)`, `LightEffect=0`, and `LightRadius=10`; light-map 2140
links nine qualifying volume lights. Representative `Lev2_fire1` lights include
exports 22, 28, 59, and 63 with volume bytes `(64,120,22)`, HSB
`(140,100,180)`, `LightEffect=0`, and `LightRadius=17`.

This establishes that shipped authored data reaches the decoded inputs needed
by the FogMap producer; the result is not a dead generic-engine facility.
**Unresolved:** the probe does not prove that each qualifying surface was
visible in a particular retail frame, survived the native translucent and
dynamic reach tests, or produced a non-null attachment in a live capture.

## OpenHP1 implementation seam

This section is an **integration inference**, not original-engine proof.

The existing shared material path already carries a dormant
`fog_map_attached` boolean in
`crates/openhp1-scene/src/render.rs:143-206`, suppresses detail through
`MaterialBinding::attachment_enabled` in
`crates/openhp1-render/src/renderer/batch.rs:24-47`, and plans the shared
attachment passes in `crates/openhp1-render/src/renderer.rs:1956-1999`. The
narrow shared implementation should:

1. Represent FogMap as an actual optional per-surface texture attachment plus
   its copied light-map projection, rather than only a boolean.
2. Generate its pixels where shared decoded BSP surface/light data is available.
   Reuse the light-map grid/facet projection and implement the native admission,
   ramp, influence, ordering, and 7-bit integer composition directly.
3. Bind it in the existing shared material bind group and append one FogMap pass
   to the existing attachment planner. Keep `FogMap => no detail` as one shared
   decision.
4. Use `ONE, INV_SRC_ALPHA`, no depth write, the FogMap's half-texel UVs, and
   raw normalized attachment bytes. Classic naturally receives the native
   UNORM clamp. Modern can retain the project's intentional HDR intermediate
   policy while sharing the same generated attachment and pass order; no
   second fog implementation is needed.

The generator must eventually support invalidation for dynamic/moving lights.
A static authored first slice is useful only if it is explicitly modeled as
incomplete; silently freezing the native dynamic path would not be parity.

### Focused tests

Producer tests should use synthetic surfaces and lights and cover:

- all four FogMap gates independently, including translucent rejection;
- actual versus power-of-two dimensions and row stride;
- copied pan/scales and half-texel projection;
- representative ramp endpoints, channel cap 127, and `VolumeFog` alpha;
- first-light overwrite and the exact multi-light integer equation, including
  saturation;
- deterministic light ordering;
- both scalar-kernel branches, radius boundary, sidedness, and zero influence;
- cache invalidation inputs and dynamic-light refresh.

Shared renderer tests should cover:

- no FogMap when the attachment is absent;
- `FogMap` suppressing detail but not macro or LightMap;
- effective order `base -> macro -> LightMap -> FogMap` in both Classic and
  Modern planning;
- `ONE, INV_SRC_ALPHA`, depth-write disabled, and full-facet draw;
- the FogMap projection being independent of base UVs; and
- Classic UNORM output versus the existing intentional Modern HDR target
  behavior without changing the fog source arithmetic.

## Boundaries and remaining questions

- `FLightManager::LightAndFog` (`0x10b02b10`,
  `res/Ghidra_Render.c:17903-18256`) is actor per-vertex lighting/fog. Its fog
  branch under flag `0x40000000` is not the BSP FogMap producer.
- An exhaustive literal/symbol search found no `FLightManager`/`FogMap`
  producer in `res/Ghidra_Engine.c` or `res/Ghidra_Fire.c`. This is negative
  evidence only; it does not rule out unrelated indirect fog behavior.
- The exact upstream native point at which the render-device
  `VolumetricLighting` option enables or prevents surface-chain construction
  remains unresolved. The shipped value is enabled and `SetupForSurf`'s local
  gates are known exactly.
- `ZoneInfo+0x2cc bit 0x02` and actor `+0x1ec` still need an independently named
  native property-offset source to promote `bFogZone` and `VolumeFog` from
  strongly supported semantic mappings to named direct proof.
- Live retail/OpenHP1 image comparison remains the acceptance gate after an
  implementation exists.
