# Pain-zone lifecycle in the shipped Engine.dll

## Scope and primary inputs

This note checks the pawn pain-zone lifecycle against the legally owned HP1
installation without running the original executable. The inspected files are:

```text
7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc  res/System/Engine.dll
b3661a1d2afb1f730ba5bb3cbd2a6716efaf55fd0d8cefa753b69026a8bc5a85  res/System/Engine.u
```

`Engine.dll` is a 32-bit x86 PE with image base `0x10300000`. Its `.text`,
`.rdata`, and `.data` RVAs equal their file offsets in this image.
`Engine.u` is used only to identify the compiled property and callback policy
that the DLL deliberately dispatches.

## Confirmed native region-update behavior

The retained PE exports identify these native entry points:

| Export | RVA / file offset | Implementation VA / file offset |
|---|---:|---:|
| `ULevel::SetActorZone(AActor*,int,int)` | `0x4935` | `0x103acdd0` / `0xacdd0` |
| `UModel::PointRegion(AZoneInfo*,FVector)` | `0x3c33` | `0x1042c2a0` / `0x12c2a0` |
| `AActor::Tick(float,ELevelTick)` | `0x4205` | `0x103b3840` / `0xb3840` |
| `APawn::eventPainTimer()` | `0x49df` | `0x1031f580` / `0x1f580` |

`ULevel::MoveActor` invokes `SetActorZone` after committing movement. At
`0x103ab1a7..0x103ab1b3` it calls virtual slot `+0xb0`; the exported `ULevel`
UObject vtable at `0x10471160` contains the `SetActorZone` thunk
`0x10304935` in that slot.

`SetActorZone` uses `UModel::PointRegion` and copies all three members of each
returned 12-byte `FPointRegion` (`Zone`, `iLeaf`, and `ZoneNumber`):

- Actor `Region` at actor offset `+0xa0` is queried from `Location` at `+0xfc`
  (`0x103ad060..0x103ad110`).
- Pawn `FootRegion` at pawn offset `+0x274` is queried from
  `Location.Z - CollisionHeight`, where `CollisionHeight` is `+0x1d4`
  (`0x103ad156..0x103ad261`).
- Pawn `HeadRegion` at pawn offset `+0x280` is queried from
  `Location + (0,0,EyeHeight)`, where `EyeHeight` is `+0x358`
  (`0x103ad250..0x103ad2e8`).

Only the `Zone` pointer is compared when deciding whether to emit a change
event. A leaf or zone-number change with the same zone actor updates the stored
structure without emitting a zone-change event.

### Callback ordering and visible values

The event order is material to script behavior:

1. For an Actor `Region` zone change, the DLL calls
   `oldZone.ActorLeaving(actor)` and then `actor.ZoneChange(newZone)` while the
   actor's `Region` still contains the old value (`0x103ad0ba..0x103ad103`).
2. It stores the new 12-byte `Region` at `0x103ad109..0x103ad110`.
3. It then calls `newZone.ActorEntered(actor)` at
   `0x103ad11a..0x103ad13e`.
4. For a Pawn `FootRegion` zone change, it calls
   `pawn.FootZoneChange(newFootZone)` at `0x103ad226..0x103ad247` while
   `pawn.FootRegion` still contains the old region. It stores the new
   `FootRegion` at `0x103ad259..0x103ad261`.
5. For a Pawn `HeadRegion` zone change, it calls
   `pawn.HeadZoneChange(newHeadZone)` at `0x103ad2d0..0x103ad2d8` while
   `pawn.HeadRegion` is still old. It stores the new `HeadRegion` at
   `0x103ad2e0..0x103ad2e8`.

The relevant exported `FName` objects corroborate the dispatch sites:
`ENGINE_ActorLeaving` RVA `0x2e9e78`, `ENGINE_ActorEntered` RVA `0x2e9e7c`,
`ENGINE_ZoneChange` RVA `0x2e9e98`, `ENGINE_FootZoneChange` RVA `0x2e9e5c`,
and `ENGINE_HeadZoneChange` RVA `0x2e9e58`.

## PainTime initialization belongs to compiled Pawn script

`SetActorZone` does not write `PainTime`. It calls the compiled
`Pawn.FootZoneChange` callback before storing the new foot region, allowing the
script to compare the old `self.FootRegion` with the `newFootZone` argument.

The active `Engine.u` exports are `Pawn.PainTime` export 335 and
`Pawn.FootZoneChange` export 1978. Its decoded execution stream proves:

- when the old `FootRegion.Zone` is painful and either `newFootZone` is not
  painful or `HeadRegion.Zone` is a water zone, it assigns `PainTime = -1.0`;
- otherwise, when `newFootZone.bPainZone` is true, it assigns
  **`PainTime = 0.01`**.

The latter assignment is decoded execution offset `0x0309`: `Let`,
`InstanceVariable` export 335 (`PainTime`), `FloatConst 0.01`. The original
serialized bytes at Engine.u file offset `0x22923` are:

```text
0f 01 50 05 1e 0a d7 23 3c
```

Here `0f` is `Let`, `01 50 05` resolves to export 335, `1e` is
`FloatConst`, and little-endian `0x3c23d70a` is `0.01f`. Therefore `0.1` is
not the shipped initial delay.

## Confirmed native countdown and PainTimer dispatch

In the pawn-only, full-tick path of `AActor::Tick`, `PainTime` is the float at
pawn offset `+0x268`. The exact path at `0x103b45a1..0x103b45f9` is:

1. Require `PainTime > 0` (`0x103b45a1..0x103b45b2`).
2. Subtract the current `DeltaSeconds` and store the un-clamped result
   (`0x103b45b4..0x103b45bd`).
3. Compare that result with the float at VA/file offset
   `0x10473814`/`0x173814`. Its bytes are `6f 12 83 3a`, or approximately
   `0.001000000047f`.
4. If the result is below that threshold, write exactly `0.0` to `PainTime`,
   look up the exported `ENGINE_PainTimer` name at RVA `0x2e9e54`, and call
   the actor's virtual `ProcessEvent` (`0x103b45d0..0x103b45f9`).

The exported `APawn::eventPainTimer` wrapper independently resolves the same
`ENGINE_PainTimer` name and calls `ProcessEvent` at
`0x1031f580..0x1031f59b`.

The name anchor itself is byte-proven: UTF-16 `PainTimer` begins at file
offset `0x1d6f44` (VA `0x104d6f44`). The initializer at
`0x10392462..0x1039247e` constructs its `FName` and stores it to the exported
`ENGINE_PainTimer` global at `0x105e9e54`. The adjacent `HeadZoneChange` and
`FootZoneChange` strings initialize globals `0x105e9e58` and `0x105e9e5c`.

The active `Pawn.PainTimer` script is Engine.u export 1993. Its compiled
execution reads the foot, body, and head pain zones, derives immersion depth,
selects the zone damage type (defaulting to `ZonePain`), calls `TakeDamage`,
and, while Health remains positive, assigns `PainTime = 1.0`. Its separate
water path assigns `PainTime = 2.0`. These are script policy after each native
expiry; they are not native constants in `AActor::Tick`.

The native countdown occurs after the automatic physics call in the same tick:
`AActor::Tick` invokes virtual physics at `0x103b4331..0x103b434c`, then
reaches the pawn timer path beginning at `0x103b450e`. Movement can therefore
update regions and run `FootZoneChange` before that tick's pain countdown.

## Result for OpenHP1

The shipped artifacts confirm the shared lifecycle, but they also expose two
required corrections to a simplified implementation:

- the initial pain-zone delay is `0.01`, not `0.1`;
- the transition policy must run through the authored `FootZoneChange`
  callback before replacing `FootRegion`, because leaving pain and head-water
  cases assign `-1.0`, and entering one pain zone from another must not
  spuriously restart the timer.

Native expiry is also threshold-based: subtract `DeltaSeconds`, then dispatch
when the result is below approximately `0.001`, zeroing it first. Clamping and
testing equality with zero is behaviorally similar for ordinary positive frame
deltas but is not the exact binary contract.

## Reproduction commands

```sh
shasum -a 256 res/System/Engine.dll res/System/Engine.u
/opt/homebrew/opt/llvm/bin/llvm-readobj --file-headers --sections \
  res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-readobj --coff-exports \
  res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103acdd0 \
  --stop-address=0x103ad330 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103b4330 \
  --stop-address=0x103b4604 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1031f580 \
  --stop-address=0x1031f5a1 res/System/Engine.dll
xxd -g 4 -s 0x17380c -l 0x20 res/System/Engine.dll
cargo run -q -p openhp1-package --example package_inspect -- \
  res/System/Engine.u
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/Engine.u 1978
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/Engine.u 1993
```

The final float/property labels above were produced by a temporary read-only
probe using `openhp1-package` and `openhp1-script`; that probe was not retained.
No original executable was launched and no shipped file was modified.
