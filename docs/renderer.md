# Textured BSP renderer

The current renderer draws paletted base textures from the original packages
on world BSP geometry.

## Data flow

1. `openhp1-package` validates the package container and object tables.
2. `openhp1-map` follows the `Level` export's world-model reference.
3. `openhp1-package::PackageStore` discovers packages through the original
   `[Core.System] Paths` and resolves grouped imports case-insensitively.
4. `openhp1-texture` expands the first P8 mip and its palette to RGBA8.
5. `Model::triangulate` emits node-local vertices with raw UE texture
   coordinates.
6. `openhp1-render` normalizes those coordinates, batches triangles by texture,
   and draws them with repeat sampling and depth testing.
7. `openhp1-viewer` presents an offscreen `Rgba8Unorm` target inside egui.

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

The initial camera starts at the converted model-bounds center. UE1 maps are
commonly subtractive worlds carved inside solid BSP; placing an overview camera
outside the bounds only exposes an unhelpful outer hull.

The shader currently derives a face normal from screen-space position
derivatives and applies one directional light. This is diagnostic shading, not
an attempt to reproduce the original game's lighting.

## Verified maps

`Quid_RavenA.unr` was visually verified on macOS through eframe's Metal-backed
wgpu renderer. It decodes to 1,120 points, 756 BSP nodes, 463 surfaces, and
1,955 triangles.

The base-texture path was visually verified with `Lev5_Chess.unr`: all 961 BSP
surfaces resolved to 15 unique decoded textures and rendered with their UE1
coordinates.

Run a map from the repository root:

```sh
cargo run -p openhp1-viewer -- res/Maps/Lev5_Chess.unr
```

The viewer accepts another map path as its only argument. Without an argument,
it uses the Quidditch map above.

## Known omissions

The current renderer uses only the first mip and ordinary opaque sampling.
It does not yet draw lightmaps, interpret masked/translucent/modulated polygon
flags, cull by zones, render actors, or decode vertex meshes. Unsupported
texture classes use a magenta checkerboard.
