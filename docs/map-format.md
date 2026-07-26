# HP1 level and BSP model format

This document covers the inline `Model` representation used by HP1 map
packages newer than package version 61. The version-61 representation stores
vectors, points, nodes, surfaces, and vertices as separate helper UObjects.

## Finding the world model

A map may contain many `Model` exports for brushes and movers. The largest
model happens to be the world in inspected maps, but relying on size would be a
heuristic. The authoritative reference is serialized by the map's `Level`
export after its actor array and travel URL fields.

`Entry.unr` is a valid version-72 package whose referenced world model is
empty: it contains no points, nodes, surfaces, or triangles. Renderers must
treat that as an empty scene rather than trying to bind zero-length buffers.

## Primitive prefix

`Model` inherits `Primitive`. After tagged UObject properties, it serializes:

- an axis-aligned box: minimum and maximum `f32` vectors plus a validity byte;
- a bounding sphere: center and radius as four `f32` values.

## Inline BSP arrays

The world model then stores compact-counted arrays in this order:

1. vectors used for normals and texture axes;
2. world-space points;
3. BSP nodes;
4. BSP surfaces;
5. a shared BSP vertex pool.

A node refers to a contiguous range in the vertex pool. Every pool entry
selects a point. The resulting node polygon is convex and can be emitted as a
triangle fan while preserving its surface index.

A surface contains its texture object reference, polygon flags, base point,
normal and texture-axis indices, lightmap and editor brush information, pan
values, and brush actor.

For a surface point `P`, base point `B`, texture vectors `U` and `V`, and
integer pans, its raw texture coordinates in texels are:

```text
u = dot(U, P - B) + PanU
v = dot(V, P - B) + PanV
```

Divide these values by the selected texture mip's width and height before
sampling a normalized repeating GPU texture. Vertices are emitted per BSP node
polygon because a shared world point may use different surfaces and therefore
different texture coordinates.

After the geometry arrays come the shared-side count, zone table, editor
polygon object, lightmap metadata and bits, collision bounds, leaf hulls,
convex leaves, light actors, and two final model flags.

Each surface selects a lightmap index. A lightmap stores its shadow-bit offset,
texture-space pan, U/V clamp dimensions, U/V scale, and an offset into the
model's null-terminated light-actor list. One shadow mask follows for each
actor in that list. Its rows use `(width + 7) / 8` bytes and the low bit
represents the leftmost texel.

For lightmap pan `LMPan` and scale `LMScale`, the raw lightmap coordinates are:

```text
u = (dot(U, P) - (dot(U, B) + LMPanU - 0.5 * LMScaleU)) / LMScaleU
v = (dot(V, P) - (dot(V, B) + LMPanV - 0.5 * LMScaleV)) / LMScaleV
```

OpenHP1 reconstructs the original static lightmap pixels from zone ambient
color, light actor properties, and the blurred one-bit shadow masks. Node
`Zone1` supplies the ambient settings for the stored polygon winding. Zone
zero is not “no zone”; it falls back to the active `LevelInfo` actor, matching
UE1's `GetZoneActor`.

## Rendering boundary

`openhp1-map` retains coordinates in Unreal's native convention. Axis or
handedness conversion belongs in one renderer conversion module; loaders must
not mutate positions to suit a particular graphics backend. Texture
coordinates remain in raw UE texels until `openhp1-render` knows the decoded
texture dimensions. Lightmap coordinates likewise remain in raw lightmap
texels until the renderer packs the decoded images into an atlas.
