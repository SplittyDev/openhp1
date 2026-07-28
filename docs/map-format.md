# HP1 level and BSP model format

This document covers the inline `Model` representation used by HP1 map
packages newer than package version 61. The version-61 representation stores
vectors, points, nodes, surfaces, and vertices as separate helper UObjects.

## Finding the world model

A map may contain many `Model` exports for brushes and movers. The largest
model happens to be the world in inspected maps, but relying on size would be a
heuristic. The authoritative reference is serialized by the map's `Level`
export after its actor array and travel URL fields.

The model reference is followed by a compact-counted `ReachSpec` array used by
UE1 navigation. Each entry stores distance, start and end navigation actors,
required collision radius and height, reach flags, and a pruned marker.

`Entry.unr` is a valid version-72 package whose referenced world model is
empty: it contains no points, nodes, surfaces, or triangles. Renderers must
treat that as an empty scene rather than trying to bind zero-length buffers.

## Level actors and scene identity

The `Level` export stores an actor array of package object references. Null
entries are empty slots, and inspected maps can repeat the same export
reference. OpenHP1 therefore retains one `SceneActor` per distinct local actor
export, identified by the package source and zero-based export index.

Each scene actor preserves its object and resolved class names, Unreal-space
transform, draw state, resolved brush or mesh, current animation state, and any
actor-local diagnostics. Visible actors also retain their vertex and index
ranges inside the shared CPU render mesh. This keeps the current batching
strategy while letting the script runtime address an actor and change its
transform or active animation without rebuilding the scene.

Actor state is assembled from the decodable class-default chain followed by
the actor export's tagged properties. A direct class-default failure contributes
an actor diagnostic; inherited failures remain logged once, while derived and
instance properties stay usable.

Executable state code retains its decoded instruction pointer and local values
across latent `Sleep` and `FinishAnim` calls. Runtime label lookup uses the
final top-level `LabelTable` in canonical decoded bytecode; the serialized
state metadata offset is not a canonical decoded-byte offset.

## Primitive prefix

`Model` inherits `Primitive`. After tagged UObject properties, it serializes:

- an axis-aligned box: minimum and maximum `f32` vectors plus a validity byte;
- a bounding sphere: center and radius as four `f32` values.

`Model` and its referenced `Polys` object may carry the standard serialized
UObject stack before those properties. This occurs on several Lev_Tut1 mover
brushes, so both decoders must consume the stack according to the export flags.

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

Moving-brush models use the referenced `Polys` object rather than BSP nodes for
their visible faces. Each polygon stores its base, normal, texture axes,
vertices, flags, actor and texture references, name, link indices, and U/V pan.

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
