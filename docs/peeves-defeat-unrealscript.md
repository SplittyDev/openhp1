# Lev_Tut3b Peeves defeat evidence

The shipped script and compiled bytecode agree on the intended result: Peeves
does not remain in the room after the encounter. He switches to an exit patrol,
reaches a station marked `BH_Die`, and is destroyed there.

## Shipped evidence

- `res/System/Tut3.u`, `tut3Peeves.shot` (export 107), tests `hitCount <= 0`
  and enters `dieing`; otherwise it returns to `patrol`.
- `res/System/Tut3.u`, `tut3Peeves.dieing` (export 108), restores the camera and
  Harry's combat state, triggers `Peevesdies`, copies the three authored exit
  fields into the active patrol fields, selects the first exit path node, and
  enters `patrol`. Its compiled bytecode contains no `Destroy` call.
- `res/System/Tut3.u`, `tut3Peeves.patrol` (export 100), calls HP1 native
  `FindPath` (`0x229`), moves through the returned navigation points, and calls
  `PawnAtStation` when the active navigation point is the destination station.
- `res/System/Tut3.u`, `tut3Peeves.atStation` (export 106), compares the
  destination station's selected behavior with `BH_Die`. The true branch at
  decoded bytecode offset `0x00d2` calls native `0x117`, UE1 `Actor.Destroy`.
- `res/Maps/Lev_Tut3b.unr`, actor `tut3peeves2`, authors the exit as
  `HPath_F1` toward `baseStation1`. The selected `baseStation1.aiData` behavior
  is serialized as enum value 4.
- `res/System/HPBase.u`, `baseStation.EBehavior`, defines value 4 as `BH_Die`.

These observations were made with the repository's read-only package and
script decoders (`openhp1-package`'s `package_inspect` and `openhp1-script`'s
`script_inspect`). The original assets were inspected locally and were not
modified or copied into the repository.

## Consequence

An immediate hide or destroy on the final spell would not match the authored
state machine. If Peeves remains visible, execution failed to complete the
exit patrol and therefore never reached the existing `Destroy` branch.

## Retail navigation discrepancy

The local retail corpus does not provide a route from the configured first
exit node to the configured station:

- `HPath_F1` leads to `HPath_F2`, which leads to the terminal `HPath_F3`.
- `HPath_F3` has no serialized `Paths`, forced path, or pruned path property.
- `baseStation1` is linked to `HPath_A3` and `CutCameraPos18`, neither of which
  is connected to the `HPath_F` component.

The matching retail `Engine.dll` implementation of
`APawn::findPath(ANavigationPoint *&, AActor *, FName)` performs a bounded
depth-first traversal through each current navigation point's authored
`Paths`. It leaves the output null unless that traversal encounters an actor
whose object name is the requested destination. It does not synthesize a
spatial link or fall back to the closest reachable node. Consequently, this
exact `Lev_Tut3b.unr` and `Engine.dll` pair cannot drive Peeves into
`baseStation1` through `FindPath` either.

For reproducibility, the inspected files have these SHA-256 hashes:

- `Lev_Tut3b.unr`: `5b0f16ee99ffb8d88a1dc1dd1da5d3acf631a01409bf685e12b73850a3e905d7`
- `Engine.dll`: `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`
- `Tut3.u`: `9b7b777ec75acd8935f94b08fef68baa1956ef5f3c5c2ddadf7071a1a74dcff6`

Independent retail gameplay recordings
([HAFanForever](https://www.youtube.com/watch?v=QmgU2quJ8gA),
[Global Gaming](https://www.youtube.com/watch?v=PJI3BIm7t_g)) show Peeves
disappearing during the post-defeat camera sequence, but do not establish that
this dead `atStation` branch caused the disappearance. A forced navigation
edge, nearest-node fallback, or Peeves-specific destroy would therefore be a
compatibility workaround rather than a demonstrated engine semantic.
