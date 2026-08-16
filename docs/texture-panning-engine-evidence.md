# UE1 BSP texture-panning evidence

This note records the original-game evidence behind OpenHP1's automatic BSP
texture panning. It contains no original assets or decompiled source.

## Shipped binaries

The legally obtained HP1 installation used for this investigation contains:

- `Render.dll` SHA-256
  `41c0e9939cac1833978c15bb10a13761b3559ad929f060ec88b6aae8b96bc55f`
- `Engine.dll` SHA-256
  `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`
- `Engine.u` SHA-256
  `b3661a1d2afb1f730ba5bb3cbd2a6716efaf55fd0d8cefa753b69026a8bc5a85`

`Engine.dll` class registration identifies `AZoneInfo::TexUPanSpeed` and
`TexVPanSpeed` at offsets `0x2e0` and `0x2e4`.

`Render.dll` function `URender::OccludeBsp` at `0x10b01140` determines which
side of each BSP node contains the view camera from `BspNode.Plane`. Positive
plane space selects `Node.Zone1`; non-positive plane space selects `Node.Zone0`.
It resolves the corresponding
`Model.Zones[zone].ZoneActor`, and groups visible surface work by both surface
and zone actor. A serialized surface reused by nodes in different zones can
therefore have different pan speeds in one frame.

When the selected model zone has no actor, `OccludeBsp` uses
`Level->Actors(0)` and verifies that actor is a `LevelInfo`. It does not search
the export table for the first object whose immediate class name is
`LevelInfo`.

The licensed SurrealEngine reference independently follows the same path in
`Render/VisibleFrame.cpp` and `Render/VisibleNode.cpp`: camera-side node zone,
`GetZoneActor`, then that actor's U/V pan speeds.

## Shipped map corpus

A read-only scan of the installed `Maps` directory found 255 auto-panned BSP
nodes across 41 maps. Eighty-eight select different zones on their two sides,
and nine auto-panned surfaces are shared by nodes with different front zones.
`Lev2_fire1` includes both patterns, so one pan speed stored per serialized
surface is observably insufficient.

## OpenHP1 mapping

OpenHP1 reads the active fallback from decoded `Level.Actors(0)`, carries both
node-side pan speeds and the node-plane normal with each triangulated BSP
vertex, and selects the speed from the active render-pass camera side in the
scene vertex shader. The existing renderer clock supplies the original 64
texture units per second.
