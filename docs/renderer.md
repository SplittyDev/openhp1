# First BSP renderer

The first renderer deliberately stops at untextured world geometry. Its job is
to prove the complete path from an original `.unr` package to visible pixels
before texture resolution, lightmaps, actors, or gameplay complicate that
path.

## Data flow

1. `openhp1-package` validates the package container and object tables.
2. `openhp1-map` follows the `Level` export's world-model reference.
3. `Model::triangulate` emits a triangle fan for each convex BSP node polygon.
4. `openhp1-render` uploads the point and index buffers and draws them with
   depth testing.
5. `openhp1-viewer` presents an offscreen `Rgba8Unorm` target inside egui.

The renderer does not know package paths or export indices. It accepts decoded
CPU geometry and a caller-provided wgpu device, queue, encoder, texture view,
format, and viewport size.

## Coordinates and camera

HP1/UE1 positions use left-handed X-forward, Y-right, Z-up coordinates. The
renderer converts these once to right-handed X-right, Y-up, negative-Z-forward
coordinates:

```text
(unreal.x, unreal.y, unreal.z) -> (unreal.y, unreal.z, -unreal.x)
```

The initial camera starts at the converted model-bounds center. UE1 maps are
commonly subtractive worlds carved inside solid BSP; placing an overview camera
outside the bounds only exposes an unhelpful outer hull.

The shader currently derives a face normal from screen-space position
derivatives and applies one directional light. This is diagnostic shading, not
an attempt to reproduce the original game's lighting.

## Verified map

`Quid_RavenA.unr` was visually verified on macOS through eframe's Metal-backed
wgpu renderer. It decodes to 1,120 points, 756 BSP nodes, 463 surfaces, and
1,955 triangles.

Run it from the repository root:

```sh
cargo run -p openhp1-viewer -- res/Maps/Quid_RavenA.unr
```

The viewer accepts another map path as its only argument. Without an argument,
it uses the Quidditch map above.

## Known omissions

The current renderer does not resolve textures, compute BSP UVs, draw
lightmaps, interpret polygon blend flags, cull by zones, render actors, or
decode vertex meshes. Texture resolution and BSP UVs are the next useful step.
