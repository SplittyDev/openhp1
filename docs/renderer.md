# Textured BSP renderer

The current renderer draws paletted base textures from the original packages
on world BSP geometry.

## Data flow

1. `openhp1-package` validates the package container and object tables.
2. `openhp1-map` follows the `Level` export's world-model reference.
3. `openhp1-package::PackageStore` discovers packages through the original
   `[Core.System] Paths` and resolves grouped imports case-insensitively.
4. `openhp1-texture` expands the first P8 mip and its palette to RGBA8.
   Water-backed `WetTexture` exports retain simulation state; `FireTexture`
   exports currently produce static preview frames.
5. `Model::triangulate` emits node-local vertices with raw UE texture
   and lightmap coordinates.
6. `openhp1-map` reconstructs static lightmap images from zone ambient colors,
   light actors, and the model's blurred one-bit shadow masks.
7. `openhp1-map` decodes the BSP `SkyZoneInfo` actor's fixed location and
   Unreal rotator.
8. `openhp1-scene` resolves actor defaults and instance properties, retains
   first-class actor records, and appends visible vertex meshes, skeletal
   meshes, and moving brushes with their materials and render ranges.
9. `openhp1-scene` combines BSP, texture, mesh, and actor flags into
   backend-neutral surface materials, including `ZoneInfo` texture-pan speeds.
10. `openhp1-render` packs lightmaps with replicated edge gutters into one
   atlas, normalizes coordinates, batches opaque triangles, sorts blended BSP
   surfaces, advances `PF_AutoUPan`/`PF_AutoVPan` texture coordinates, and draws
   them with repeat sampling and depth testing. A sky map first renders to a
   separate color/depth target; fake-backdrop polygons sample that target in
   screen space during the playable pass.
11. `openhp1-viewer` presents an offscreen `Rgba8Unorm` target inside egui and
    exposes searchable actor state and diagnostics.

The renderer does not know package paths or export indices. It accepts decoded
CPU geometry and texture images plus a caller-provided wgpu device, queue,
encoder, texture view, format, and viewport size.

## Coordinates and camera

HP1/UE1 positions use left-handed X-forward, Y-right, Z-up coordinates. The
renderer converts these once to right-handed X-right, Y-up, negative-Z-forward
coordinates:

```text
(unreal.x, unreal.y, unreal.z) -> (unreal.y, unreal.z, -unreal.x)
```

This conversion changes handedness, so UE polygon winding is clockwise after
conversion. One-sided wgpu pipelines therefore use clockwise front faces.

The initial camera starts at the converted model-bounds center. UE1 maps are
commonly subtractive worlds carved inside solid BSP; placing an overview camera
outside the bounds only exposes an unhelpful outer hull.

Lit surfaces multiply their base texture by the reconstructed lightmap in
display space using UE1's 2x modulation. This deliberately matches UE1's
fixed-function rendering instead of applying modern linear-light sRGB math.
Zone-zero surfaces inherit the active `LevelInfo` ambient color. Unlit and
lightmap-less surfaces bypass that multiply.

## Verified maps

`Quid_RavenA.unr` was visually verified on macOS through eframe's Metal-backed
wgpu renderer. It decodes to 1,120 points, 756 BSP nodes, 463 surfaces, and
1,955 triangles.

The base-texture path was visually verified with `Lev5_Chess.unr`: all 961 BSP
surfaces resolved to 15 unique decoded textures and rendered with their UE1
coordinates.

The translucent and modulated paths were visually verified on
`Lev2_HogFront.unr`, including overlapping fountain sheets and basins using
`WetWater`.

Run a map from the repository root:

```sh
cargo run -p openhp1-viewer -- res/Maps/Lev5_Chess.unr
```

The viewer accepts another map path as its only argument. Without an argument,
it uses the Quidditch map above.

## Known omissions

The renderer supports opaque, masked, translucent, and modulated base
textures. Translucent and modulated BSP surfaces use the original blend
equations, depth-test without writing depth, and are sorted by surface center
for each frame. UE1 precedence makes translucent win when both blend flags are
present and clears masking only for translucent surfaces.

Masked P8 textures discard palette index zero, invisible surfaces are omitted,
and only surfaces or textures marked two-sided disable backface culling.
When a map has fake backdrops, the renderer first draws the same BSP from the
map's fixed `SkyZoneInfo` viewpoint into a separate target. The playable pass
then composites that image only over depth-tested fake-backdrop polygons.
Backdrop depth prevents geometry outside the visible zone from leaking through
until full BSP visibility traversal exists.

Surfaces carrying UE1's `PF_Unlit` flag bypass lightmap multiplication. This
matters for sky-box cube faces.

The renderer still uses only the first mip and does not cull by zones. Visible
vertex- and skeletal-mesh actors are decoded, lit, rendered, and can play
serialized or runtime-selected animation sequences. Mover geometry comes from
the brush model's `Polys` export and follows UE1's
`Location * Rotation * MainScale * -PrePivot` transform; runtime mover rotation
therefore pivots around `Location`, not the mesh-actor
`Location + PrePivot` origin. Sprite actors use texture-sized quads aligned to
the active UE1 view axes. Engine `S_*` textures and textures from the `HPEdit`
package are editor viewport icons, not runtime sprites, and are excluded from
that path. Particle actors use their live `ParticleFX` emitter state; coronas
remain unsupported.
Water-backed `WetTexture` exports animate independently of actor scripts.
Automatically panned BSP surfaces use their associated zone's `TexUPanSpeed` and
`TexVPanSpeed`; zone zero inherits the active `LevelInfo` values.
`FireTexture` previews and time-varying light effects remain static.
Unsupported texture classes use a magenta checkerboard.

Sky rendering clips at the fake-backdrop polygons rather than reproducing
UE1's scanline BSP portal-span clipper. Full BSP zone/visibility traversal can
replace that rasterized equivalent when it is needed for broader engine
compatibility.
