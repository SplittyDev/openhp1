# Latest in-game reports: original-engine evidence

This note covers the two newest reports in the effective macOS report
directory. `OPENHP1_SETTINGS_DIR` was unset, so the directory was
`~/Library/Application Support/OpenHP1/Reports`.

| Capture time (Asia/Bangkok) | Report | Symptom |
| --- | --- | --- |
| 2026-08-13 23:26:21 | `report-1786638381-118618000.md` | the Golden Snitch seems slow and cannot be caught |
| 2026-08-13 23:23:58 | `report-1786638238-389477000.md` | Hagrid is standing in the air |

The evidence below comes from the captured runtime state, the shipped map and
script packages, and disassembly of the shipped `Core.dll`/`Engine.dll`.
SurrealEngine is used only as a licensed cross-check. No retail asset or
extracted script is reproduced here.

## Quidditch: failed catch logic from eager `&&`

### Captured and authored state

The `Lev2_Quid1.unr` report has one deferred runtime call:
`QuidditchReferee0.Tick` failed because index 8 was used on an array of length
8. That actor owns the active match progress and catch transition. OpenHP1
then records a deterministic Tick failure and does not run that Tick again
until a state or event change, so the referee can no longer complete the
authored catch path.

The map authors `Snitch1` as `Hub2.Snitch` (map export 587). Its own properties
include four path references, `fLaunchSpeed=300`, and
`bSwitchPathsOnTrigger=true`. The compiled `Snitch` and `QuidditchPawn`
startup path selects `PHYS_Flying`, chooses an authored interpolation path,
then hands it to an `InterpolationManager`; routine path motion uses the
authored interpolation-point speeds. `fLaunchSpeed` is used for launch/pursuit
behavior, not as a universal path-speed override.

### Exact original VM evidence

`Hub2.u` (SHA-256
`b44c845961a45d6b34577a59309c569c4c8236ec9ff7f7bb82526e7f499e39d1`)
compiles `QuidCommentator.SayComment` as export 377. Its variant scan is
bounded by `Variant < 8 && Variant[Variant].DlgName != ""`. At bytecode offset
`0x0299` the compiler emits native 130 (`AndAnd_BoolBool`); after the left
comparison it emits `EX_Skip` at `0x02a3`, and only then the array expression
at `0x02a6`. The skip is executable short-circuit metadata, not padding.

The shipped `Core.dll` (SHA-256
`60f441ee152e13fa79de481901645ddc65638b97142e2e3570c1e76e3de8c788`)
exports an `execAndAnd_BoolBool` thunk at `0x10103661` whose implementation is
`0x10136350`. It evaluates the left operand at `0x10136359..0x10136371`, reads
the encoded 16-bit skip and advances past its three-byte header at
`0x10136378..0x10136388`, and evaluates the right operand only when the left
operand is true (`0x1013638d..0x101363a2`). When false,
`0x101363cd..0x101363db` adds the skip count to the right-expression start,
writes false, and stores the advanced instruction pointer without executing
the array access. Licensed SurrealEngine independently implements natives 130
and 132 by lazy operand evaluation in `ExpressionEvaluator.cpp`.

OpenHP1 instead consumes the `EX_Skip` count and immediately evaluates its
child expression before dispatching scalar native 130. When `Variant` reaches
8, it therefore evaluates `Variant[8]` even though `Variant < 8` is false. The
capture's exact exception follows directly.

### Shared fix seam and remaining limit

The authorized fix is at VM expression evaluation: native 130 (`&&`) and
native 132 (`||`) must evaluate the right encoded expression only when the
left result requires it, while advancing the instruction pointer by the
compiled `EX_Skip` distance otherwise. Synthetic regressions should cover
`false && failing_rhs` and `true || failing_rhs`, including preservation of
the post-expression instruction pointer. No Quidditch actor or array special
case is warranted.

This fixes the proven catch blocker. The report contains no nearby Snitch
snapshot, interpolation point, manager state, or measured velocity, so it
does not prove a separate path-speed defect. Perceived slow flight remains an
unresolved observation and must not be "fixed" by tuning `fLaunchSpeed` or a
map value.

## Hagrid: non-retail `SetLocation` placement grid

### Captured and authored state

In `Lev2_fire1.unr`, `H2Hagrid0` (map export 1169) is captured at
`(6709.4, 2975.7, -237.0)`, playing its authored idle `Breathe` animation.
`CutMark22` (map export 1050) is at `(6709.4, 2975.7, -307.0)`. Thus Hagrid's
X/Y exactly equal the marker and his Z is exactly `70` higher. The actor's
serialized collision height is exactly 70; it also has radius 40,
`DrawScale=1.2`, initial `PHYS_Falling`, and authored map location
approximately `(6336.43, 3470.28, -310.4)`. `H2Hagrid` inherits the shared
`baseChar` cutscene behavior and does not override placement or physics.

The authored caller is `Lev2_fire1.CutScene5` (map export 2035): cast 2 is
Hagrid, and `Cast2Script[0]` is exactly `teleport cutmark22`, followed by
`face cutmark11` and `setidle breathe`. Compiled `CutScene.handleCast`
(HPBase export 3563) implements `TELEPORT` by passing the resolved marker's
location to native 267 `Actor.SetLocation`. The report's marker X/Y, exact
collision-height Z delta, and subsequent `Breathe` animation therefore record
the result of that direct native placement path rather than a walking or
timeout inference.

Hagrid's shipped skeletal mesh has Z bounds about `0.268..125.709`. Retail
bottom alignment for a colliding, non-`PHYS_None` skeletal actor applies

```text
(Origin.Z - Bounds.Min.Z) * Mesh.Scale.Z * DrawScale
    - CollisionHeight - 2.5
```

which is about `-72.82` for this actor. At the captured center, that puts the
visible feet near Z `-309.5`, about 57.5 units above the local BSP floor near
Z `-367.0`; the reported visual symptom agrees with the captured transform.

### Exact original placement evidence

The shipped `Engine.dll` (SHA-256
`7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`)
routes `FarMoveActor` through thunk `0x10304bbf` to `0x103a9de0`. It obtains
the cylinder extent and calls virtual `FindSpot` at `0x103a9f4a`.
`FindSpot` (`0x103030b7 -> 0x103a9690`) is not a first-free cube grid:

1. `0x103a98c1..0x103a99ce` tries negative and positive adjustment along each
   extent axis by calling virtual `AdjustSpot` six times;
2. `0x103a9a62` performs `SinglePointCheck`;
3. only if that fails, `0x103a9a99..0x103a9b3d` explores the eight signed
   corners, still through `AdjustSpot` rather than accepting a raw offset.

`AdjustSpot` (`0x10302cf2 -> 0x103a9570`) calls `SingleLineCheck`. On a hit it
changes the spot by `Hit.Normal * (1.05 - Hit.Time) * Size`
(`0x103a95d3..0x103a963a`; the shipped 1.05 constant is at `0x104770dc`). At
the captured marker, a vertical cylinder probe toward that floor has a hit
fraction of about `0.87146`, so this component is about 12.5 units, not 70.
Nearby side geometry means the complete retail result must be obtained by
running all sequential adjustments; a guessed final coordinate is not
authorized.

OpenHP1's shared `set_actor_location_placing` instead tests the requested
point and then a `[0, +1, -1]` cube of offsets scaled by
`max(CollisionRadius, CollisionHeight)`. For Hagrid that scale is 70 and the
first accepted candidate is the unchanged X/Y plus Z 70 seen in the report.
That exact fingerprint cannot be produced by the shipped adjustment
algorithm.

### Shared fix seam and remaining limit

The authorized fix is to replace the cube-grid approximation behind native
`Actor.SetLocation`/shared placement with the retail `FindSpot` plus
`AdjustSpot` sequence: the same axis order, trace types, hit-fraction normal
correction, final point check, and corner fallback. Tests should use synthetic
BSP geometry to prove a partially penetrating cylinder is corrected by the
trace result instead of being displaced by one full extent. Hagrid, this map,
and `CutMark22` must not appear in the implementation.

The capture and authored script prove the wrong direct-teleport placement
algorithm but do not provide the complete original sequence of collision
traces or an exact retail final coordinate. Therefore they do not authorize a
hard-coded final location, a Hagrid render offset, or changes to the already
evidenced retail skeletal bottom-alignment formula.

After implementing that shared sequence, a read-only local-corpus replay of
native placement at `CutMark22` returns Hagrid center Z `-294.5`. Applying the
existing retail skeletal adjustment puts his feet near Z `-367.3`, matching
the local BSP floor near Z `-367.0`. This is OpenHP1 replay evidence, not a
claim that retail was directly observed at that exact center coordinate.

## Reproduction anchors

```sh
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/Hub2.u 377
xcrun llvm-objdump --disassemble --x86-asm-syntax=intel \
  --start-address=0x10136350 --stop-address=0x10136440 res/System/Core.dll
xcrun llvm-objdump --disassemble --x86-asm-syntax=intel \
  --start-address=0x103a9570 --stop-address=0x103a9690 res/System/Engine.dll
xcrun llvm-objdump --disassemble --x86-asm-syntax=intel \
  --start-address=0x103a9690 --stop-address=0x103a9c24 res/System/Engine.dll
xcrun llvm-objdump --disassemble --x86-asm-syntax=intel \
  --start-address=0x103a9de0 --stop-address=0x103aa0e4 res/System/Engine.dll
```
