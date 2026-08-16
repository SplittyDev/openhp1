# Textured BSP renderer

The current renderer draws paletted base textures from the original packages
on world BSP geometry.

## Data flow

1. `openhp1-package` validates the package container and object tables.
2. `openhp1-map` follows the `Level` export's world-model reference.
3. `openhp1-package::PackageStore` discovers packages through the original
   `[Core.System] Paths` and resolves grouped imports case-insensitively.
4. `openhp1-texture` expands every authored P8 mip and its palette to RGBA8.
   `openhp1-scene` retains that exact chain for Classic and Modern; generated
   Wet, Fire, and Ice frames remain single-level. AnimNext frames may replace
   the GPU texture when their authored chain shape changes.
5. `Model::triangulate` emits node-local vertices with raw UE texture
   and lightmap coordinates.
6. `openhp1-map` reconstructs Classic lightmap images and separately preserves
   zone ambient, light actors, and blurred one-bit shadow masks for Modern.
7. `openhp1-map` decodes the BSP `SkyZoneInfo` actor's fixed location and
   Unreal rotator.
8. `openhp1-scene` resolves actor defaults and instance properties, retains
   first-class actor records, and appends visible vertex meshes, skeletal
   meshes, moving brushes, and authored `bCorona`/`Skin` records with their
   materials and render ranges. Actor-mesh faces whose selected material has no
   texture are retained but hidden, matching UE1's mesh rendering path.
9. `openhp1-scene` combines BSP, texture, mesh, and actor flags into
   backend-neutral surface materials, including `ZoneInfo` texture-pan speeds.
10. `openhp1-render` packs Classic lightmaps or Modern visibility masks with
   replicated edge gutters, batches opaque triangles, sorts blended BSP
   surfaces, advances `PF_AutoUPan`/`PF_AutoVPan` texture coordinates, and draws
   them with repeat sampling and depth testing. A sky map first renders to a
   separate color/depth target; fake-backdrop polygons sample that target in
   screen space during the playable pass. A map carrying `PF_Mirrored` likewise
   renders the shared scene from a camera reflected across the authored BSP
   plane, then projects that target over the mirror polygons.
11. `openhp1-viewer` presents an offscreen `Rgba8Unorm` target inside egui and
    exposes searchable actor state and diagnostics.
12. `openhp1-game` renders the scene and its UI into the selected internal
    resolution, then presents that texture into the largest centered rectangle
    that fits the independently resizable window. Exact integer enlargement
    uses nearest sampling; other sizes use linear sampling and black
    letterbox/pillarbox bars.

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
Authored `bDarkLight` actors subtract their masked contribution and clamp at
zero; the shipped `Render.dll` performs the same signed-light branch before
accumulation. Classic reconstruction and Modern per-fragment lighting preserve
that flag through their shared authored-light data.
Zone-zero surfaces inherit the active `LevelInfo` ambient color. Unlit and
lightmap-less surfaces bypass that multiply.

## Modern rendering

Classic rendering remains the default. `--renderer=modern` keeps the decoded
UE1 textures, materials, batching, sky, and animation path, but evaluates the
original light actors per fragment in the same display-space modulation domain
as Classic before decoding the combined overbright result into an `Rgba16Float`
target. Per-surface light lists and the original blurred one-bit masks retain
authored visibility without reusing the precomposited colored lightmaps. Zone
ambient and UE1 radius, spotlight, radial, shell, and cylinder falloff remain
authored inputs. Individual light contributions use Classic's bound, while the
accumulated 2x-modulated result remains unclamped for HDR post-processing. The
modern post pass provides:

- selectable AgX, default luminance-preserving Reinhard Equation 4 with a
  `1.25` white point that retains a short UE1 overbright shoulder, and ACES tone
  mapping;
- selectable SSAO or XeGTAO reconstructed from the scene depth buffer, with a
  full-resolution intermediate visibility texture and two edge-aware spatial
  denoise passes; fake-backdrop pixels are excluded so sky-box seams do not
  receive ambient occlusion;
- selectable FXAA or three-pass SMAA 1x after tone mapping, with SMAA enabled
  by default;
- authored UE1 coronas drawn as HDR screen-space sprites when volumetric
  lighting is disabled;
- depth-clipped HDR volumetric scattering for authored UE1 fog volumes,
  visible `bCorona` sources, and textured light sprites such as candle flames;
- shadowed world-space sunlight shafts on maps that contain both a
  `SkyZoneInfo` and fake-backdrop portal surfaces;
- quarter-resolution HDR bright extraction with separable bloom blur that
  excludes ordinary sub-white texture detail; and
- sRGB output encoding followed by hue-preserving display-space contrast and
  the existing brightness adjustment after tone mapping.

Classic base textures and lightmaps remain `Rgba8Unorm`, preserving UE1's
display-space 2x modulation. Modern uses the authored masks only as visibility,
recreates that display-space response from the original lights, and then
decodes the potentially overbright result to linear HDR. Values above one
remain available to tone mapping and bloom.

The Modern lighting shader rejects lights that cannot contribute at the
current fragment before sampling their authored visibility mask. The final
composite similarly avoids AO and bloom texture reads when those effects are
disabled; enabled paths keep the same sampling and arithmetic order.

Modern-only HDR, sampleable-depth, post-processing, bloom, AO, corona, and
volumetric resources are created only for `RendererMode::Modern`. Each unique
eligible light draws one projected sphere. Authored UE1 volumes use UE1's
analytic fog-sphere density integral; visible corona and textured light sources
use a compact, low-energy inverse-square profile concentrated near the source.
Textured sources use the sprite's chroma and a smaller profile so multi-flame
fixtures do not stack into a white fog volume.
Invisible fill lights used only to shape baked illumination do not become
visible fog orbs.
Enabling volumetric lighting suppresses the legacy corona sprites so the two
source-glow treatments do not stack.
When volumetric lighting is disabled, scene updates and frame preparation skip
the unused volumetric renderer entirely; changing the setting rebuilds the
renderer. When it is enabled, sprite-derived light colors are cached per unique
source texture and recomputed only when `update_textures` reports that texture
as changed. Animated geometry therefore does not repeatedly scan unchanged
sprite pixels.
Scene depth terminates the ray, and the result enters the HDR scene before bloom
and tone mapping. Unshadowed fallback volumes retain the analytic integral;
shadowed sources use a bounded march. The classic scene shader, uniform layout,
target format, depth usage, draw order, and display-gamma path remain unchanged.

Point-shadow cube faces persist across frames. A slot is redrawn only when its
selected source position or radius changes, or when an exactly changed opaque
shadow-caster triangle overlaps that light's cube. Camera motion can change the
selected slot but does not otherwise invalidate a point shadow. Changed caster
bounds include both the old and new triangle positions, so moving geometry
cannot leave stale depth behind.

After shadow generation and froxel compute complete, froxel composite,
directional shafts and local volumes draw in that order inside one additive HDR
render pass. None samples the updated HDR attachment, so the merge preserves
the previous draw order while avoiding intermediate attachment stores and
loads on tile-based GPUs.

Directional shafts use a renderer-owned four-layer shadow map over opaque scene
geometry and a camera-aligned `rgba16float` froxel volume. At the normal
1024x768 internal resolution the volume is 128x96x64, with exponential depth
slices from the camera near plane to the shaft distance. Each froxel evaluates
the visible window portals, their stained-glass transmission masks, and the
matching directional shadow layer. Because the original maps provide no sun,
the renderer uses a shallow synthetic indoor-sun direction and reflects it
inward for windows on opposing walls; its horizontal travel remains greater
than its downward travel so high windows illuminate across the room rather than
falling directly into the nearby floor. It samples a two-octave world-space
density field and integrates single scattering front-to-back with Beer-Lambert
transmittance, a moderate Henyey-Greenstein phase function, and fixed high
scattering albedo. The completed 3D volume is trilinearly sampled at scene depth
and added to the HDR scene.

The source triangles only bound light injection; Composite and Scattering modes
never render their extruded faces. All triangles from one authored window
surface share one averaged tint, while each froxel receives the triangle that
covers its back-projected source coordinate. This keeps haze anchored in the
room as the camera moves, lets nearer air attenuate farther scattering, and
removes the camera-facing translucent-prism appearance. See
[`research/window-volumetric-lighting.md`](research/window-volumetric-lighting.md)
for the production-engine comparison and staged design.

In the composite view, each window's affine texture mapping is also projected
across the complete authored surface rather than clipped to its individual BSP
triangles, then adds a low-energy, one-sided, shadowed window-shaped footprint
to the HDR scene.
The aperture mask is prefiltered with a 13-tap separable tent kernel and retains
fractional transmission; a nine-tap mask filter then grows from 1.5 to 8 texels
with distance from the window. A nine-tap shadow filter grows from 2 to 12 shadow
texels, while the surface-cookie boundary fades from 3 to 12 screen pixels. This
approximates the softer transmission and penumbra produced by stained glass
without exposing BSP triangle edges.
Fake-backdrop surfaces are authored sky openings. The fixed shipped
maps do not mark indoor stained-glass windows, so the scene loader also marks
surface texture names containing `win`, excluding known frame and non-aperture
tokens `arch`, `column`, `wood`, `wallwindow`, and `furnace`. This corpus-backed
fallback finds the `Lev_Tut1` windows while finding none in `Lev3_Dungeon`.
Classified window textures feed a renderer-owned 128-pixel transmission-mask
array. Because the shipped textures include both the window and its surrounding
stone wall, mask construction flood-fills the border-connected wall and retains
only mid-luminance glass inside the painted frame. The shaft march projects each
sample back through the opening's authored texture coordinates, so painted
mullions split the light even when the original map did not model them as
geometry. Fake-backdrop sky openings use a fully transmissive mask.
Only source triangles whose light volumes intersect the camera frustum are
injected, capped at the 128 nearest triangles. Opaque walls and props shadow the
resulting volume.
The accumulation is forward-weighted along the light-to-camera path and uses
additive HDR scattering with no scene-wide extinction, retaining values for
bloom and tone mapping without tinting the whole room.
Window shafts and local volumetric sources share a slowly drifting world-space
density field with configurable haze cells. Window shafts also carry
sparse world-space motes inside their authored prisms; camera-facing billboards
keep them round while scene depth and the sun shadow map clip them to the lit
volume. Both layers pause with the rest of the scene.

The map viewer exposes live dust size, density, opacity, and speed controls,
plus haze field size, density, opacity, and speed. These are temporary tuning
controls and do not change `OpenHP1.ini`. Its volumetric view selector can show
the normal composite, scattering alone on black, the projected window mask,
directional shadow visibility, or local-light shadow visibility. Visibility
views use green for light-visible samples and red for blocked samples, while the
mask view uses white for transmission. Local visibility isolates the nearest
shadowed local light so overlapping authored ranges do not add into yellow;
moving the camera near another source selects it. The shared defaults are dust size
`4 px`, density `64`, opacity `0.05`, and speed `5 units/s`; haze size `60
units`, density `0.75`, opacity `0.5`, and speed `25 units/s`.

Local volumetric sources outside the shadow budget retain their compact HDR
halos. Up to twenty nearest visible emitters or explicitly authored fog lights
use their authored UE1 lighting radius capped to a 300-unit fog extent, receive
renderer-owned cube shadow maps, and use a bounded 32-sample
world-space scattering march, forward-weighted along the light-to-camera path.
Near a local source, bright textured fixture triangles transmit while darker
triangles remain two-sided shadow casters, so a lamp's panes shape the volume
and its metal frame splits it into rays. Geometry farther from the source stays
fully opaque regardless of its texture.
Nearby emitters belonging to one physical fixture share at most three shadowed
samples. Three-emitter corona lanterns retain their authored output. Sprite-only
candles instead use a compact 50-unit fog extent and one-third emitter energy
per flame, with each fixture capped at one emitter's total energy. Dense
multi-candle chandeliers cannot consume the entire shadow budget or add the same
fixture's haze dozens of times; they use a 150-unit fog extent.
This budget keeps candles and chandeliers responsive without turning invisible
level-lighting helpers into disembodied fog or rendering a shadow cube for every
light in a room.

Each directional shadow projection is snapped to shadow-map texels so subtle
shafts do not crawl when the camera moves. Directional rays are bounded by
opening prisms. Local rays integrate only samples visible from their light;
lights selected for cube shadow maps are removed from the unshadowed fallback
pass so geometry leaves genuinely dark air behind it. Lights beyond the fixed
shadow budget retain the compact fallback. Local corpus inspection confirms
`Lev_Tut1`, `Lev_Tut2`, and `Lev_Tut3` contain classified window apertures,
while `Lev3_Dungeon` contains none; `Furnacewindow` in `Lev3_DungeonB` is
explicitly excluded.

SSAO uses a stable 16-sample screen-space kernel instead of rotating that
kernel independently at every pixel. XeGTAO uses a five-level positive
view-depth pyramid, depth-derived normals, three horizon slices with three
steps per side, fixed spatial noise, and the same denoiser. Temporal
reprojection is intentionally not part of either path. Both methods remain
screen-space effects: an occluder that has left the depth buffer cannot keep
contributing AO even when the receiving surface remains visible.

FXAA uses a single fullscreen pass. SMAA uses separate color-edge detection,
blend-weight, and neighborhood-blending shaders with the reference area and
search lookup textures. Both operate on the final display-encoded image; SMAA
uses its medium preset without temporal or multisample accumulation.

Shader sources live under `src/shaders`. The shared scene shader stays outside
the `modern` directory because Classic and Modern use the same scene/material
path. Focused Modern WGSL fragments are concatenated into complete shader
modules in Rust because WGSL has no source-include directive.

The viewer exposes these choices in its sidebar, keeps independent Classic and
per-tone-mapper Modern display values, and provides a Modern-only contrast
control. The game and viewer default to brightness/contrast `0.66/1.05` for
Reinhard, `0.64/0.75` for ACES, and `0.6/0.9` for AgX. Classic retains
brightness `0.6` and neutral contrast. Reinhard is the default tone mapper.
Renderer mode, tone mapper, ambient occlusion, and anti-aliasing are also
available on the command line:

```sh
cargo run --release -p openhp1-viewer -- \
  res/Maps/Lev5_Chess.unr \
  --renderer=modern \
  --tone-mapper=agx \
  --ambient-occlusion=ssao \
  --anti-aliasing=smaa
```

`--tone-mapper` accepts `agx`, `reinhard` (or `classic`), and `aces`.
`--ambient-occlusion` accepts `off`, `ssao`, and `xegtao` (or `gtao`). These
settings have no effect on the classic renderer. `--anti-aliasing` accepts
`off`, `fxaa`, and `smaa`.

The in-game Graphics Settings page edits the same `RendererSettings` used by
the viewer and command line. Its Classic-only 16-bit option keeps the actual
wgpu targets at 32-bit and quantizes the final composed frame to RGB565 in the
presentation shader; it is an output emulation rather than a claim that every
texture and blend operation ran through a historical 16-bit framebuffer.

Modern BSP lighting follows runtime light brightness, position, and spotlight
rotation changes through `RenderScene`; the volumetric instances follow those
same runtime changes. Mesh actors retain the existing CPU vertex-lighting path.
The volumetric pass is camera-depth-aware and sunlight shafts use
direction-specific shadow maps. A bounded set of nearby local lights uses cube
shadow maps over the same opaque scene geometry, including BSP and actor meshes.
Diffuse GI, DDGI volumes, specular materials, and reflection probes are not yet
implemented. Future GI and
reflection captures should remain
renderer-owned resources built from `RenderScene`; package references and BSP
serialization details must not cross into the renderer.

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

The viewer accepts one map path followed by renderer options. Without a map
path, it uses the Quidditch map above.

## Known omissions

The renderer supports opaque, masked, translucent, modulated, and HP1
actor-opacity base textures. Translucent and modulated BSP surfaces use the original blend
equations, depth-test without writing depth, and are sorted by surface center
for each frame. UE1 precedence makes translucent win when both blend flags are
present and clears masking only for translucent surfaces.

Actors with `Opacity < 1` use HP1's native `SrcAlpha` / `OneMinusSrcAlpha`
blend, clear masking, and join the same depth-tested, non-writing sorted pass.

Masked P8 textures discard palette index zero, invisible surfaces are omitted,
and only surfaces or textures marked two-sided disable backface culling.
When a map has fake backdrops, the renderer first draws the same BSP from the
map's fixed `SkyZoneInfo` viewpoint into a separate target. The playable pass
then composites that image only over depth-tested fake-backdrop polygons.
Backdrop depth prevents geometry outside the visible zone from leaking through
until full BSP visibility traversal exists.

`PF_Mirrored` (`0x08000000`) marks a planar reflection rather than an ordinary
opaque wall. As in UE1, the effective surface flags combine the BSP surface's
flags with its texture flags; a texture whose serialized `bMirrored` property
is set therefore creates a mirror even when the BSP surface itself does not
carry `PF_Mirrored`.
Original maps contain both world-BSP and mover-brush mirrors, and a single map
can contain multiple reflection planes. Surface ownership and each reflection
plane come from the authored triangle surface index and triangle geometry;
shared vertex metadata can describe an adjacent plane. OpenHP1 gives every
authored mirror surface its own reflected camera and render target in both
renderer modes. The reflected pass clips world and actor fragments to the
viewer's side of the authored plane, matching the half-space retained by UE1's
mirror-portal BSP traversal. Geometry physically behind a mirror therefore
cannot cover or leak into its reflection.

`PF_Portal` (`0x04000000`) surfaces whose BSP zone is owned by a
`WarpZoneInfo` use that actor's serialized `WarpCoords` and live
`OtherSideActor` connection. The renderer applies UE1's source-unwarp then
destination-warp camera transform, clips the destination scene at the warped
portal plane, and projects the result behind the authored portal polygons
before opaque overlays. Portal recursion preserves the authored BSP side: a
warp is active only when the opposite zone is owned by its `WarpZoneInfo`.
`PF_Mirrored` and `PF_Portal` surfaces may interleave, so the Erised path renders
warp, reflection, then warp rather than following `OtherSideActor` links as an
unconditional chain. The three-traversal limit matches HP1's shipped
`Render.dll`. Until OpenHP1 implements UE1's scanline portal-span traversal,
the nearest nested surface intersecting the portal view's center ray selects
that single recursive branch.

Surfaces carrying UE1's `PF_Unlit` flag bypass lightmap multiplication. This
matters for sky-box cube faces.

Base textures use every authored mip with linear min/mag filtering, point mip
selection, and the shipped D3D `-0.5` LOD bias; `bNoSmooth` changes only
min/mag to point. The renderer does not cull by zones. Visible
vertex- and skeletal-mesh actors are decoded, lit, rendered, and can play
serialized or runtime-selected animation sequences. Their transforms use
HP1's upward-positive pitch, so their visual forward axis matches runtime
`GetAxes` movement. Mesh placement follows UE1's
`(Location + PrePivot + MeshAdjust) * Rotation * DrawScale * meshToObject`
transform. `meshToObject` retains the mesh's authored origin, scale, and
rotation. HP1's skeletal `MeshAdjust` aligns the visual and collision bottoms
when `bAlignBottom` and `bCollideWorld` are enabled, `Physics` is not
`PHYS_None`, and `CollideType` is not `CT_Shape`; other actors keep their
authored placement. The shipped `Engine.dll` reads the actor byte at offset
`0x30` for this condition, and `AActor::setPhysics` writes that same byte. Its
vertical adjustment is
`(Mesh.Origin.Z - Mesh.BoundingBox.Min.Z) * Mesh.Scale.Z * DrawScale -
CollisionHeight - 2.5`, matching the shipped `Engine.dll`. Mover geometry comes
from the brush model's `Polys` export and follows UE1's
`Location * Rotation * MainScale * -PrePivot` transform; runtime mover rotation
therefore pivots around `Location`. A `MainScale` transform with negative
determinant also reverses the generated triangle winding so mirrored brushes
retain their authored front faces and normals. Mesh actors instead pivot around
their `Location + PrePivot` origin. Sprite actors use texture-sized quads
aligned to the active UE1 view axes. Engine `S_*` textures and textures from
the `HPEdit`
package are editor viewport icons, not runtime sprites, and are excluded from
that path. Particle actors use their live `ParticleFX` emitter state. In modern
mode, coronas use their authored `Skin`, `DrawScale`, hue, and saturation; a
fixed HDR gain supplies the luminance that UE1 did not author so they feed the
bloom pass. Their display-space texture and tint are decoded to HDR after UE1's
translucent RGB modulation; P8 palette alpha does not attenuate the glow.
Corona size uses UE1's fixed `0.8 * DrawScale` viewport-width fraction, and
positions and lifetimes follow their actors. Their quads are depth-tested
rather than using UE1's center-point BSP visibility trace.
Water-backed `WetTexture` exports animate independently of actor scripts.
Automatically panned BSP surfaces select `Node.Zone0` or `Node.Zone1` from the
render-pass camera side of the node plane and use that actor's `TexUPanSpeed`
and `TexVPanSpeed`. Positive node-plane space selects `Zone1`; non-positive
space selects `Zone0`.
A missing zone actor falls back to `Level.Actors(0)`, the active `LevelInfo`.
See [the original-engine evidence](texture-panning-engine-evidence.md).
`FireTexture` previews and time-varying light effects remain static.
`WaveTexture` is also missing; its exact shared Water kernel is tracked under
`BASE-009C`, with no shipped gameplay representative.
Unsupported texture classes use a magenta checkerboard.

Sky rendering clips at the fake-backdrop polygons rather than reproducing
UE1's scanline BSP portal-span clipper. Full BSP zone/visibility traversal can
replace that rasterized equivalent when it is needed for broader engine
compatibility.
