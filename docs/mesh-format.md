# HP1 mesh and animation conventions

This document records coordinate and sampling behavior needed to decode the
game's classic, LOD, and skeletal meshes.

## Ownership

Package-specific geometry, animation-sequence decoding, and pose sampling live
in `openhp1-mesh`. Playback orchestration remains with its consumer until more
than one consumer needs a reusable animation controller.

## Actor transform

Mesh placement applies the actor transform, mesh origin, scale, and
`RotationOrigin` in the established scene transform chain. UE1 object rotation
uses positive yaw, negative pitch, and negative roll in yaw/pitch/roll
composition order. It must not be replaced with the inverse camera rotation.

## Skeletal coordinates

HP1 skeletal data uses a mirrored ActorX local Y axis relative to the rest of
OpenHP1's coordinate path. Skeletal geometry and sampled poses therefore:

- negate local Y positions;
- reverse triangle winding;
- negate `RotationOrigin.Yaw`; and
- conjugate bind and animated bone orientations before composing the bone
  hierarchy.

These corrections belong in the shared mesh decoding/sampling path, not as
per-actor or renderer fixes.

## Packed rotations

Each signed packed quaternion component maps to:

```text
sin(value * pi / (2 * 32767))
```

Do not replace this with linear signed normalization.

## Sampling

Vertex and skeletal sequences interpolate between adjacent samples and wrap at
the sequence boundary. Runtime playback treats `AnimLast` as completion rather
than waiting for the sampler to wrap toward frame zero.
