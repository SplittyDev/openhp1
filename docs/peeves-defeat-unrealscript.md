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

This second retail revision therefore does not prove a shared runtime bug or
an exact runtime fix. Its authored graph and native `findPath` still cannot
reach `baseStation1`. Adding a navigation edge, nearest-node fallback, or
Peeves-specific destroy would remain an unproven compatibility workaround;
the retail disappearance must involve some other behavior not established by
these assets.

Independent retail gameplay recordings
([HAFanForever](https://www.youtube.com/watch?v=QmgU2quJ8gA),
[Global Gaming](https://www.youtube.com/watch?v=PJI3BIm7t_g)) show Peeves
disappearing during the post-defeat camera sequence, but do not establish that
this dead `atStation` branch caused the disappearance. A forced navigation
edge, nearest-node fallback, or Peeves-specific destroy would therefore be a
compatibility workaround rather than a demonstrated engine semantic.
