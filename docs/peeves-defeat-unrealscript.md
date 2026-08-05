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
