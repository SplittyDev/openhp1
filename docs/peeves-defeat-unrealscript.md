# Lev_Tut3b Peeves defeat evidence

The shipped bytecode moves Peeves to `HPath_F1` after the encounter and then
leaves him in an idle loop under `PHYS_Flying`. The retail movement native
curves a non-strafing flyer toward that node and preserves his final
acceleration when the move completes, carrying him into the wall where he
becomes occluded. The station's `BH_Die` branch is not reached.

## Shipped evidence

- `res/System/Tut3.u`, `tut3Peeves.shot` (export 107), tests `hitCount <= 0`
  and enters `dieing`; otherwise it returns to `patrol`.
- `res/System/Tut3.u`, `tut3Peeves.dieing` (export 108), restores the camera and
  Harry's combat state, triggers `Peevesdies`, copies the three authored exit
  fields into the active patrol fields, selects the first exit path node, and
  enters `patrol`. Its compiled bytecode contains no `Destroy` call.
- `res/System/Tut3.u`, `tut3Peeves.patrol` (export 100), calls HP1 native
  `FindPath` (`0x229`) at bytecode offset `0x016f` and immediately calls
  `MoveTo(navP.Location)` at `0x017c`. The embedded source's null-result guard
  is commented out and the executable bytecode contains no such branch.
- `FindPath(HPath_F1, baseStation1)` returns null, but the unconditional move
  still targets the current `navP`, `HPath_F1`. On arrival, `destP != navP`,
  `navP` becomes null, and the authored `bLoopPath=false` keeps Peeves in the
  idle animation loop permanently.
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

An immediate hide or destroy on the final spell would not match the executable
runtime path. `HPath_F2`, `HPath_F3`, `PawnAtStation`, and the existing
`Destroy` branch are dead on this exit. Retail disappearance instead depends on
the flying pawn retaining movement after `MoveTo(HPath_F1)` completes.

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

## Retail movement completion

The matching `Engine.dll` implementation of `APawn::moveToward` treats
`PHYS_Swimming` and `PHYS_Flying` specially when `bCanStrafe` is false. While
the move is active, it derives acceleration from the pawn's current rotation
rather than the normalized destination delta; the normal rotation poll still
turns the pawn toward the destination. This produces a curved approach rather
than allowing the pawn to strafe directly toward the node. Flying arrival
requires less than 16 horizontal units to the destination and a vertical
separation below `max(48, CollisionHeight)`. On success, the non-strafing
flying/swimming branch bypasses the acceleration-zeroing block; `PollMoveTo`
then clears only the latent-action field. Strafing flyers use direct
destination acceleration and clear it on arrival.

The flag identification is encoded in the shipped data and binary. `Pawn`'s
embedded declaration lists nine packed booleans before `bCanStrafe`, making it
bit `0x200`; `APawn::moveToward` tests bit `0x200` of the Pawn flag word before
selecting the flying/swimming branch. Neither `Tut3.u` nor `Lev_Tut3b.unr`
overrides `bCanStrafe` for Peeves, so he uses the inherited false default.

Peeves also sets `bCollideWorld=false` before entering the exit patrol. His
curved approach leaves a final tangent aimed left of the direct route, and the
retained acceleration and velocity carry him beyond `HPath_F1` into the wall
while the script remains in its idle loop. OpenHP1 previously accelerated him
directly at `HPath_F1`; retaining that incorrect straight-line acceleration
made him pass through the castle and continue indefinitely.

For reproducibility, the inspected files have these SHA-256 hashes:

- `Lev_Tut3b.unr`: `5b0f16ee99ffb8d88a1dc1dd1da5d3acf631a01409bf685e12b73850a3e905d7`
- `Engine.dll`: `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`
- `Tut3.u`: `9b7b777ec75acd8935f94b08fef68baa1956ef5f3c5c2ddadf7071a1a74dcff6`

## Mounted EFG retail comparison

The read-only CD-ROM volume `HARRY_POTTER_EFG` contains an earlier, distinct
retail copy. Its English, French, and German readmes identify version 1.0 and
are dated October 17, 2001. `setup/Setup.ini` selects English (`0x0009`) by
default and also lists French (`0x040c`) and German (`0x0007`);
`autorun/autorun.cfg` likewise enables exactly those three languages and marks
the product as non-demo.

The relevant mounted files are all distinct from the local corpus:

- `Maps/Lev_Tut3b.unr`: 2,380,807 bytes, mtime October 22, 2001 03:54:46
  `+0700`, SHA-256
  `f7e112899cbc5125466f0ceb06ba4e4847cabe0fc7768cb748bd02ea4b5c7095`.
- `System/Tut3.u`: 26,353 bytes, mtime October 22, 2001 04:05:50 `+0700`,
  SHA-256
  `d89538aa1a747639b2217ff9c4c476ea9a17ca4cc297473df358ee91d3b8e5ca`.
- `System/Engine.dll`: 2,121,728 bytes, mtime October 22, 2001 03:48:16
  `+0700`, SHA-256
  `9207af078045adbd672adfa54f91b177013b80afbd730c246a87c19e2ecf6d0e`.
  Its PE timestamp is October 22, 2001 11:32:53, and it has no PE resource
  directory or embedded version resource.

Both mounted packages are Unreal package version 76, licensee version 0. The
mounted map has the same relevant authored data as the local map:
`tut3peeves2` selects `HPath_F`, `HPath_F1`, and `baseStation1`; the only
reachable exit-path component is `HPath_F1`--`HPath_F2`--`HPath_F3`; and
`baseStation1` remains in the separate component linked to `HPath_A3` and
`CutCameraPos18`. The mounted `Tut3.u` exports 100, 106, 107, and 108 decode to
the same `patrol`, `atStation`, `shot`, and `dieing` bytecode as the local
package. All nine serialized `tut3Peeves` class-default properties and values
also match, including `hitCount = 4.0`.

The DLL differs as a whole, but not in this native. On the mounted DLL, the
exported `APawn::findPath` thunk is at `0x1030354e` and jumps to
`0x10402220`; on the local DLL it is at `0x10303558` and jumps to
`0x10402540`. Each implementation body and exception tail is `0x25e` bytes.
After stripping instruction addresses, symbol comments, and relocated absolute
addresses, both disassemblies have SHA-256
`6d5c2008743e8291aa6195e21d02a224d0b2a12c770d1fac54b943af3662ed00`.
The instruction streams use the same 100-entry traversal arrays, authored
`Paths` walk, visited-node suppression, backtracking, destination-name
comparison, and null-output failure path.

The primary comparisons can be repeated without writing either corpus:

```sh
shasum -a 256 /Volumes/HARRY_POTTER_EFG/{Maps/Lev_Tut3b.unr,System/Tut3.u,System/Engine.dll}
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-package \
  --example package_inspect -- /Volumes/HARRY_POTTER_EFG/System/Tut3.u
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- /Volumes/HARRY_POTTER_EFG/System/Tut3.u 100
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- /Volumes/HARRY_POTTER_EFG/System/Tut3.u 106
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- /Volumes/HARRY_POTTER_EFG/System/Tut3.u 107
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- /Volumes/HARRY_POTTER_EFG/System/Tut3.u 108
objdump -x /Volumes/HARRY_POTTER_EFG/System/Engine.dll
objdump -d --start-address=0x10402220 --stop-address=0x1040247e \
  /Volumes/HARRY_POTTER_EFG/System/Engine.dll
```

This second retail revision confirms that no missing navigation edge or
alternate Peeves bytecode explains the exit. Adding a navigation edge,
nearest-node fallback, or Peeves-specific destroy would remain an authored-data
workaround rather than the shared movement behavior used by retail.

Independent retail gameplay recordings
([HAFanForever](https://www.youtube.com/watch?v=QmgU2quJ8gA),
[Global Gaming](https://www.youtube.com/watch?v=PJI3BIm7t_g)) show Peeves
continuing beyond the stair waypoint and disappearing into the wall during the
post-defeat camera sequence. Quarter-second inspection shows him turning left
before he vanishes. That motion agrees with the shipped native's non-strafing
curved approach and retained final acceleration; it does not require another
waypoint or the dead `atStation` branch.
