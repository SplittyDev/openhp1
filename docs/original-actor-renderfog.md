# Original actor-mesh RenderFog

This note isolates the original per-vertex fog path used by actor meshes. It is
not BSP `FogMap`, zone/distance fog, or OpenHP1 Modern volumetrics.

Evidence labels:

- **Direct**: stated by the shipped HP1 binaries, configuration, or packages.
- **Reconstruction**: algebra or field names inferred from direct native data
  flow, with the raw offsets retained.
- **Reference**: corroboration from the licensed local SurrealEngine clone.
- **Unresolved**: not established strongly enough to implement as parity.

## Result

**Direct.** The feature is the internal polygon flag
`PF_RenderFog = 0x40000000`, not an actor `RenderFog` property. For both legacy
`UMesh` and `ULodMesh`, the renderer performs one actor-scoped light setup, runs
`FLightManager::LightAndFog` on transformed vertices, passes its fog RGB through
`FTransTexture`, and finishes the actor after all its triangle batches. The
shipped D3D device realizes that fog as fixed-function **specular RGB**, enabled
only for this path. **Reconstruction from the fixed-function D3D contract:**
specular RGB is added after texture/diffuse modulation. Translucent and
modulated materials suppress it; opaque and masked materials retain it.

The smallest OpenHP1 seam is therefore one camera-dependent fog value per
actor-mesh vertex in the shared scene draw, followed by one post-texture RGB add
in the common Classic/Modern scene shader path. It must precede optional Modern
volumetrics and must not reuse BSP fog-map or froxel state.

## Native call and lifetime

1. **Direct.** `URender::DrawMesh` (`FUN_10b0e920`, address `0x10b0e920`) and
   `URender::DrawLodMesh` (`FUN_10b0ff00`, address `0x10b0ff00`) transform the
   mesh and resolve its textures before calling light-manager vtable slot `+8`
   once for the actor. They OR its returned mode/flags into each triangle's
   authored flags, call slot `+0x18` for each participating transformed vertex,
   submit triangle batches, then call slot `+0x14` once to finish the actor
   ([Render named DrawMesh](../res/Ghidra_Render.c#L10130),
   [DrawMesh setup/shade/finish](../res/Ghidra_Render.c#L10470),
   [Render named DrawLodMesh](../res/Ghidra_Render.c#L15113),
   [DrawLodMesh setup/shade/finish](../res/Ghidra_Render.c#L15920),
   [actual DrawMesh address](../res/Ghidra_Render.c#L23813),
   [actual DrawLodMesh address](../res/Ghidra_Render.c#L24539)).
2. **Direct.** Vtable slot `+8` is `FLightManager::SetupForActor`, decompiled as
   `FUN_10b098f0` at `0x10b098f0`; slot `+0x18` is `LightAndFog`, decompiled as
   `FUN_10b02b10` at `0x10b02b10`; slot `+0x14` is `FinishActor`. The unwind
   labels identify all three functions
   ([SetupForActor body/name](../res/Ghidra_Render.c#L22155),
   [LightAndFog body/name](../res/Ghidra_Render.c#L17903),
   [FinishActor name](../res/Ghidra_Render.c#L22613)).
3. **Direct.** `SetupForActor` clears the actor-local light arrays, derives its
   ambient state, selects and visibility-tests ordinary lights, processes the
   supplied linked fog-light list, and returns `0x40000000` only when that list
   contributes. Each supplied fog light is marked at light-info offset `+0x50`,
   missing records are appended, and the marked records are gathered and
   sorted before vertex evaluation. The qsort target at export thunk
   `0x10b0133e` resolves to `FUN_10b077c0` at `0x10b077c0`, which orders records
   by descending float `light_info +0x10` (the third component of the
   frame-relative light vector)
   ([setup initialization and early gates](../res/Ghidra_Render.c#L22195),
   [light selection and visibility](../res/Ghidra_Render.c#L22323),
   [fog-list flag, marking, and sorting](../res/Ghidra_Render.c#L22540),
   [fog comparator](../res/Ghidra_Render.c#L20798)).
4. **Direct.** `FinishActor` follows all triangle submissions and texture
   unlocks, so the global light-manager arrays and cache locks are actor-scoped,
   not persistent vertex state. Fog is recomputed when the actor is drawn; it
   is camera-dependent and cannot be baked at map-load time
   ([DrawMesh finish ordering](../res/Ghidra_Render.c#L10755),
   [DrawLodMesh finish ordering](../res/Ghidra_Render.c#L16155),
   [FinishActor cleanup](../res/Ghidra_Render.c#L22613)).

### Setup gates

**Direct.** `SetupForActor` skips its entire light/fog setup under the raw
conditions at actor offsets `+0x198` bit `0x1`, `+0xa4 == -1`, a missing level
lighting structure, a non-normal viewport render mode, or a nonzero device/view
field at `Frame->Viewport +0x48`. The decompile also limits chosen ordinary
lights, distinguishes special-lit state, and line-traces candidate lights
before accepting them ([raw gate](../res/Ghidra_Render.c#L22240),
[selection limits and trace](../res/Ghidra_Render.c#L22375)).

**Unresolved.** The semantic names of every raw setup field above are not all
proven by the shipped binaries. They should be preserved as acceptance cases
only after their actor/device property identities are traced; they are not a
license to invent approximate gates.

## Light-info inputs

**Direct.** `FLightInfo::ComputeFromActor`, `FUN_10b06920` at `0x10b06920`,
constructs the values later consumed by `LightAndFog`:

| Light-info field | Native construction |
|---|---|
| `+0x08..+0x10` | light position relative to the actor/frame coordinates |
| `+0x3c` | light brightness scalar |
| `+0x64` / `+0x68` | `R = (VolumeRadius + 1) * 25`, then `R^2` |
| `+0x6c` | `B = VolumeBrightness * (+0x3c) / 64` |
| `+0x70` | squared light distance in the current frame |
| `+0x74` / `+0x78` | `1/R`, then `1/R^2` |
| `+0x7c` | whether the camera/frame origin is inside the sphere |
| `+0x9c..+0xa4` | fog RGB derived through the light's HSB/global-light path |
| `+0xa8` | `VolumeFog / 255` |

The construction and cached 256-entry light/fog ramps are visible directly at
[Render light-info base fields](../res/Ghidra_Render.c#L20246) and
[Render volumetric fields](../res/Ghidra_Render.c#L20650). The Engine binary
independently identifies `VolumeBrightness` at actor byte `+0x1ea` and
`VolumeRadius` at `+0x1eb` in its property replication comparison
([Engine light properties](../res/Ghidra_Engine.c#L127192)). `VolumeFog` is the
byte read at actor `+0x1ec` by the Render construction above.

## Per-vertex equation

Let `p` be the camera-to-vertex vector used by `FTransTexture`, `c` the
camera-to-fog-light center, `D = |p|`, `R` the volumetric radius, and `B` the
brightness scalar above. `LightAndFog` first writes ordinary vertex-light RGB
to `FTransTexture +0x30..+0x3c` and initializes fog RGBA at
`+0x40..+0x4c` to zero ([initialization](../res/Ghidra_Render.c#L17965)).

**Direct.** `PF_Unlit` (`0x00400000`) returns immediately after those initial
writes. Ordinary lighting is accumulated and RGB-clamped first. Fog is entered
only when the per-triangle flags contain `PF_RenderFog` (`0x40000000`)
([unlit early return](../res/Ghidra_Render.c#L17981),
[ordinary-light clamp and fog gate](../res/Ghidra_Render.c#L18090)).

**Reconstruction, exact algebra from `0x10b030ed..0x10b033c9`.** For an ordinary
spherical fog light, native computes the visible camera-to-vertex segment
through the sphere. With

```text
q   = dot(c, p)
rho2 = R*R - q*q / dot(p, p)
tc  = q / D
h   = sqrt(R*R - rho2)
lo  = max(-h, tc - D)
hi  = min( h, tc)
u0  = lo / R
u1  = hi / R
A   = 3 - 3*rho2/(R*R)
```

write `G(u) = (A - u*u)*u`. The contribution before its final factor is

```text
f = B * (G(u1) - G(u0))
```

for the ordinary clipped case. In the explicit full-entry branch where native
selects `lo = -h`, it instead uses `f = B * G(u1)`; this seemingly asymmetric
piece is retained because it is what the shipped decompile executes, not
silently replaced by a cleaner sphere integral. An empty/negative result is
zero. Native also rejects a sphere behind an outside camera and caches one
perpendicular rejection. A distinct native `LightEffect == 2` branch uses
`f = B * D / R`. In both branches it performs `f = min(f, 1)` and then
**doubles it**, producing `f2 = 2*f`
([sphere segment](../res/Ghidra_Render.c#L18130),
[clamp and final doubling](../res/Ghidra_Render.c#L18188)).

For each fog light in the sorted actor-local list, native then composes:

```text
a       = min(f2 * fog_alpha, 1)
fog.rgb = min((1 - a) * fog.rgb + f2 * light_fog_rgb, 1)
fog.a   = min(fog.a + a, 1)
```

([composition](../res/Ghidra_Render.c#L18196)). This is ordered and therefore
must not be replaced by an unordered sum. The accumulated alpha controls
subsequent fog-light composition; D3D does not copy it to the final specular
alpha.

**Reference.** SurrealEngine independently models the same camera-to-vertex
sphere integral and ordered `rgb = source + destination*(1-alpha)` composition
in [`RenderFog.cpp`](../../SurrealEngine/SurrealEngine/Render/RenderFog.cpp#L143).
Its simplified density normalization/multiplier is not treated as proof where
it differs from the shipped decompile.

## D3D material and device behavior

**Direct.** `UD3DRenderDevice::DrawTriangles` (export annotation `0x1055`) sets
its fog/specular path only when both conditions hold:

```text
device_capability_0x9d0 != 0
(PolyFlags & 0x40000044) == 0x40000000
```

Thus `PF_RenderFog` must be present while `0x04` (Translucent) and `0x40`
(Modulated) must both be absent. Masked is not excluded. `SetBlending` (export
annotation `0x104b`) repeats the same precedence test, clears `PF_RenderFog`
when it fails, and maps surviving bit `0x40000000` to D3D render state `0x1d`
(`SPECULARENABLE`) ([SetBlending gate/state](../res/Ghidra_D3DDrv.c#L1659),
[DrawTriangles gate](../res/Ghidra_D3DDrv.c#L1817)).

**Direct material precedence.** The fog eligibility check runs before the
remaining blend normalization. A draw with none of `0x10000044` is made
occluding; a draw with Modulated clears Masked. `DrawLodMesh` also turns actor
opacity below one into flags containing `0x10000004`, so the D3D fog gate rejects
that alpha-blended actor path as well
([blend normalization](../res/Ghidra_D3DDrv.c#L1668),
[LOD opacity flags](../res/Ghidra_Render.c#L15720)).

For eligible vertices, the 32-byte transformed/lit vertex is packed as:

```text
diffuse  = AARRGGBB from Light at FTransTexture +0x30..+0x3c
specular = FFRRGGBB from Fog RGB at FTransTexture +0x40..+0x48
```

For ineligible opaque/modulated vertices specular is zero; Translucent forces
white diffuse and zero specular. The upload and packing are direct at
[D3D vertex packing](../res/Ghidra_D3DDrv.c#L1854). **Reconstruction from that
vertex layout plus state 29:** actor RenderFog is a post-texture fixed-function
specular add, not alpha blending of the mesh.

**Direct.** Capability `+0x9d0` defaults to true, is explicitly disabled on a
3dfx device path, and is disabled when the detected capability word at `+0x2fc`
lacks bit `0x200` ([default](../res/Ghidra_D3DDrv.c#L1172),
[device exceptions/capability](../res/Ghidra_D3DDrv.c#L16844)).
**Reconstruction.** The bit and render-state use identify this as fixed-function
Gouraud-specular support, not a user fog preference.

**Direct.** `UseVertexFog` is separately registered as a config boolean at
device offset `+0x9e4`, but that offset has no behavioral reads in the shipped
D3D decompile (only registration and object copy operations). The shipped D3D
`Default.ini` section does not set it
([registration](../res/Ghidra_D3DDrv.c#L1142),
[shipped D3D config](../res/System/0/Default.ini#L261)). It is not the gate for
this feature.

## Shipped reachability

**Direct package data.** A read-only scan of all shipped maps using
`LoadedScene::load` and the existing light decoder found nonzero
`VolumeFog`+`VolumeRadius` lights only in:

- `Lev2_Fire2.unr`: 18 lights; representative authored tuples include
  `(VolumeFog, VolumeBrightness, VolumeRadius) = (25,45,10)`, `(100,80,80)`,
  and `(150,120,5)`.
- `Lev2_fire1.unr`: 12 lights; tuples include `(120,64,16)`, `(120,64,22)`,
  and `(150,64,25)`.

The values are decoded by the same shipped-property path at
[lighting property decode](../crates/openhp1-map/src/lighting.rs#L103) and
retained on [`RenderLight`](../crates/openhp1-scene/src/render.rs#L24).

**Inference, not live proof.** Comparing decoded fog-sphere radii against
decoded rendered-mesh centers produces candidates `WoodChest1`, `WoodChest3`,
and `RememberallChaseArrow9` in `Lev2_Fire2`, plus `Snail0` and `Snail1` in
`Lev2_fire1`. Their decoded materials are opaque, unmasked, and lit, so they
pass the known material/Unlit exclusions. This still proves only spatial and
material eligibility in decoded data. Native `SetupForActor` also requires the
fog light to reach the actor's supplied leaf-linked fog list and survive its
visibility/device gates.

**Unresolved.** No candidate above is yet a proven visible opaque/masked retail
draw with nonzero specular fog. A retail capture or an exact replay of native
leaf linking and visibility is still required before naming a live acceptance
representative.

## Distinctions

- **BSP FogMap** is a generated texture attachment on `FSurfaceInfo`, submitted
  as a separate full-facet surface pass. Actor RenderFog is a per-vertex
  `FTransTexture::Fog` value sent in D3D specular. The data, draw call, and blend
  path are separate.
- **Zone/distance fog** uses `ZoneInfo` fog properties and remains a separate
  camera/zone feature.
- **Modern volumetrics** are OpenHP1's optional ray-marched/froxel enhancement.
  They globally gather authored `RenderLight` instances and render a volume in
  a later Modern-only pass; they neither populate actor vertices nor provide
  the fixed-function post-texture add
  ([Modern instance gathering](../crates/openhp1-render/src/renderer/modern/volumetric.rs#L416),
  [Modern pass](../crates/openhp1-render/src/renderer/modern.rs#L354)).

## Smallest shared implementation seam

No new renderer or volumetric subsystem is needed.

1. Add one fog RGB channel to the shared GPU scene vertex/output used by both
   modes. Default it to zero for BSP, sprites, and ineligible materials.
2. Retain decoded fog-ball inputs and actor eligibility beside the existing
   actor vertex-lighting context. Evaluate the exact camera-to-vertex equation
   at draw/update time, because camera motion changes it even when the scene is
   otherwise static. Do not bake it in `LoadedScene::load`.
3. In the shared scene shader, add interpolated fog RGB **after** texture ×
   diffuse lighting for Opaque and Masked actor materials. For Modern, perform
   the same display-space add before its display-to-linear conversion. Do not
   run it for Unlit, Translucent, Modulated, or AlphaBlended draws, and do not
   route it through the Modern volumetric pass.

The present shared anchors are actor render ranges and vertex relighting in
[`runtime_display.rs`](../crates/openhp1-scene/src/loader/runtime_display.rs#L407),
the one scene vertex upload in [`renderer.rs`](../crates/openhp1-render/src/renderer.rs#L1029),
and the paired Classic/Modern light helpers in
[`scene.wgsl`](../crates/openhp1-render/src/shaders/scene.wgsl#L430).

### Minimum deterministic checks

- Pure evaluator: miss, tangent/empty segment, camera inside, vertex-clipped
  segment, `LightEffect == 2`, `f` clamp then `f2 = 2*f`.
- Ordered accumulation: two differently colored fog lights prove source order,
  RGB clamp, alpha clamp, and that accumulated alpha is not output specular
  alpha.
- Gates: no linked fog lights, `PF_Unlit`, Translucent, Modulated, and disabled
  capability produce zero; Opaque and Masked retain fog; toggling
  `UseVertexFog` changes nothing.
- Shared output: a synthetic textured triangle proves post-texture addition in
  Classic and the same display-space base result in Modern with Modern
  volumetrics both enabled and disabled.
- Lifecycle: moving only the camera changes actor fog on the next draw; moving
  or hiding a fog light refreshes actor eligibility without retaining the prior
  actor's light list.
