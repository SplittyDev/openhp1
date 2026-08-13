# Lev2_Fire2 rolling-log initial placement investigation

Status: reopened (2026-08-13). A supplied recording directly confirms that an
original PC retail installation shows the log correctly from the beginning.
Its exact regional build is unknown; see section 21. That observation
falsifies the prior static-trace conclusion that every retail build must bury
the log. The native formula and authored values below remain established, but
at least one retail lifecycle/render or version-specific condition is still
missing from the trace. No code change is justified until that condition is
identified.

## Constraints

- Do not run the original game, Wine, or Crossover.
- Do not patch `rolllog1`, `Lev2_Fire2`, an actor name, a map coordinate, or an
  authored property pattern.
- Do not change engine behavior without evidence from the shipped packages,
  shipped UnrealScript/bytecode, or `Engine.dll`.
- Treat the collision/visual disagreement as a clue, not as permission to move
  either the collision or the actor to a guessed location.

## Primary inputs

- Shipped map: `res/Maps/Lev2_Fire2.unr`.
- Shipped script packages under `res/System` (exact package/export references
  are resolved below).
- Shipped native binary `Engine.dll` (the completed native trace is recorded
  below).
- OpenHP1 report:
  `~/Library/Application Support/OpenHP1/Reports/report-1786541150-533200000.md`.
  This report is local diagnostic output, not original-engine evidence; it is
  used only to identify the affected authored objects and OpenHP1's observed
  state.

## Evidence log

### 1. Locate and read the in-game report

Repository discovery began with Semble, as required:

```text
mcp__semble__search
  repo=/Users/splitty/Developer/OpenHP1
  query="in-game report command captures current level nearby actors and writes markdown Reports directory"
  content=code
```

Semble located
`crates/openhp1-game/src/app/console/commands/report.rs:22`. Reading the
function established that reports are written below the active settings
directory in `Reports/`; the report includes every named actor within 2048
Unreal units and records package path, zero-based export index, class,
location, rotation, mesh, and animation.

The report directory was ordered by modification time without launching any
game:

```sh
find "$HOME/Library/Application Support/OpenHP1/Reports" -maxdepth 1 \
  -type f -name '*.md' -exec stat -f '%m %Sm %N' \
  -t '%Y-%m-%d %H:%M:%S' {} \; | sort -nr | head -n 20
```

Newest report: `report-1786541150-533200000.md`, captured 2026-08-12
20:25:50 local time. Its issue text is: "The tree log here is not visible
until Flipendo is cast on it."

Concrete report evidence:

| Item | Value |
| --- | --- |
| Shipped map filename | `Lev2_Fire2.unr` |
| Player location | `(-20.0, -928.0, 45.0)` |
| Affected actor | `rolllog1` |
| Scene actor index | 150 (runtime-local, not a package identity) |
| Shipped map export index | 2132 (zero based) |
| Class | `rolllog` |
| Runtime location before the spell | `(-2.6, -1125.6, 6.9)` |
| Runtime rotation before the spell | `(pitch 0, yaw -16384, roll 0)` |
| Draw state | not hidden, draw type 2, mesh `sklogMesh` |
| Animation | `Stop`, phase 0.269, rate 5.000 |
| Player-to-actor distance | 202.0 Unreal units |

The report therefore identifies `Lev2_Fire2.rolllog1` / map export 2132 as
the exact log. OpenHP1 already holds a plausible world `Location` for it and
uses that location for collision, while the screenshot shows the skeletal
mesh below the floor. That narrows the package investigation to visual
transform inputs and initialization ordering; it does **not** yet establish
which one is wrong.

### 2. Initial hypotheses and their status

| Hypothesis | Status | Reason |
| --- | --- | --- |
| The map authored the log's center/collision box overlapping the floor | confirmed input, not a complete cause | The two local Version 1.0 maps share location Z 6.8568, box half-height 89.8914, and a floor near Z 0. The retail recording proves that an original PC build can still render the log visibly, but its exact map revision is unknown. |
| The whole actor is initialized at a map-specific wrong location | rejected as a fix hypothesis | The reported logical location exactly matches the shipped map property and collision is at the intended obstruction. Reauthoring or offsetting it would be a workaround. |
| The actor is hidden until Flipendo | contradicted by report | `hidden=false`, `DrawType=2`, mesh `sklogMesh` before the spell. |
| The mesh has a bad static asset origin | rejected | The compiled mesh origin is zero and the retail `GetMeshCoords` path explicitly incorporates its bounds; changing the asset origin would contradict the shipped data/native contract. |
| Initial `Stop` pose differs vertically from `Roll` | rejected | Shipped `Stop` is constant, both sequence phase-zero poses have nearly identical Z bounds, and both report zero extracted root motion. |
| Flipendo's script uniquely changes physics from none to walking | rejected | Compiled `waitforspell` and inherited `patrol` both contain `SetPhysics(PHYS_Walking)`. Native state/tick ordering permits none only before the first effective new-level actor tick; it cannot persist until Flipendo through stasis, tick exclusion, or walking physics. |

### 3. Package/tool discovery

Semble was queried for an existing package/actor/default/bytecode inspection
path before adding any tool:

```text
mcp__semble__search
  repo=/Users/splitty/Developer/OpenHP1
  query="example CLI dump Unreal package export actors properties class defaults bytecode inspect map actor"
  content=code
```

It located the actor decoder at `crates/openhp1-map/src/actor.rs` and script
decoder at `crates/openhp1-script`. An exact filename listing then confirmed
the already-existing examples:

- `crates/openhp1-package/examples/package_inspect.rs`
- `crates/openhp1-script/examples/script_inspect.rs`
- `crates/openhp1-scene/examples/actor_scan.rs`
- `crates/openhp1-scene/examples/runtime_scan.rs`

The existing examples will be reused where sufficient. A focused read-only
probe may be added only if none exposes the authored tagged properties and
class/default references needed here.

### 4. Prior investigation trail located and rechecked

`docs/rolllog-retail-alignment.md` was found after the report had identified
the same actor. That older trail contains a substantial shipped-binary trace.
It is retained rather than duplicated wholesale; its central package claims
were re-run in this investigation with the surviving read-only probe at
`/tmp/rollprobe`.

Commands:

```sh
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/Maps/Lev2_Fire2.unr props 2132
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/Maps/Lev2_Fire2.unr 2132
```

The instance's serialized object stack is:

```text
function = HarryPotter.rolllog class import
state = HarryPotter.rolllog class import
probe_mask = 0xffffffffffffffff
latent_action = 0
bytecode_offset = -1
```

This is the UE1 initial-state sentinel, not a saved suspended instruction in a
running state. It permits normal auto-state selection. The actor's resolved
serialized properties are:

```text
stationDestination = baseStation5
firstPath = HPath_A1
Tag = rolllog
Location = (-2.5671997, -1125.587, 6.8567886)
Rotation = (pitch 0, yaw -16384, roll 0)
OldLocation = (0, -1136, 23.6)
SpecularGlow = 0.5
AmbientGlow = 20
SpecularWidth = 160
CollisionRadius = 40
CollisionWidth = 115
CollisionHeight = 89.891426
CollideType = 2
```

There is no per-instance `Physics`, `PrePivot`, `Mesh`, `DrawType`,
`DrawScale`, `AnimSequence`, or animation-frame/rate override. `OldLocation`
is serialized actor history and is not the active location used for rendering
or collision.

The actor links directly to an authored route rather than to a separate log
controller:

| Object | Map export | Authored location | Relevant link |
| --- | ---: | --- | --- |
| `rolllog1` | 2132 | `(-2.5672, -1125.5870, 6.8568)` | `firstPath=HPath_A1`, `stationDestination=baseStation5` |
| `HPath_A1` | 432 | `(1.7601, -1302.5026, 43.1883)` | first navigation point |
| `baseStation5` | 276 | `(0.1974, -1475.4979, 43.2868)` | destination station, yaw `-16384` |

The exact engine/native evidence already recorded in
`docs/rolllog-retail-alignment.md` remains applicable:

- `CollideType=2` selects `UBox`, whose symmetric extent is
  `(CollisionRadius, CollisionWidth, CollisionHeight)` around `Location`.
- The map's horizontal floor is about `Z=1`; the authored collision box at
  center `Z=6.8568`, half height `89.8914`, begins overlapped with that floor.
- `USkeletalMesh::GetMeshCoords` computes the retail bottom-alignment
  adjustment
  `(Origin.Z - Bounds.Min.Z) * Mesh.Scale.Z * DrawScale
  - CollisionHeight - 2.5` when `bAlignBottom`, `bCollideWorld`, non-none
  physics, and non-shape collision apply.
- For this log the raw adjustment is about `-92.4435`; later retail
  output applies that negative adjustment to world Z. The complete native
  composition is documented in section 8. A patch that simply reverses this
  sign is contradicted by `Engine.dll`.
- `SetPhysics(PHYS_Walking)` calls `FindBase` but does not itself rewrite
  `Location`; `MoveActor` contains no generic depenetration/nudge.

This recheck is important because it rejects four tempting workarounds:
changing the authored location, applying collision-height placement to all
actors, reversing the retail mesh-adjust sign, or adding an unconditional
startup upward nudge.

### 5. Exact class, defaults, mesh, and stop-pose evidence

The map's class import resolves to `HarryPotter.u` export 839, class
`rolllog`. Its compiled inheritance/default chain is
`HarryPotter.rolllog -> HPBase.baseChar -> Engine.Pawn -> Engine.Actor`.

Commands:

```sh
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u 839 1366
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u classprops 839
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u classprops 1
```

`rolllog`'s compiled own defaults include:

```text
bFlipTarget = true
walkAnimName = Roll
idleAnimName = Stop
GroundSpeed = 200.0
eVulnerableToSpell = byte 13 (SPELL_Flipendo in the shipped enum)
bGestureOnTargeting = true
DrawType = DT_Mesh (2)
Mesh = HPModels.sklogMesh
CollisionRadius = 0
CollisionHeight = 0
CollideType = CT_Shape (3)
bProjTarget = true
```

The zero collision dimensions and `CT_Shape` above are class values; map
export 2132 intentionally overrides them with its nonzero box. `Actor` supplies
`bAlignBottom=true` and `SizeModifier=1.0`; `Pawn` supplies
`bCollideWorld=true`. Neither `rolllog` nor `baseChar` disables those flags.

#### Independent effective `bAlignBottom` gate verification

Because a false effective `bAlignBottom` would disable the entire retail mesh
adjustment, this was independently checked from raw compiled tagged defaults,
not inferred from the prior investigation document.

```sh
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u import 5
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPBase.u import 11
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u classprops 1
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u classprops 0
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPBase.u classprops 4
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u classprops 839
```

The compiled superclass references resolve exactly as:

```text
HarryPotter.u export 839 rolllog
  base Import(5) -> HPBase.baseChar (HPBase.u export 4)
HPBase.u export 4 baseChar
  base Import(11) -> Engine.Pawn (Engine.u export 0)
Engine.u export 0 Pawn
  base Export(1) -> Engine.Actor (Engine.u export 1)
```

Raw own-default tags along the chain:

| Layer | Relevant own tags |
| --- | --- |
| `Engine.Actor` export 1 | `bAlignBottom Bool bool=Some(true) bytes=[]`; `DrawScale Float bytes=00 00 80 3f` (=1); `CollisionHeight Float bytes=00 00 b0 41` (=22) |
| `Engine.Pawn` export 0 | `bCollideWorld Bool bool=Some(true) bytes=[]`; no `bAlignBottom` tag |
| `HPBase.baseChar` export 4 | no `bAlignBottom`, `bCollideWorld`, `Physics`, `PrePivot`, `CollideType`, `CollisionHeight`, or `DrawScale` tag |
| `HarryPotter.rolllog` export 839 | `CollisionHeight Float bytes=00 00 00 00` (=0), `CollideType Byte bytes=03`; no `bAlignBottom`, `bCollideWorld`, `Physics`, `PrePivot`, or `DrawScale` tag |
| `Lev2_Fire2.rolllog1` export 2132 | `CollisionHeight Float bytes=69 c8 b3 42` (=89.891426), `CollideType Byte bytes=02`; no `bAlignBottom`, `bCollideWorld`, `Physics`, `PrePivot`, or `DrawScale` tag |

UE1 tagged defaults are inherited unless a descendant serializes an override.
No descendant in this exact chain does. Therefore `rolllog1`'s effective
`bAlignBottom` is **true**, its effective `bCollideWorld` is **true**, and its
effective initial serialized physics is inherited `PHYS_None`. Its auto-state
bytecode requests `PHYS_Walking`; the exact point at which retail processes
that leading state code relative to the observed first draws remains open.
The compiled walking requests on both sides of Flipendo are in section 6.

The compiled `Mesh` default is **not** the same-package duplicate at
`HarryPotter.u` export 1853. Its import chain is
`HPModels.sklogMesh`, shipped as `HPModels.u` export 609, with default animation
`HPModels.u` export 496 (`sklogAnims`). This was verified with:

```sh
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u import 351
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPModels.u 609 496
```

The actual mesh has one bone (`Cylinder02`), origin `(0,0,0)`, unit scale,
and bounds:

```text
min = (-44.720745, -127.06107, 0.052073475)
max = ( 45.118603,  126.938896, 89.891426)
```

The animation asset defines:

| Sequence | Frames | Nominal rate |
| --- | ---: | ---: |
| `Roll` | 0..59 | 30 fps |
| `Stop` | 0..5 | 30 fps |

A read-only representative sampling probe used the existing
`Mesh::sample_skeletal_vertices` API:

```sh
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPModels.u sample 609 496
```

`Stop` is geometrically constant at every sampled phase (0, .25, .5, .75,
.999), with local Z bounds `0.046028..89.89747` and zero root motion. `Roll`
also has zero extracted root motion; its phase-zero bounds are
`0.045731..89.89721`, essentially the same initial pose. Therefore the spell
does not reveal the log because `Roll` contains an authored root-height jump,
and a missing initial animation pose is not supported: OpenHP1's report already
records `Stop` as active before the spell.

### 6. Exact pre- and post-Flipendo script/state transition

The relevant shipped exports are:

| Package export | Symbol | Role |
| --- | --- | --- |
| `HarryPotter.u:1366` | `rolllog.waitforspell` | auto initial state |
| `HarryPotter.u:1370` | `waitforspell.TakeSpellEffect` | accepts Flipendo and changes state |
| `HPBase.u:2660` | `baseChar.patrol` | inherited state entered after the spell |

Read-only bytecode commands:

```sh
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/HarryPotter.u 1366
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/HarryPotter.u 1370
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/HPBase.u 2660
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u import 304
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u name 101
```

The shipped embedded UnrealScript text is a useful label for the compiled
tokens, but the claims below were checked against bytecode:

```uc
auto state waitforspell
{
    function bool TakeSpellEffect(baseSpell spell)
    {
        if (spell.class == class'spellflip')
        {
            PlaySound(sound'HPSounds.Hub2_sfx.big_boulder_roll');
            gotostate('patrol');
            return true;
        }
    }
Begin:
    SetPhysics(PHYS_walking);
    loopanim('stop');
Loop:
    sleep(1);
    goto 'Loop';
}
```

Compiled confirmation:

- `waitforspell` bytecode offset `0x0000` is extended native 3970
  `SetPhysics`, with `ByteConst 1` (`PHYS_Walking`); offset `0x0005` is native
  260 `LoopAnim`, with `NameConst Stop`.
- `TakeSpellEffect` first compares the argument's class against the compiled
  `ObjectConst -305`, which resolves by UE1's signed object-reference encoding
  to `HarryPotter.u` import 304, class `HPBase.spellFlip`. On success it calls
  native 264 `PlaySound`, then
  native 113 `GotoState` with `NameConst` index 101, resolved from the shipped
  name table as `patrol`, and returns true.
- The inherited `baseChar.patrol` begin code enables `Tick`, calls native 3970
  `SetPhysics(PHYS_Walking)` again, calls virtual `startup`, then (because this
  instance has `firstPath=HPath_A1`) calls virtual 401
  `patrolPlayWalkAnim`. Its shipped implementation is
  `LoopAnim(walkAnimName)`, and `rolllog`'s compiled `walkAnimName` resolves to
  `Roll`.
- The state then resolves `findPath(navP, stationDestination)` and executes
  latent `MoveTo(navP.Location)`. Thus the first post-spell physical change is
  movement from the authored location toward `HPath_A1` at Z 43.1883, followed
  eventually by `baseStation5` at Z 43.2868. It is not a script `SetLocation`,
  `PrePivot`, collision-height, or mesh-origin correction.

Most importantly, the compiled scripts do not make walking unique to the spell
boundary: both the pre-spell auto state and post-spell patrol state explicitly
request `PHYS_Walking`. The deterministic bytecode changes are `Stop -> Roll`,
state, sound, and commencement of native `MoveTo` along the authored
navigation path. This does not by itself prove the exact live pre-draw Physics
value; the reopened native trace now treats that timing as unresolved.

This rules out a proposed fix that keys visual alignment on the spell, on
`waitforspell` versus `patrol`, or on a `PHYS_None -> PHYS_Walking` transition.
Any correct engine fix must explain why the same walking actor and retail
bottom-alignment contract are represented differently before its first
movement update.

### 7. What the "Flipendo target" actually is

No separate map-authored target actor is attached to `rolllog1`. The target
effect is runtime-spawned by shipped `HPBase.target` code.

The relevant compiled functions were also decoded, rather than relying only
on the embedded source labels:

```sh
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/HPBase.u 2338  # Target.seeking.LockOn
cargo run -q -p openhp1-script --example script_inspect -- \
  res/System/HPBase.u 2385  # Target.DrawSpellFX
```

`LockOn`'s bytecode begins with a context call to extended native 286
`GetWorldCollisionBox(True)`, then compiled vector arithmetic over the returned
local `TargetArea`; later it dispatches the three target-lock functions and
the virtual DrawSpellFX call. `DrawSpellFX` contains the context spawn native,
then context calls for owner, physics, and the remaining particle fields. This
matches the executable statements below; they are active compiled behavior,
not merely comments preserved in a text buffer.

The compiled/default facts are:

- `rolllog.bProjTarget=true`, `bFlipTarget=true`,
  `eVulnerableToSpell=SPELL_Flipendo`, and `bGestureOnTargeting=true` make the
  log itself the spell victim.
- `target.LockOn(victim)` calls native `victim.GetWorldCollisionBox(true)`.
- It computes
  `TargetCentre = midpoint(TargetArea) + victim.CentreOffset` and target width,
  height, and depth from that collision box. `Actor.CentreOffset` defaults to
  zero and `SizeModifier` defaults to one; `rolllog1` has no overrides.
- `DrawSpellFX` spawns the selected spell's `GestureParticleEffectClass` at
  that computed `HitLocation`, sets its owner to the victim, assigns
  `PHYS_Trailer`, and enables `bTrailerSameRotation`.

Consequently the submerged-looking Flipendo target is not evidence that a
second authored object has a bad transform. It is a spawned effect positioned
from `rolllog1`'s native world collision box and then trailed to `rolllog1`.
The collision box is centered on the authored `Location`, so its midpoint is
also near Z 6.8568. A correct shared fix should leave this authored targeting
contract intact; adding an offset to a named Flipendo effect would be another
forbidden actor-specific workaround.

### 8. Retail load, idle walking, and first movement (`Engine.dll`)

The native trace here refers to the local installed
`res/System/Engine.dll`, SHA-256
`7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`.
It was obtained by disassembly only; no original executable was run.

#### Map loading preserves the authored location

`UGameEngine::LoadMap` (`0x1039c3d0..0x1039eb97`) obtains its loaded `ULevel`
from engine offset `+0x70`. At `0x1039d6c6..0x1039d6cd` it calls level virtual
slot `+0x5c`, whose `ULevel` vtable entry resolves to the
`SetActorCollision` thunk `0x10304d72 -> 0x103aea90`.

The enable branch creates the collision hash and registers actors through
`AddActor` (`0x103021bc -> 0x10364b50`). At
`0x10364d3a..0x10364d53`, `AddActor` copies `Actor.Location` (`+0xfc`) into
the actor's collision-hash bookkeeping (`+0x120`); it does not rewrite
`Location`. `AActor::PostLoad` (`0x10379910..0x103799ba`) and
`ULevel::PostLoad` (`0x103ae930..0x103aea3c`) likewise do not call movement or
write actor location.

Retail `FarMoveActor` (`0x10304bbf -> 0x103a9de0`) is a distinct path that can
call `FindSpot` and eventually write `Location`, but the inspected load/
registration chain does not call it. Thus retail map loading provides no
missing initial relocation: it preserves the authored Z 6.8568.

Reproduction commands:

```sh
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1039c3d0 \
  --stop-address=0x1039eb98 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103ae930 \
  --stop-address=0x103aeb10 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10364b50 \
  --stop-address=0x10364d70 res/System/Engine.dll
```

#### Superseded partial trace: idle walking was initially read as downward-only

> **Correction (section 27):** this subsection stopped too early at
> `0x103e9b27`. The following historical bound is wrong: the continuation at
> `0x103e9b27..0x103e9bc8` contains the proven upward 1.9/2.1/2.4 floor-
> clearance branch. It is retained to show exactly where the investigation
> went astray; section 27 supersedes its conclusion.

`APawn::physWalking` is at `0x103e6b60`. It builds the per-slice movement
delta in locals `[ebp-0x20, -0x1c, -0x18]`. At
`0x103e706c..0x103e70d0`, all three absolute components are compared with the
`0.0001` float at `0x1047381c`. If all are below that threshold,
`0x103e70d2` sets the stationary marker (`edi=1`), sets remaining time to
zero, and jumps to **floor maintenance** at `0x103e9293`, not to the function
epilogue. The earlier version of this note incorrectly called that destination
an epilogue and said idle walking made no `MoveActor` call.

The complete zero-velocity/zero-acceleration route is now bounded. The log is
an `APawn` descendant, so the class test at `0x103e9293..0x103e92aa` passes.
The packed Pawn flag test at `0x103e92b0` is byte `actor+0x58c`, mask `0x10`.
The shipped Pawn declaration makes this fifth bool `bIsWalking`; neither the
Pawn default nor the `baseChar -> rolllog -> rolllog1` override chain sets it,
so the log takes `0x103e9859`. Because the stationary marker is one, it then
takes `0x103e9861`, performs a zero-extent line check from the collision bottom
to 20 units farther down at `0x103e9861..0x103e9943`, and reaches the main
floor sweep at `0x103e9981..0x103e9a0e`.

That main sweep is downward by `MaxStepHeight + 2`: `0x103e6e37..0x103e6e9d`
forms the gravity-direction vector from `actor+0x2d0` plus the float 2.0 at
`0x104770ec`. For this log that is 27 units. Its extent comes from
`AActor::GetCylinderExtent` (`0x103028d3 -> 0x1037a900`). The explicit
`CollideType==2` branch at `0x1037a957..0x1037a9b2` returns
`(CollisionWidth, CollisionWidth, CollisionHeight)` when CollisionWidth is
nonzero, hence `(115,115,89.891426)` here. A floor hit with `Hit.Time<1`
reaches the downward `ULevel::MoveActor` call at `0x103e9a86..0x103e9ad5` and
may update Base at `0x103e9adb..0x103e9aed`. A miss instead follows
`0x103e9a26 -> 0x103e9b27`; the lose-floor tail eventually emits the event at
`0x103ea154..0x103ea16e` and changes walking to falling at
`0x103ea171..0x103ea17d`.

The conclusion originally written here was that a stationary walking tick
could request only the **downward** `MoveActor`. That conclusion is withdrawn:
the trace omitted the separate upward clearance call documented in section
27. The narrower observation that this path does not call `stepUp` remains
correct.
Neither path lifts the actor to make a buried render become visible.

That bound has now been followed through the complete retail movement and BSP
collision implementation. `ULevel::MoveActor` is the export thunk
`0x1030404d -> 0x103aa3a0`. For the idle floor call its input Delta is
`(0,0,-27)` and all four trailing booleans are zero. `MoveActor` initializes
`Hit.Time=1.0` at `0x103aa530..0x103aa552`, obtains BSP/actor sweep results
through virtual `ULevel::MultiLineCheck` at `0x103aa89d..0x103aa912`, then
uses only a scalar fraction of the original Delta:

```text
if Hit.Time < 1 and !NoFail:
    fraction = ((padding + distance) * Hit.Time - padding) / distance
    if fraction <= 0.0001:
        Hit.Time = 0
        return false                     # no Location write
    movement = original_Delta * fraction
else:
    movement = original_Delta
```

The exact scalar adjustment is `0x103aaad2..0x103aab6b`; the rejection is
`0x103aab0d..0x103aab38`. For the ordinary box mover the padding selected at
`0x103aa666..0x103aa696` is 2.0. Shipped `UModel::LineCheck`
(`0x103042d2 -> 0x10429c80`) takes the nonzero-extent swept-hull branch for
this CT_Box, initializes `Hit.Time=2.0` at `0x1042a048..0x1042a09c`, and,
when the hull reports a hit, explicitly clamps the skin-adjusted result to the
closed interval `[0,1]` at `0x1042a141..0x1042a17c`. (Its point branch has
the same clamp at `0x10429e47..0x10429e76`.) Thus even a reported initial
overlap is time zero after clamping; more generally every accepted BSP hit
supplies a fraction in `[0,1]`. `MoveActor` can therefore reject, leave the
mover unchanged, or apply a positive fraction no greater than one; none can
change the sign of the input Z. The sole principal mover-location commit is
`0x103aafd2..0x103ab005`:

```text
Actor.Location.X += movement.X
Actor.Location.Y += movement.Y
Actor.Location.Z += movement.Z
```

Hence the idle request can leave Z unchanged or lower it by at most 27 units.
It cannot depenetrate upward, independent of which exact BSP polygon supplies
the floor hit.

The prospective-position encroachment branch also contains no hidden upward
push of the mover, and the log does not enter it. The APawn inheritance check
at `0x103aaef1..0x103aaf15` sends a Pawn directly to the normal commit. For
non-Pawns, `MoveActor` calls `ULevel::CheckEncroachment` at
`0x103aaf1b..0x103aaf84`; a true/reject result returns false at
`0x103aaf88..0x103aafcf` before the location commit. The callee is
`0x103026fd -> 0x103ab5f0`. Its native push path calls `moveSmooth` on the
*overlapped other actor* at `0x103ab7d7..0x103ab81d`. A later event-check path
temporarily copies the proposed location into the mover at
`0x103ab99b..0x103ab9e5`, calls `moveSmooth` on the other actor at
`0x103aba02`, and unconditionally restores the mover's saved XYZ at
`0x103aba07..0x103aba15`. No native encroachment branch adds a separation
normal or a positive-Z correction to this mover. The `rolllog -> baseChar ->
Pawn -> Actor` compiled script chain has no encroachment override that adds a
separate relocation.

Finally, neither floor tracing nor base assignment can move the render through
`PrePivot`. `AActor::SetBase` is `0x10304cb9 -> 0x1037a6f0`; its complete body
only validates the base chain, updates `Actor.Base` at `actor+0x9c`, maintains
the base attachment count at `base+0xc4`, and emits base-change events
(`0x1037a71d..0x1037a7e8`). It never reads or writes `Actor.PrePivot`
(`actor+0x170..0x178`) or `Actor.Location`. The two floor queries return
`FCheckResult` data, and the relevant `physWalking`, `MoveActor`,
`MultiLineCheck`, `UModel::LineCheck`, and `SetBase` ranges contain no
`PrePivot` store. This rejects SetBase/floor-trace initialization as the
missing visual correction.

When `patrol` begins a horizontal `MoveTo`, `physWalking` calls `MoveActor`
through the `ULevel` virtual slot `+0x8c` at `0x103e8e40..0x103e8e86`.
`Hit.Time` is initialized to 1.0 at `0x103e6ef2`; a blocking result below 1.0
falls through the comparison at `0x103e8f2d..0x103e8f3e` to the `stepUp` call
at `0x103e9008`.

`APawn::stepUp` is `0x103028a6 -> 0x103ec690`. With the normal gravity
direction `(0,0,-1)`, its first movement is
`-(GravDir.Z * MaxStepHeight)`, computed at
`0x103ec6fb..0x103ec787`, and submitted to `MoveActor` at `0x103ec79f`.
`Pawn`'s compiled `MaxStepHeight` default is exactly 25.0 (`00 00 c8 41`) and
neither `baseChar`, `rolllog`, nor `rolllog1` overrides it. The first step-up
request is therefore +25 Z. Its final step-down uses the unnegated
`GravDir * MaxStepHeight` at `0x103ecbb0..0x103ecbf9` and may be limited by
collision.

Reproduction commands:

```sh
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103e6d70 \
  --stop-address=0x103e70f5 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103e9250 \
  --stop-address=0x103e9b40 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103ea0c0 \
  --stop-address=0x103ea190 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1037a900 \
  --stop-address=0x1037a9b3 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103e8e40 \
  --stop-address=0x103e9020 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103ec690 \
  --stop-address=0x103ec800 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103ecbb0 \
  --stop-address=0x103ecc10 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103aa3a0 \
  --stop-address=0x103ab1f8 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103ab5f0 \
  --stop-address=0x103aba8c res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10429c80 \
  --stop-address=0x1042a24f res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1037a6f0 \
  --stop-address=0x1037a7f9 res/System/Engine.dll
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u classprops 0 \
  | rg '^MaxStepHeight'
```

This `stepUp` path remains a separate real retail mechanism once Flipendo
starts horizontal path movement. It is no longer the candidate explanation
for startup placement: section 27 proves the earlier idle path itself performs
the progressive lift.

#### Complete skeletal-coordinate sign and cache behavior

The full retail composition corrects an earlier incomplete sign
interpretation:

- `USkeletalMesh::GetMeshCoords` at `0x1041afab..0x1041afec` computes the
  bottom-alignment adjustment `A` (about `-92.4435` here).
- At `0x1041b1cf..0x1041b1f5`, it adds `Actor.Location`, returning coordinate
  origin `L + A` for identity Z.
- Every `GetFrame` call at `0x1041dfa8..0x1041dfad` calls `ApplyAnim`.
  `ApplyAnim` unconditionally recomputes `GetMeshCoords` at
  `0x1041bc57..0x1041bc65` and copies the 48-byte coordinates into the cache
  before cache early-outs (`0x1041bc6f..0x1041bc7b`). Thus the old coordinates
  cannot remain cached across `SetPhysics`.
- Final vertex output at `0x1041e3de..0x1041e3f8` explicitly adds coordinate
  origin Z, yielding `v.z + L + A` for the identity case.

Therefore the effective raw sign is `+A`: the negative adjustment moves this
mesh downward. The inverse-coordinate subtraction elsewhere in `GetFrame`
does not reverse the final world result. Neither reversing `A` nor blaming a
stale cached pre-physics transform is supported by the shipped binary.

#### Reopened render-actor and virtual-dispatch trace

The retail observation required reopening one assumption in the
preceding trace: whether the `AActor*` received by `GetMeshCoords` is really
`rolllog1`, or a render copy/owner/proxy with different physics or collision
flags. The complete shipped `Render.dll -> Engine.dll` path rules out that
explanation.

The binaries used for this pass are:

```text
Render.dll 41c0e9939cac1833978c15bb10a13761b3559ad929f060ec88b6aae8b96bc55f
Engine.dll 7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc
```

Exact native path:

1. `URender::DrawActor` is exported at `0x10b01091` and jumps to
   `0x10b33980`. At `0x10b339d5` it reads the incoming actor argument and at
   `0x10b339df` stores that pointer at local `-0x40`, exactly
   `FDynamicSprite+0x98` for the sprite beginning at local `-0xd8`.
   `FDynamicSprite` construction at `0x10b30c70` reads this field but does not
   replace it.
2. `URender::DrawActorSprite` (`0x10b01294 -> 0x10b32850`) loads the actor
   verbatim from `sprite+0x98` at `0x10b328b5`. The normal `DT_Mesh` branch
   verifies `actor+0x154 == 2` at `0x10b32e4a`, verifies the mesh at
   `actor+0x164` at `0x10b32e53..0x10b32e5b`, reloads `sprite+0x98` at
   `0x10b32e91`, and pushes that pointer as `DrawMesh`'s actor argument at
   `0x10b32e9e` before the call at `0x10b32ea1`.
3. `URender::DrawMesh` (`0x10b011cc -> 0x10b0e920`) loads that actor argument
   at `0x10b0e953` and its mesh at `0x10b0e95a`. Its class-chain test at
   `0x10b0e963..0x10b0e980` recognizes this `USkeletalMesh` as a `ULodMesh`
   and dispatches `URender::DrawLodMesh` at `0x10b0e991`; the later generic
   slot-`+0x7c` call is not the actual path for this asset.
4. `DrawLodMesh` (`0x10b01343 -> 0x10b0ff00`) retains the actor argument at
   `[ebp+0x10]`. After choosing a LOD vertex count, it pushes that unchanged
   actor at `0x10b1025e`, copies the selected rendering coordinates by value at
   `0x10b1025f..0x10b10269`, and calls mesh vtable slot `+0xa4` at
   `0x10b10270`. In the shipped `USkeletalMesh` vtable (VA `0x1047b43c`),
   slot `+0xa4` is `0x10301983`, the `GetFrame(..., AActor*, int&)` thunk to
   the already-traced body `0x1041df50`. The unused slot-`+0x7c` wrapper would
   reach the same body via `+0xa4`, but it is not needed to establish the
   rolling log's real path.

There is one explicit actor substitution inside the final body:

```text
0x1041df78  read byte actor+0x28
0x1041df7e  test mask 0x80
0x1041df88  read actor+0x38 when set
0x1041df91  otherwise retain original actor
0x1041dfaa  push selected actor
0x1041dfad  call USkeletalMesh::ApplyAnim
```

Reduced pseudocode:

```text
selected = ((actor_flags_at_28 & 0x80) != 0 && actor_at_38 != null)
         ? actor_at_38
         : actor;
ApplyAnim(selected, null, false);
```

Offset `+0x38` is hard-proven as `Owner`: `AActor::SetOwner`
(`0x10301faa -> 0x1037a5e0`) reads the old value at `0x1037a60a` and writes
the new value at `0x1037a637`. The `+0x28` mask is semantically identified as
`bAnimByOwner`: `Engine.u` declares that property, this path uses the bit only
to select a non-null owner as the animation actor, and the same sequence occurs
in `ULodMesh::GetFrame` at `0x1035e499..0x1035e4ac`. The stripped DLL has no
individual bool symbol, so the property name is a semantic identification,
not a PDB-backed field label.

For this actor the substitution is inactive. Raw tagged properties for
`Engine.Actor` export 1, `Engine.Pawn` export 0, `HPBase.baseChar` export 4,
`HarryPotter.rolllog` export 839, and map export 2132 contain no
`bAnimByOwner=true` or `Owner` override. The zero/default value therefore
survives the exact inheritance chain. The shipped `rolllog` source contains no
runtime `SetOwner` or `bAnimByOwner` assignment. Consequently the actor whose
`Physics`, `Location`, `bAlignBottom`, and `bCollideWorld` reach
`GetMeshCoords` on this normal render path is `rolllog1` itself.

This also closes an earlier virtual-dispatch gap: Render uses the five-argument
body traced in the preceding subsection directly through `DrawLodMesh`'s
slot-`+0xa4` call. There is no alternate skeletal `GetFrame` branch here that
bypasses `ApplyAnim` or `GetMeshCoords`.

Reproduction commands:

```sh
shasum -a 256 res/System/{Render.dll,Engine.dll}
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10b32850 \
  --stop-address=0x10b33520 res/System/Render.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10b0e920 \
  --stop-address=0x10b0eb20 res/System/Render.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10b0ff00 \
  --stop-address=0x10b10280 res/System/Render.dll
xxd -g4 -l 192 -s 0x17b43c res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10404470 \
  --stop-address=0x104044b1 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041df50 \
  --stop-address=0x1041dfb2 res/System/Engine.dll
/tmp/rollprobe/target/debug/rollprobe res/Maps/Lev2_Fire2.unr props 2132 \
  | rg 'Owner|bAnimByOwner'
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u classprops 1 \
  | rg 'Owner|bAnimByOwner'
```

Both property commands intentionally produce no matching line.

#### Stationary skeletal frames are not render-cached across physics changes

The proposed stale-frame explanation has now been tested at the renderer call
site, not only inside `USkeletalMesh`. It is contradicted by the shipped
binaries. A visible stationary skeletal actor is sampled on every world draw;
neither an unchanged `AnimSequence=Stop` nor unchanged animation phase can
suppress `GetFrame`.

`Render.dll::URender::DrawWorld` (`0x10b01069 -> 0x10b27ba0`) calls
`OccludeFrame` at `0x10b27c7b` and then `DrawFrame` at `0x10b27c83` on every
invocation. `DrawFrame` (`0x10b01032 -> 0x10b254f0`) rebuilds its arrays from
the current frame's dynamic-sprite lists. Its visible-sprite loops call
`DrawActorSprite` at `0x10b25859`, `0x10b266ef`, and `0x10b26734`; none of
those loops tests an actor animation/physics dirty flag.

For the log's generic `DT_Mesh` branch, `DrawActorSprite` reaches `DrawMesh`
at `0x10b32ea1`. `DrawMesh`'s class-chain test routes this skeletal `ULodMesh`
to `DrawLodMesh` at `0x10b0e991`. After LOD selection, `DrawLodMesh` reaches
this unconditional call:

```text
0x10b1024f..0x10b10252  select rendering FCoords
0x10b10252                 load original actor argument
0x10b1025e                 push actor
0x10b1025f..0x10b10269     copy FCoords by value
0x10b1026b                 push vertex stride
0x10b1026d                 push requested vertex count
0x10b10270                 call mesh vtable +0xa4 ; GetFrame with count result
```

There is no sequence, phase, physics, transform, or dirty comparison between
entry to `DrawMesh` at `0x10b0e920` and this call. Therefore each visible
world draw invokes `USkeletalMesh::GetFrame`, even when the actor and its
`Stop` animation are stationary.

The internal pose cache also cannot retain an old mesh coordinate system.
Every `GetFrame` calls `ApplyAnim` at `0x1041dfad`. Every `ApplyAnim` calls
`GetMeshCoords` at `0x1041bc65` and copies all 48 bytes of the returned
`FCoords` to cache-header offset `+0x20` at `0x1041bc6f..0x1041bc79` *before*
the unchanged-sequence/frame cache tests at `0x1041bc7d..0x1041bca4` can
jump to `0x1041d571`. Back in `GetFrame`, it reacquires that header and copies
the just-written coordinates from header `+0x20` at
`0x1041dfff..0x1041e03b`, composes them with the caller coordinates, and uses
the result for vertex output. The cache can reuse skeletal pose work, but not
the previous call's physics-dependent `GetMeshCoords` result.

Load ordering rejects a one-time pre-state sample as the source of an initial
frame that survives. `UGameEngine::LoadMap` assigns the newly loaded level to
`engine+0x70` at `0x1039cde5`. Later in the same non-returned call it walks
that level's actor array four times and dispatches, in order:

```text
0x1039dec1  ENGINE_PreBeginPlay    (FName at 0x105e9ddc)
0x1039df00  ENGINE_BeginPlay       (FName at 0x105e9de0)
0x1039df72  ENGINE_PostBeginPlay   (FName at 0x105e9dd8)
0x1039dfb1  ENGINE_SetInitialState (FName at 0x105e9db4)
```

`LoadMap` does invoke `UGameEngine::Draw` at `0x1039c7f7` and
`0x1039c8ac`, but both calls precede disposal of the old current level at
`0x1039cba3` and assignment of the new level at `0x1039cde5`; they are load
progress draws of the prior viewport/world, not samples of `rolllog1`. From
the new-level assignment through the lifecycle loops there is no call through
the engine's render interface (`engine+0x50`) and no `GetFrame`, `ApplyAnim`,
or `GetMeshCoords` call. Thus the first possible visible sample of the new log
occurs only after `SetInitialState` has been dispatched, and even if an earlier
sample existed it could not survive the unconditional per-draw coordinate
refresh proved above.

This rules out the specific hypothesis that retail cached the raw authored
mesh while serialized `Physics=None`, retained it because `LoopAnim('Stop')`
did not change the sequence, and refreshed only when Flipendo selected
`Roll`. The missing retail condition remains elsewhere; this hypothesis does
not justify an implementation change.

Reproduction commands:

```sh
objdump -d --start-address=0x10b27ba0 --stop-address=0x10b27e80 \
  res/System/Render.dll
objdump -d --start-address=0x10b254f0 --stop-address=0x10b26760 \
  res/System/Render.dll
objdump -d --start-address=0x10b0e920 --stop-address=0x10b0eb20 \
  res/System/Render.dll
objdump -d --start-address=0x10b0ff00 --stop-address=0x10b10280 \
  res/System/Render.dll
objdump -d --start-address=0x1041ba60 --stop-address=0x1041bd00 \
  res/System/Engine.dll
objdump -d --start-address=0x1041d520 --stop-address=0x1041d768 \
  res/System/Engine.dll
objdump -d --start-address=0x1041df50 --stop-address=0x1041e150 \
  res/System/Engine.dll
objdump -d --start-address=0x1039c640 --stop-address=0x1039c8d0 \
  res/System/Engine.dll
objdump -d --start-address=0x1039c3d0 --stop-address=0x1039eb98 \
  res/System/Engine.dll | rg -n -B12 -A18 \
  '105e9ddc|105e9de0|105e9dd8|105e9db4'
objdump -p res/System/Engine.dll | rg \
  'ENGINE_(PreBeginPlay|BeginPlay|PostBeginPlay|SetInitialState)'
```

#### Auto-state entry is not after `SetPhysics`

Another possible bypass was that retail might enter `waitforspell` at a label
after its leading `SetPhysics(PHYS_Walking)`. The decoded state disproves it.
`HarryPotter.u` export 1366 has `label_table_offset=0x20`; its label table
contains `Begin` (name index 16) with code offset **0**, followed by `Loop`
(name index 25) with code offset `0x0d`. Thus normal `GotoState('Auto')` enters
the leading native 3970 call at bytecode offset zero. There is no compiled
initial-state entry point that skips the physics change.

```sh
/tmp/rollprobe/target/debug/rollprobe res/System/HarryPotter.u scriptbytes 1366
/tmp/rollprobe/target/debug/rollprobe res/System/HarryPotter.u name 16 25
```

The relevant decoded bytes are:

```text
code 0x00..0x1f: SetPhysics(Walking), LoopAnim(Stop), Sleep/Goto loop
label table at 0x20:
  name 25 (Loop),  code offset 0x0000000d
  name 16 (Begin), code offset 0x00000000
```

#### State-frame, tick, and stasis scheduling do not keep the log at `PHYS_None`

The remaining scheduling hypothesis was that `SetInitialState` selected the
auto state but retail deferred its `Begin` code indefinitely because the log
was in stasis or was not in the level's tick range. The shipped Core/Engine
binaries and package defaults reject that as an explanation for a stable
pre-spell scene. They permit, at most, a very short pre-tick sample before the
first effective actor tick.

The serialized ObjectStack for Lev2_Fire2 export 2132 is also consistent with
a placed actor awaiting startup rather than an already-running or latent
state: at map serial range `0x171746..0x1717e6`, Node and StateNode resolve to
`rolllog`, ProbeMask is all ones, LatentAction is zero, and the serialized
instruction pointer is `-1`/null. There is no pre-existing state instruction
pointer that could jump past `waitforspell.Begin`.

`UGameEngine::LoadMap` synchronously dispatches the four actor lifecycle
events in order: `PreBeginPlay` at `0x1039dea0..0x1039deda`, `BeginPlay` at
`0x1039dedf..0x1039df19`, `PostBeginPlay` at
`0x1039df51..0x1039df8b`, and `SetInitialState` at
`0x1039df90..0x1039dfca`. Each loop's per-actor skip is the same
`test [actor+0x14c],0x10000` (at `0x1039deb5`, `0x1039def4`,
`0x1039df66`, and `0x1039dfa5`). This is
`bScriptInitialized`, not a physics/stasis test. The identity follows directly
from the shipped `Engine.u` declaration order: the DWORD begins with the nine
editor booleans `bHiddenEd` through `bTempEditor`, continues with seven filter
booleans `bDifficulty0` through `bNetSpecial`, and declares
`bScriptInitialized` next, at bit 16 (`0x10000`). The comment says that this
flag prevents reinitializing actors spawned during startup. Neither
`Actor`, `Pawn`, `baseChar`, `rolllog`, nor map export 2132 serializes this flag
true, and the map's state-frame sentinel is uninitialized. `AActor`'s native
constructor (`0x1031b920..0x1031b93d`) does not write `+0x14c`. Thus the placed
log is not skipped by this lifecycle loop.

The selection of the auto state is synchronous, while execution of its state
body is tick-driven. `AActor::eventSetInitialState`
(`0x1031a780..0x1031a7a0`) synchronously calls `ProcessEvent`. The shipped
event first sets `bScriptInitialized=true` and executes `GotoState('Auto')`.
Core's `UObject::GotoState` export (`0x101030fd`) jumps to
`0x10131bb0`. It clears the old latent action at `0x10131bf4..0x10131bf7`;
recognizes the `Auto` name at `0x10131c14..0x10131c27`; and walks the class
state chain, selecting a state whose flags byte at `state+0x80` carries mask
`0x02`, at `0x10131c3e..0x10131c70`. The decoded `waitforspell` export has
exact flags `0x02`, so this is the selected auto state. `GotoState` then writes
that state to `frame+0x04` and `frame+0x1c` and deliberately clears the code
pointer at `frame+0x0c` (`0x10131cf4..0x10131d12`). It dispatches the state
change events but does not itself execute the state's ordinary body.
`UObject::execGotoState` (`0x101022ac -> 0x10141c50`) then supplies the default
label name when no second argument is present (`0x10141ced..0x10141cf8`) and
calls virtual `GotoLabel` at `0x10141cfd`. The target is
`UObject::GotoLabel` (`0x101029c8 -> 0x10131df0`): it reads the current state's
label-table offset from `state+0x84`, searches eight-byte name/offset entries
at `state.script+offset` (`0x10131e08..0x10131e35`), and on a match writes
`frame.code = state.script + label.code_offset` at
`0x10131e4c..0x10131e5d`. The default name is therefore not merely recorded
as metadata; it resolves the executable instruction pointer synchronously.
Combined with the byte-proven `waitforspell.Begin = code offset 0`, the state
frame points at the leading `SetPhysics(PHYS_Walking)` before
`SetInitialState` returns. Only execution of that instruction awaits the first
effective `ProcessState` call.

The engine-level ordering leaves room for one initial raw-placement frame, but
not a persistent state. `UGameEngine::Tick` (`0x10303a12 -> 0x103a0900`)
ticks the level that was current on entry through virtual slot `+0x60` with
literal `TickType=2` at `0x103a0b00..0x103a0b0f`. Pending travel/Browse is
handled later at `0x103a11a9..0x103a11ea`; Browse reaches `LoadMap` through
virtual slot `+0xb4` at `0x1039af91..0x1039afa0` (and the parallel path
`0x1039b3b7..0x1039b3cd`). That engine tick then returns without ticking the
newly loaded level. Therefore the first draw after travel can observe the new
level after `SetInitialState`/`GotoState` but before its first `ULevel::Tick`,
with the state body still pending and Physics still serialized none. The next
engine tick begins ticking the new level; this is a bounded startup ordering,
not a condition that lasts until Flipendo.

The log is in the normal dynamic-actor tick range. During `LoadMap`, retail
groups the actor array by the first Actor flags DWORD at `+0x28`:

```text
0x1039e242..0x1039e24e  select bStatic && !bAlwaysRelevant
0x1039e2ec..0x1039e2f8  select bStatic &&  bAlwaysRelevant
0x1039e33a              store the end of both static groups at level+0x110
0x1039e360..0x1039e39d  append every !bStatic actor
```

The mask identities are fixed by the shipped Actor boolean declaration order:
`bStatic` is bit 0 and `bAlwaysRelevant` is bit 20 (`0x100000`). The native
constructors cannot silently change the assumed lifecycle fields:

- `AActor::InternalConstructor` (`0x1031b900`) enters `AActor::AActor`
  (`0x1031b920..0x1031b93d`). That constructor calls the base constructor,
  zeroes only `+0x6c/+0x70/+0x74`, and installs the AActor vtable. It does not
  write flags `+0x28`, Physics `+0x30`, Role `+0x31`, or the packed
  lifecycle word at `+0x14c`.
- `APawn::InternalConstructor` (`0x10320200`) enters `APawn::APawn`
  (`0x10320220..0x10320280`). It performs the same three zero writes,
  constructs the Pawn member at `+0x398`, and installs the Pawn vtable. It
  likewise does not write any of `+0x28`, `+0x30`, `+0x31`, or `+0x14c`.

The compiled/tagged default chain
`Engine.Actor -> Engine.Pawn -> HPBase.baseChar -> HarryPotter.rolllog ->`
map export 2132 consequently remains authoritative. It gives `Role=4`
(`ROLE_Authority`) from `Actor`, serialized Physics none, and no true override
for `bStatic`, `bStasis`, `bForceStasis`, `bTicked`, or
`bScriptInitialized`. No `TickGroup` property exists in this shipped Actor
generation, and the native `ULevel::Tick` actor loop contains no tick-group
comparison. Thus the log belongs after `level+0x110`, with no hidden native or
serialized scheduling category that excludes it.
On the normal full-level branch, `ULevel::Tick` starts exactly at this index
(`0x103b7177`), walks to the actor count, and calls each non-null actor's
virtual slot `+0x74` at `0x103b7199`. `APawn` does not export a Tick override,
so this is the exported `AActor::Tick` implementation
(`0x10304205 -> 0x103b3840`).

The tick de-duplication parity cannot defer the state indefinitely either.
At `0x103b387c..0x103b3893`, `AActor::Tick` compares actor flag `bTicked`
(bit 11) with `level+0x10c` and returns if they already match. At the end of
every completed `ULevel::Tick`, retail flips `level+0x10c` at
`0x103b74cc..0x103b74db`. Therefore even if the loaded log and the level
initially have equal parity, only one completed level tick is skipped; the
next full tick necessarily differs and enters the actor tick.

One additional outer tick skip is explicit and inactive in ordinary play.
At `0x103b3899..0x103b38b4`, retail reads the level's special-pause state at
`level+0x111c`; when nonzero it returns unless `actor+0x2c` carries mask
`0x02`, then sets mask `0x04`, while the inactive path clears `0x04`. The
shipped Actor declaration order identifies these as
`bCanMoveInSpecialPause` and `bInSpecialPause`, not `bAlwaysTick` or the
standard `bPlayersOnly` flag. The normal level is not in this special pause.
The later network/simulation branch also returns at
`0x103b41e5..0x103b41ef` for `TickType==1`; `UGameEngine::Tick` passes the
normal full-level value 2 at `0x103a0b00..0x103a0b0f`. Thus neither outer
condition suppresses this ordinary authority actor before the virtual
`ProcessState` call.

Stasis and zone visibility also cannot suppress this log. `AActor::Tick`
tests `actor+0x28` mask `0x00800000` at `0x103b38b7..0x103b38bf` and jumps
straight past all stasis logic when clear. The shipped Actor declaration makes
this bit 23, `bStasis`; the next mask `0x01000000` at `0x103b38c1` is bit 24,
`bForceStasis`. Only when `bStasis` is true and Physics is none/rotating (or
`bForceStasis` is true) does retail consult the zone-render recency path at
`0x103b38d3..0x103b3936`. That path reads
`UModel[24*ZoneNumber + 0x108]`; `Render.dll::URender::OccludeBsp`
(`0x10b01140 -> 0x10b22480`) writes current Level time to this exact visible-
zone slot at `0x10b227df..0x10b22802`, proving it is zone
`LastRenderTime`. The tick path subtracts it from current Level time and
compares the age with 5.0 (raw `0x40a00000` at `0x10476584`). It suppresses
the actor only if the age exceeds five seconds and `LevelInfo+0x464` is zero.
Cross-references comparing `+0x464` with values 1, 2, and 3 in network/demo
paths identify it as the `ENetMode` byte, so zero is `NM_StandAlone`, exactly
matching the shipped Actor comment. All four class-default exports and the map
actor contain no true `bStasis` or `bForceStasis` tag. Consequently
`LastRenderTime`, the actor's zone number, and recent zone visibility are not
read for this log before its tick continues at `0x103b393c`.

On the normal authority path, `AActor::Tick` calls its virtual `ProcessState`
slot at `0x103b4248..0x103b424d` before native physics dispatch. The exact
actor override is `0x1040ef10`. It returns before executing bytecode only when
one of these gates fails:

- `actor+0x0c` (the state frame) is null, or its code pointer at `frame+0x0c`
  is null (`0x1040ef35..0x1040ef46`);
- `Role` at `actor+0x31` is below 4 and the current state/node does not carry
  the simulated flag at `[frame+0x1c]+0x80`, mask `0x04`
  (`0x1040ef4c..0x1040ef5c`);
- virtual slot `+0x2c` reports pending kill (`0x1040ef62..0x1040ef6b`). The
  AActor vtable at `0x1046e1dc` maps this slot to thunk `0x10302347`, body
  `0x1031c380`, which returns actor flag bit 9 (`bDeleteMe`).

The log inherits `Role=4` (`ROLE_Authority`) from `Engine.Actor`, so the
simulated-state test is bypassed at `0x1040ef50`; it has the non-null compiled
`waitforspell` frame/code described above and is not pending kill. Once inside,
`0x1040efdf..0x1040eff6` advances the state code pointer and dispatches
bytecode tokens. Execution stops after a token if `bDeleteMe` becomes true
(`0x1040efba..0x1040efc0`), the state code pointer becomes null, or the latent
action at `frame+0x28` becomes nonzero (`0x1040efc6..0x1040efd9`); it also
bounds repeated state-node changes to four at `0x1040eff9..0x1040f00c`.
The compiled leading `SetPhysics(Walking)` and `LoopAnim(Stop)` are both before
the state's sleep, so `SetPhysics` executes in this first effective
`ProcessState` call. None of the ordinary dynamic authority log's skip
conditions can defer that call until Flipendo.

There is no native post-state restoration to `PHYS_None` on the relevant
idle-walking path. `AActor::setPhysics` (`0x103e5140`) is the writer at
`actor+0x30`. After `ProcessState` returns, `AActor::Tick` reads the possibly
new Physics byte at `0x103b4324`; nonzero authority physics calls virtual
`performPhysics` at `0x103b4331..0x103b4336`. Thus the leading state-code
`SetPhysics(PHYS_Walking)` can take effect and be processed in the very same
tick; initial none does not skip state execution. `APawn::performPhysics`
(`0x103e5520`) dispatches mode 1 to
`APawn::physWalking` at `0x103e5571..0x103e5579`. Across the complete walking
body (`0x103e6b60..0x103ea54c`) there is exactly one direct native call to
`setPhysics`: if walking loses its floor, `0x103ea171..0x103ea17d` changes
mode 1 to mode 2 (`PHYS_Falling`), still nonzero. It never selects mode zero.
The shipped `waitforspell` loop likewise has no later `SetPhysics(None)`.

Hard conclusion: a first render before the first effective actor tick could
observe serialized `PHYS_None` and therefore bypass MeshAdjust. After travel,
however, the log executes `waitforspell.Begin` on its first eligible actor
tick; arbitrary initial `bTicked` parity can skip at most one completed
new-level tick, so execution occurs by the second completed new-level tick at
latest. Neither stasis, zone visibility, Role, a tick group, lifecycle
skipping, native construction, nor native walking physics can keep or restore
`PHYS_None` for a stable active pre-spell scene. There is therefore no retail
condition in this traced lifecycle path that prevents `SetPhysics(Walking)`.
This scheduling path does **not** supply the missing condition and does not
justify an OpenHP1 implementation change.

Reproduction commands:

```sh
shasum -a 256 res/System/{Engine.dll,Core.dll}
xxd -g1 -s 0x171746 -l 0xa0 res/Maps/Lev2_Fire2.unr
strings -a res/System/Engine.u | rg -n -A25 -B5 \
  'bTempEditor|bScriptInitialized|bStasis'
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u classprops 1 \
  | rg 'bStatic|bScriptInitialized|bTicked|bStasis|bForceStasis|Physics|Role'
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u classprops 0 \
  | rg 'bStatic|bScriptInitialized|bTicked|bStasis|bForceStasis|Physics|Role'
/tmp/rollprobe/target/debug/rollprobe res/System/HPBase.u classprops 4 \
  | rg 'bStatic|bScriptInitialized|bTicked|bStasis|bForceStasis|Physics|Role'
/tmp/rollprobe/target/debug/rollprobe res/System/HarryPotter.u classprops 839 \
  | rg 'bStatic|bScriptInitialized|bTicked|bStasis|bForceStasis|Physics|Role'
/tmp/rollprobe/target/debug/rollprobe res/Maps/Lev2_Fire2.unr props 2132 \
  | rg 'bStatic|bScriptInitialized|bTicked|bStasis|bForceStasis|Physics|Role'
objdump -d --start-address=0x1039de90 --stop-address=0x1039dfcd \
  res/System/Engine.dll
objdump -d --start-address=0x103a0900 --stop-address=0x103a1240 \
  res/System/Engine.dll
objdump -d --start-address=0x1039af70 --stop-address=0x1039afb0 \
  res/System/Engine.dll
objdump -d --start-address=0x10131bb0 --stop-address=0x10131df0 \
  res/System/Core.dll
objdump -d --start-address=0x10131df0 --stop-address=0x10131e70 \
  res/System/Core.dll
objdump -d --start-address=0x10141c50 --stop-address=0x10141d75 \
  res/System/Core.dll
objdump -d --start-address=0x1031b900 --stop-address=0x1031b950 \
  res/System/Engine.dll
objdump -d --start-address=0x10320200 --stop-address=0x10320290 \
  res/System/Engine.dll
objdump -d --start-address=0x1039e224 --stop-address=0x1039e3a6 \
  res/System/Engine.dll
objdump -d --start-address=0x103b3840 --stop-address=0x103b39a0 \
  res/System/Engine.dll
objdump -d --start-address=0x10b22480 --stop-address=0x10b22820 \
  res/System/Render.dll
objdump -d --start-address=0x103b7170 --stop-address=0x103b71a4 \
  res/System/Engine.dll
objdump -d --start-address=0x103b74c0 --stop-address=0x103b74e4 \
  res/System/Engine.dll
objdump -d --start-address=0x103b41ae --stop-address=0x103b4357 \
  res/System/Engine.dll
objdump -d --start-address=0x1040ef10 --stop-address=0x1040f0a5 \
  res/System/Engine.dll
objdump -d --start-address=0x103e5520 --stop-address=0x103e55f0 \
  res/System/Engine.dll
objdump -d --start-address=0x103e6b60 --stop-address=0x103ea54c \
  res/System/Engine.dll | rg -B8 -A4 'calll.*0x10304426'
```

The five package-property filters intentionally print no matching true flag
override except `Engine.Actor`'s unrelated `Role` line.

#### Gate-bit identity and startup-write audit

The retail recording makes the `MeshAdjust` gate the
critical boundary: a visible log at authored Z is consistent with the
adjustment returning zero, while OpenHP1's buried result is the exact result
of taking the adjustment branch. The two flag tests in that branch are now
identified and their startup writers audited directly, rather than inferred
only from decoded defaults.

`USkeletalMesh::MeshAdjust` (`0x1041ae00`) and the inlined copy inside
`GetMeshCoords` (`0x1041af91`) read the actor DWORD at `+0x1dc` and require
both masks:

```text
0x1041ae07  mov  edx,[actor+0x1dc]
0x1041ae0d  test dl,0x20
0x1041ae12  test dl,0x02
0x1041ae17  mov  dl,[actor+0x30]       ; Physics
0x1041ae1e  cmp  byte [actor+0x1d8],3 ; CollideType != CT_Shape
```

The shipped `Engine.u` source buffer declares this exact native collision
bitfield in order:

```uc
var(Collision) bool bCollideActors;
var(Collision) bool bCollideWorld;
var(Collision) bool bBlockActors;
var(Collision) bool bBlockPlayers;
var(Collision) bool bProjTarget;
var(Collision) bool bAlignBottom;
```

The native writer independently fixes the bit positions. `AActor::SetCollision`
(`0x10379b60..0x10379c01`) preserves bit `0x02`, clears/rebuilds bits
`0x01`, `0x04`, and `0x08` from its three public arguments, and writes the
word at `0x10379bd3`. The path-builder scout setup then calls
`SetCollision(true,true,true)` and separately ORs `0x02` at
`0x103c7f83..0x103c7f8f`; the extra flag is therefore the only collision
boolean not owned by `SetCollision`, `bCollideWorld`. With the shipped
six-boolean declaration and the consumers above, `0x10` is `bProjTarget` and
`0x20` is `bAlignBottom`. Thus the formula's masks really are
`bCollideWorld` and `bAlignBottom`; they are not guessed neighboring fields.

The native startup paths do not clear or overwrite either mask:

- `AActor::PostLoad` (`0x10379910..0x103799ba`) calls its superclass,
  marks referenced primitives, and conditionally validates `CollideType==3`;
  it never writes `actor+0x1dc` or `actor+0x30`.
- `ULevel::PostLoad` (`0x103ae930..0x103aea3c`),
  `ULevel::SetActorCollision` (`0x103aea90`), and the inspected
  `UGameEngine::LoadMap` actor-registration span contain no write to
  `actor+0x1dc`. The only `+0x1dc` access in the `LoadMap` body is a test at
  `0x1039e03c`.
- `AActor::InitExecution` (`0x10406b90..0x10406c99`) only validates the
  state-frame/object/class links. It does not touch actor flags or Physics.
- The native `PreBeginPlay`, `BeginPlay`, `PostBeginPlay`, and
  `SetInitialState` wrappers (`0x1031aaa0`, `0x1031ab30`, `0x1031a7c0`,
  `0x1031a780`) only resolve their `UFunction` and dispatch through
  `ProcessEvent`; they contain no direct actor-field write. The compiled
  `Actor`, `Pawn`, and `baseChar` implementations called for this actor do
  not assign `bAlignBottom`, `bCollideWorld`, or call `SetCollision`.
- `AActor::setPhysics` (`0x103e5140..0x103e523b`) writes only Physics at
  `actor+0x30` (`0x103e517a`), manages Base through `SetBase`/`FindBase`, and
  clears velocity/acceleration fields for none/rotating physics. `FindBase`
  (`0x103e4fd0..0x103e50be`) does not touch `+0x1dc`.

Reflection can of course populate the word while constructing/loading the
object, but the raw shipped default chain supplies `bAlignBottom=true` from
`Actor` and `bCollideWorld=true` from `Pawn`, and no descendant or map tag
serializes a false override. No later native initialization write found here
turns either one off.

The neighboring mutable-looking input, `CollideType` at `actor+0x1d8`, has
also been audited by an exhaustive direct constant-offset memory-operand scan
of the shipped `Engine.dll`. There are only four byte-sized stores in actor or
Pawn code, and all four copy an existing actor value into a newly copied or
assigned object:

| destination store | enclosing native body | value source |
| --- | --- | --- |
| `0x1031d15b` | `AActor` copy constructor, body `0x1031c700` | source actor `+0x1d8` loaded at `0x1031d14f` |
| `0x1031e2eb` | `AActor::operator=`, body `0x1031d8d0` | source actor `+0x1d8` loaded at `0x1031e2df` |
| `0x10320cb4` | `APawn` copy constructor, body `0x10320320` | source Pawn `+0x1d8` loaded at `0x10320cae` |
| `0x10322ab4` | `APawn::operator=`, body `0x10322100` | source Pawn `+0x1d8` loaded at `0x10322aa8` |

The other apparent `+0x1d8` stores are demonstrably not actor-field writers.
`0x1036976c` and `0x1036a14c` are DWORD copies inside the exported
`UNetConnection` copy/assignment bodies (`0x10369290` and `0x10369db0`).
`0x1041a6be` is one element of the `USkeletalMesh` constructor's contiguous
field clearing at `0x1041a5d0..0x1041a6dc`. `0x10433b85` belongs to
`UChannel::SendBunch` (`0x10433960`), `0x1043d1ae` to
`UNetConnection::SendRawBunch` (`0x1043d130`), and `0x1043dc5d` to
`UNetConnection::Tick` (`0x1043d710`); they operate on channel/connection
structures. Meanwhile,
the stores around `0x104555c3` clear a regular three-bank table at offsets
`x`, `x+0x100`, and `x+0x200`; none receives an AActor pointer. Finally,
`0x10393b80..0x10393b90` is a four-instruction metadata callback registered at
`0x10394050..0x103940b7` for `ServerCommandlet`: that registration passes
property offset `0x1dc`, proving the callback's own `ecx+0x1d8` is class or
property metadata, not an actor's `CollideType`.

Computed-offset reflection/deserialization is outside what a literal operand
scan can enumerate, but its effect is already fixed by the serialized input:
the map instance explicitly loads `CollideType=2`. `AActor::PostLoad` only
**reads** `+0x1d8` at `0x1037997b`, and the startup/tick/SetPhysics/FindBase
paths above contain no store. The shipped class-chain source contains no
`CollideType = ...` assignment for this log, and the field is declared const
in `Engine.u`; the compiled `waitforspell` and `patrol` states likewise have no
property write. At package-reference level, `HPBase.u` is the only gameplay
package importing `CollideType`; its only compiled reference is the
`setTarget` comparison matching shipped source
`if (possiblevictim.CollideType == CT_Box)`. `HarryPotter.u` does not import
the property. There is therefore no original native or script path found
that changes this placed actor from `CT_Box` (2) to `CT_Shape` (3) before its
first draw. The `CollideType != 3` MeshAdjust gate remains true.

This closes the proposed "retail clears an alignment/collision flag during
startup" explanation. At the first draw, the two authored flag inputs and
`CollideType=2` should still pass. If the retail observation is explained by
the formula gate being off, the remaining mutable gate is the precise live
timing/value of `Physics` at `actor+0x30`, not a native
`bAlignBottom`/`bCollideWorld` overwrite. The subsequent state/tick audit above
now bounds `PHYS_None` to the interval before the first effective new-level
actor tick; it cannot explain stable pre-spell visibility. This must not be
replaced with an assumed flag or timing workaround.

Reproduction commands:

```sh
objdump -d --start-address=0x1041ae00 --stop-address=0x1041aeb4 \
  res/System/Engine.dll
objdump -d --start-address=0x10379b60 --stop-address=0x10379c02 \
  res/System/Engine.dll
objdump -d --start-address=0x103c7f75 --stop-address=0x103c7f96 \
  res/System/Engine.dll
objdump -d --start-address=0x10379910 --stop-address=0x103799bb \
  res/System/Engine.dll
objdump -d --start-address=0x10406b90 --stop-address=0x10406c9a \
  res/System/Engine.dll
objdump -d --start-address=0x103e4fd0 --stop-address=0x103e523c \
  res/System/Engine.dll
strings -a res/System/Engine.u | rg -n -A10 -B2 'bCollideActors'
objdump -d res/System/Engine.dll | rg '0x1dc\\('
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel res/System/Engine.dll \
  | rg '^10[0-9a-f]+:.*(mov|inc|dec|and|or|xor|add|sub).*\\[[^]]+ \\+ 0x1d8\\]'
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10393b70 \
  --stop-address=0x103940c0 res/System/Engine.dll
strings -a res/System/Engine.u | rg -n -A14 -B2 'enum ECollideType'
/tmp/rollprobe/target/debug/rollprobe res/System/HPBase.u scriptrefs -448
strings -a res/System/HPBase.u | rg -n -A8 -B8 'CollideType'
```

### 9. Mounted original-edition comparison

The mounted legally obtained edition at `/Volumes/HARRY_POTTER_EFG` was
decoded read-only to rule out a different effective class default. It is a
genuinely different build: its map, script packages, model package, and
`Engine.dll` all have different whole-file SHA-256 hashes from local `res/`.
For example:

```text
Lev2_Fire2.unr  local 8c3b03e1...  mounted 8e92344e...
HarryPotter.u   local 5f18066a...  mounted 8780db87...
HPBase.u        local 0cec62e0...  mounted bf99ff12...
Engine.u        local b3661a1d...  mounted dd7f0890...
HPModels.u      local 45ecb483...  mounted 5645656d...
Engine.dll      local 7756a2a3...  mounted 9207af07...
```

Commands:

```sh
shasum -a 256 res/Maps/Lev2_Fire2.unr \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr \
  res/System/{Engine.dll,Engine.u,HPBase.u,HarryPotter.u,HPModels.u} \
  /Volumes/HARRY_POTTER_EFG/System/{Engine.dll,Engine.u,HPBase.u,HarryPotter.u,HPModels.u}
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr props 2126
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  /Volumes/HARRY_POTTER_EFG/System/HarryPotter.u classprops 835
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  /Volumes/HARRY_POTTER_EFG/System/Engine.u classprops 1
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  /Volumes/HARRY_POTTER_EFG/System/Engine.u classprops 0
```

Export indices differ, but the effective data does not:

| Effective item | Local `res/` | Mounted edition |
| --- | --- | --- |
| Map actor export | 2132 | 2126 |
| Actor name/class | `rolllog1` / `rolllog` | same |
| Location / yaw | `(-2.5672,-1125.587,6.8568)` / `-16384` | byte-identical values |
| Collision radius/width/height/type | `40 / 115 / 89.891426 / 2` | byte-identical values |
| Per-instance Physics/PrePivot/Mesh/DrawScale/animation override | absent | absent |
| Class export | `HarryPotter.u:839` | `HarryPotter.u:835` |
| Effective `bAlignBottom` | true from `Engine.Actor` | true from `Engine.Actor` |
| Effective `bCollideWorld` | true from `Engine.Pawn` | true from `Engine.Pawn` |
| Effective serialized Physics | inherited `PHYS_None` until auto state | same |
| Class collision height/type | `0 / CT_Shape(3)` | same raw bytes |
| Map override collision height/type | `89.891426 / CT_Box(2)` | same raw bytes |
| Draw type / draw scale | `DT_Mesh(2) / 1.0` | same |
| Effective mesh | `HPModels.sklogMesh` export 609 | same package/name/export |
| Mesh origin/scale/bounds/default animation | zero/unit/same bounds/export 496 | byte-identical decoded values |
| `walkAnimName` / `idleAnimName` | `Roll / Stop` | `Roll / Stop` |
| `Pawn.MaxStepHeight` | 25.0 | 25.0, same raw bytes |

The mounted map actor uses the same initial-state sentinel and the mounted
compiled `waitforspell` again begins with native 3970
`SetPhysics(ByteConst 1)` followed by native 260 `LoopAnim(NameConst Stop)`.
Its inherited `HPBase.u:2899 baseChar.patrol` again begins by enabling Tick,
calling `SetPhysics(ByteConst 1)`, then `startup` and the walk-animation
virtual. The mounted `TakeSpellEffect` export 1365 still compares `spellFlip`,
plays the roll sound, and performs native `GotoState('patrol')`; this edition
returns the function's default value instead of local `res/`'s explicit true,
but that does not alter the state/physics/transform transition.

Result: an edition-specific `bAlignBottom`, `bCollideWorld`, `Physics`,
`CollideType`, mesh, collision-size, or animation default does **not** explain
the symptom. The alternate original package set independently reproduces the
same problematic authored/default contract.

### 10. OpenHP1 causal trace

OpenHP1's observed transition follows the same inputs and retail gates:

1. Scene assembly resolves the map/class defaults into `ActorState` in
   `crates/openhp1-scene/src/loader.rs`. Before script execution its inherited
   `Physics` is zero, so `skeletal_mesh_adjust` returns zero. The authored mesh
   initially spans approximately world Z `6.91..96.75`.
2. The auto-state begin code executes native 3970. OpenHP1 writes
   `Physics=PHYS_Walking` and emits `ActorAction::SetPhysics` from
   `crates/openhp1-runtime/src/world/physics.rs`.
3. `crates/openhp1-scene/src/runtime.rs` projects that action through
   `LoadedScene::set_actor_physics`. Because the value crosses zero to
   nonzero, `runtime_display.rs` rebuilds the skeletal actor. The two local
   shipped `ApplyAnim` implementations recompute `GetMeshCoords` every frame,
   but the retail recording proves that this fact alone is insufficient to
   declare OpenHP1's rebuild result equivalent to every retail build.
4. The rebuilt render applies `A=-92.4435`. Its mesh spans approximately world
   Z `-85.53..4.31`, while actor `Location` and collision remain centered at
   Z `6.8568`. That exactly explains a buried visual, correct obstruction, and
   a target effect computed from the unchanged collision box.
5. Flipendo enters `patrol` and begins horizontal `MoveTo`. The retail and
   OpenHP1 walking paths can then step the blocked actor upward; later location
   changes translate the collision and render together. That explains why the
   symptom clears only when movement starts without implying a spell-specific
   render fix.

The arithmetic above reproduces OpenHP1's symptom; it no longer proves that
the physics-triggered rebuild introduced by commit `239bf31` is correct or
incorrect. The shipped binary rejects only the narrow stale-cache explanation:
retail recomputes coordinates during `ApplyAnim`. A still-untraced condition
must make at least one input or gate differ in the observed retail initial
frame. The later reverted `cde89e8` change, which called a generic
spawn-location search on `PHYS_None -> PHYS_Walking`, remains disproven:
native `setPhysics` calls only `FindBase`, and the load/idle traces above prove
that neither path performs that relocation.

### 11. Rejected hypotheses after the package/native trace

| Hypothesis | Result |
| --- | --- |
| `rolllog1` persistently remains in `PHYS_None` until Flipendo | not established: compiled `waitforspell` requests walking at `Begin`, but exact retail processing/live Physics timing remains the open gate |
| Flipendo selects a different physical collision primitive | rejected: neither state changes `CollideType` or dimensions |
| `Roll` has a root translation that lifts the mesh into place | rejected: representative samples report zero root motion; `Stop` and `Roll` phase zero have essentially identical local Z bounds |
| The target is a separate misplaced authored actor | rejected: it is spawned by `HPBase.target`, positioned from the victim's world collision box, then trailed to the victim |
| A per-instance `PrePivot`/mesh/animation override is corrupt | rejected: none is serialized on map export 2132 |
| The log's initial destination begins on the floor at the authored center height | rejected: `HPath_A1` and `baseStation5` are both at Z about 43.2, substantially above the initial center Z 6.8568 |
| Render passes a copied/proxy actor with different physics or collision flags | rejected: `FDynamicSprite+0x98`, `DrawMesh`, both skeletal `GetFrame` slots, and `ApplyAnim` preserve the same actor pointer |
| `bAnimByOwner` causes `GetMeshCoords` to use another actor | rejected for `rolllog1`: the native branch exists, but every authored layer leaves the flag false and Owner null |
| Auto-state entry skips the leading `SetPhysics(PHYS_Walking)` | rejected: the compiled `Begin` label points to bytecode offset zero |

### 12. Exhaustive shipped-script modifier audit

The retail recording reopened one remaining package-side possibility:
some other actor or inherited script might change `rolllog1.Physics`,
`bAlignBottom`, or `bCollideWorld` before the first visible frame. The local
and mounted shipped packages were therefore searched independently by literal
name, serialized actor reference, compiled field reference, native-call
opcode, class ownership, lifecycle source, and stasis defaults. This audit
found no such pre-visibility modifier.

#### No authored actor or compiled script selects `rolllog1`

A binary literal search over every shipped system and map package finds
`rolllog` only in `HarryPotter.u` (the class/source/name metadata) and
`Lev2_Fire2.unr` (the two placed log instances). The mounted edition has the
same result. No other package can contain a compiled `NameConst('rolllog')` or
`ObjectConst(class'rolllog')` without containing that name-table string.

The stronger structured probes agree:

- decoding every actor in local `Lev2_Fire2.unr` and examining every `Name`,
  `Object`, and `Class` tagged property finds the name `rolllog` only in the
  own `Tag` properties of exports 2132 (`rolllog1`) and 2134 (`rolllog5`);
  no actor has `Event=rolllog`, an attachment/interpolation tag of `rolllog`,
  or an object/class reference to export 2132;
- the mounted map gives the same result for exports 2126 and 2128;
- scanning every compiled `Class`, `State`, `Function`, and `Struct` bytecode
  body in local `HarryPotter.u` finds no encoded reference to rolllog class
  export 839 (object reference `840`) and no `NameConst` operand for name-table
  index 1574 (`rolllog`);
- the corresponding mounted references, class export 835 (`836`) and name
  index 1564, likewise have zero compiled-bytecode hits.

This closes direct selection by placed object reference, class literal,
constant tag/event name, attachment, or interpolation property. It also closes
the one generic gameplay trigger which could otherwise affect this subclass:
embedded `TriggerSetBaseCharOnPatrolPath.ProcessTrigger` iterates
`AllActors(class'baseChar', a, Event)` and sends matching actors to `patrol`,
but no placed actor supplies `Event=rolllog`. Its body is not an automatic
lifecycle hook.

Reproduction commands:

```sh
rg -a -l -i 'rolllog' res/System res/Maps
rg -a -l -i 'rolllog' \
  /Volumes/HARRY_POTTER_EFG/System /Volumes/HARRY_POTTER_EFG/Maps
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/Maps/Lev2_Fire2.unr actorrefs 2132
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr actorrefs 2126
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u scriptrefs 840
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u scriptrefs 1574
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  /Volumes/HARRY_POTTER_EFG/System/HarryPotter.u scriptrefs 836
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  /Volumes/HARRY_POTTER_EFG/System/HarryPotter.u scriptrefs 1564
strings -a res/System/HPBase.u | \
  rg -n -A30 -B10 'class TriggerSetBaseCharOnPatrolPath'
```

`actorrefs` and `scriptrefs` are read-only modes in the temporary
`/tmp/rollprobe` investigation program. The former decodes the Level actor
array and tagged properties; the latter decodes each script export and checks
the package-version bytecode operands rather than searching decompiled prose.

#### No compiled class-chain writer exists for either renderer flag

The gameplay packages cannot silently assign the two renderer-gate flags via
compiled UnrealScript:

- `HarryPotter.u` imports `Physics` (import 65, encoded `-66`) and
  `bCollideWorld` (import 124, encoded `-125`), but imports neither
  `bAlignBottom`, `bStasis`, nor `bForceStasis`;
- `HPBase.u` imports `bCollideWorld` (import 241, encoded `-242`), but imports
  neither `Physics`, `bAlignBottom`, `bStasis`, nor `bForceStasis`;
- searching compiled `Engine.u` bytecode for its own `bAlignBottom` export
  reference 5909, `bStasis` reference 3700, and `bForceStasis` reference 3701
  yields no function/state hits;
- the embedded shipped source buffers contain no assignment to
  `bAlignBottom` in any of these three packages.

Every compiled `bCollideWorld` candidate was resolved through its outer chain.
In `HPBase.u` they belong only to `BaseCam.SetCollisionState` and
`FireSeeds.pickupSeed`. In `HarryPotter.u` they belong only to `QuidCam`,
`BroomHarry`, `QuidPlayer`, and `ChocolateFrog`. None is a member inherited by
`rolllog`, and the selection audit above finds no reference through which one
could target `rolllog1`. The only external `bCollideWorld` assignments in
shipped `Engine.u` concern an explicitly triggered interpolation instigator or
freshly spawned/dropped inventory; neither is a map-start lifecycle path for
this actor.

Likewise, exact compiled `Physics` property references in `HarryPotter.u`
belong to `Harry` and its player states. Those in `HPBase.u` belong to
`baseProps`, `Target`, `SpellLearnTrigger`, and HUD classes. This is distinct
from calls to native `SetPhysics`, so all compiled native-3970 call sites were
also enumerated. The only inherited calls that apply to the log are the
already-proven `rolllog.waitforspell` call at export 1366 offset zero and
`baseChar.patrol` at `HPBase.u` export 2660 offset 7. External receivers in
the embedded source are limited to explicit carried/caught actors and newly
spawned trail/particle effects; no map-start path selects the log.

The broad engine-level `AllActors(class'Pawn', P)` occurrences were checked
separately because they do not need the name `rolllog`:

- two network prediction loops only apply `MoveSmooth` to a different,
  already-moving blocking pawn;
- the mutator loop only registers player HUD mutators;
- `PlayerPawn.GameEnded.BeginState` sets every pawn to `PHYS_None`, but is
  entered only on the game-ended state, not during map actor initialization.
  It would also disable the alignment gate rather than create OpenHP1's buried
  result.

No generic startup pawn loop changes the log's Physics or either flag.

Reproduction commands:

```sh
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u findimports bAlignBottom
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u findimports bCollideWorld
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u findimports bStasis
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPBase.u findimports bCollideWorld
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u scriptrefs 5909
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u scriptrefs 3700
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u scriptrefs 3701
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u scriptrefs -125
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPBase.u scriptrefs -242
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u setphysics
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPBase.u setphysics
strings -a res/System/Engine.u | \
  rg -n -A14 -B8 "AllActors\\(class'Pawn'"
strings -a res/System/{Engine,HPBase,HarryPotter}.u | \
  rg -n 'bAlignBottom\\s*='
```

#### Stasis does not provide a hidden state transition

`Engine.u` declares `bStasis` and `bForceStasis` as opt-in actor booleans. The
raw class-default chain contains no true tag for either property in `Actor`,
`Pawn`, `baseChar`, or `rolllog`, and map export 2132 has no override. Their
effective values are therefore false. Neither gameplay package imports these
fields and no compiled `Engine.u` state/function references them. Even the
embedded declaration describes stasis as suppressing updates for eligible
actors; it does not define a write to Physics or the collision/alignment bits.
Stasis cannot account for a pre-draw gate change here.

#### Embedded source and compiled initial-state entry agree exactly

The embedded `rolllog` source says `auto state waitforspell`, with `Begin:`
immediately calling `SetPhysics(PHYS_Walking)` and `LoopAnim('Stop')`. This is
not accepted on source text alone. The map actor's serialized state frame has
`function=rolllog`, `state=rolllog`, `latent_action=0`, and
`bytecode_offset=-1`, the engine's uninitialized-state sentinel. Compiled
`HarryPotter.u` export 1366 is a State with AUTO flag `0x2` and label-table
offset `0x20`; `Begin` resolves to bytecode offset zero and `Loop` to `0x0d`.
The first three bytes at zero are the extended-native-3970 `SetPhysics`
opcode, followed by byte constant 1 (`PHYS_Walking`), and the next call is
native 260 `LoopAnim` with `Stop`.

Thus there is no source/compiled discrepancy, no alternate compiled entry
label, and no serialized resume offset that skips the leading physics call.
Together with the native load/tick proof above, the first possible post-travel
draw is bounded to Physics none, but the first effective actor tick executes
the leading walking call. It does not reveal an evidence-backed package or
UnrealScript correction for stable pre-spell visibility.

```sh
strings -a res/System/HarryPotter.u | \
  rg -n -A14 -B8 'auto state waitforspell'
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/Maps/Lev2_Fire2.unr props 2132
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u 1366
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u scriptbytes 1366
```

### 13. Shipped executable and configuration startup audit

The final package-adjacent startup route was the game launcher itself or an
INI-loaded default: HP.exe could in principle select a custom game engine, or
Unreal's configuration loader could replace an actor default after package
deserialization. The legally obtained executable and all shipped INI/INT files
were inspected read-only; neither route exists for this actor.

#### HP.exe selects the stock engine and is not in the log's native class path

Local `res/System/HP.exe` is a 32-bit PE executable with SHA-256
`75fea8e8ef096936bdfe11b615459ccd459492a9b164818503fbc56c1c68aea9`.
The mounted original edition has a different whole-file hash,
`0de4de184f707a3598b5fb5d946eba897d7a47f61ba599ac329a45662c4d2169`,
but the same relevant PE interface. Each imports exactly one symbol from
Engine.dll:

```text
?PrivateStaticClass@UGameEngine@@0VUClass@@A
```

Neither executable's PE import table contains `AActor`, `APawn`, `ULevel`,
`SetPhysics`, `ProcessState`, `SetInitialState`, or a collision function. The
readable narrow/wide identifier pools likewise contain no `rolllog`, Physics,
CollideType, alignment, collision-world, or actor-state name. The executable's
wide guard labels describe the generic launch loop (`InitEngine`, `MainLoop`,
`UpdateWorld`, and `EnforceTickRate`), not a custom actor hook.

That static-import observation is not used alone: the package/config ownership
chain closes the indirect alternative. Every shipped language-specific
`Default.ini` sets:

```ini
[Engine.Engine]
GameEngine=Engine.GameEngine
DefaultGame=Engine.GameInfo
DefaultServerGame=Engine.GameInfo
```

No `HP.*` or other custom `UGameEngine` subclass is selected. `rolllog`
(`HarryPotter.u:839`) and `baseChar` (`HPBase.u:4`) have compiled class flags
`0x36` and contain UnrealScript; their native ancestors are exactly
`Engine.Pawn` and `Engine.Actor`, whose native implementation is the already
traced Engine.dll. The package dependency lists for `HarryPotter.u`,
`HPBase.u`, `Hub2.u`, and `Hog2.u` do not import an `HP`/executable native
package, and the map itself imports no such package. Therefore HP.exe supplies
neither the actor's class, a custom game-engine override, nor a native function
in the log's inherited startup/state/physics path.

The shipped executable is copy-protected/obfuscated: its additional `stxt*`
sections and encrypted-looking `.text` make a claim about every internal
instruction unjustified without executing or dynamically unpacking it, which
is prohibited here. The evidence-backed conclusion is narrower and sufficient:
there is no configured class or package edge by which HP.exe owns this actor's
lifecycle, and no static engine/actor API import indicating a direct hook.
This audit does **not** treat the modified `res/NoCD/HP.exe` as original-engine
evidence.

Reproduction commands:

```sh
file res/System/HP.exe /Volumes/HARRY_POTTER_EFG/System/HP.exe
shasum -a 256 res/System/HP.exe \
  /Volumes/HARRY_POTTER_EFG/System/HP.exe
/opt/homebrew/opt/llvm/bin/llvm-objdump -p res/System/HP.exe
/opt/homebrew/opt/llvm/bin/llvm-objdump -p \
  /Volumes/HARRY_POTTER_EFG/System/HP.exe
/opt/homebrew/opt/llvm/bin/llvm-objdump --section-headers res/System/HP.exe
strings -a -t x res/System/HP.exe | \
  rg -i 'rolllog|Physics|CollideType|AlignBottom|CollideWorld|ProcessState|SetInitialState'
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/Maps/Lev2_Fire2.unr packages
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u packages
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPBase.u packages
```

The wide-string check was a read-only scan for runs of printable UTF-16LE
characters and reported the guard labels quoted above.

#### The renderer-gate and scheduling fields are not configurable properties

Every shipped `.ini` and `.int` in both original editions was searched for
`Physics`, `CollideType`, `bAlignBottom`, `bCollideWorld`, `bStasis`,
`bForceStasis`, and `bAlwaysTick`. There is no assignment and no class section
for `Engine.Actor`, `Engine.Pawn`, `HPBase.baseChar`, or
`HarryPotter.rolllog`. The language-specific `Default.ini` variants differ in
the `Language` value, not in engine/game/default classes or actor settings.

More decisively, the compiled Engine.u property metadata says these fields are
not eligible for Unreal's config overlay. Their exact property flags are:

| Engine.u property export | Field | Flags |
| --- | --- | --- |
| 81 | `Physics` | `0x23` |
| 160 | `bCollideWorld` | `0x21` |
| 3697 | `bAlwaysTick` | `0x02` |
| 3699 | `bStasis` | `0x01` |
| 3700 | `bForceStasis` | `0x01` |
| 5908 | `bAlignBottom` | `0x01` |
| 5910 | `CollideType` | `0x21` |

None contains the `CPF_Config` bit `0x4000`. As a positive control, the
`Engine.GameEngine.CacheSizeMegs` property used by `[Engine.GameEngine]` is
export 5982 and has flags `0x4001`, including that bit. Thus an INI section
could not legally rewrite any of the log fields relevant to mesh adjustment,
tick/stasis, or physics even if such a key were present.

The only configured `ServerActors` are the stock UDP beacon/query/uplink and
web server classes. They do not create a custom game engine or appear in the
map actor/property reference chain. The exact map `LevelInfo0` has no
GameInfo/class override; its tagged fields are title/author/summary, visible
groups, detail/profile values, and ordinary Actor state. There is no
configuration-driven or LevelInfo-driven startup mutation of `rolllog1`.

Reproduction commands:

```sh
find res/System -type f \( -iname '*.ini' -o -iname '*.int' \) -print
find /Volumes/HARRY_POTTER_EFG/System -type f \
  \( -iname '*.ini' -o -iname '*.int' \) -print
rg -n -i \
  '(^|[.=])(Physics|CollideType|bAlignBottom|bCollideWorld|bStasis|bForceStasis|bAlwaysTick)(\[|=|$)' \
  res/System /Volumes/HARRY_POTTER_EFG/System \
  --glob '*.ini' --glob '*.int'
diff -u res/System/0/Default.ini res/System/1/Default.ini
diff -u res/System/0/Default.ini \
  /Volumes/HARRY_POTTER_EFG/System/0/Default.ini
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 81
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 160
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 3697
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 3699
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 3700
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 5908
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 5910
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 5982
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/Maps/Lev2_Fire2.unr props 0
```

#### Imported map packages expose no additional generic startup mutator

`Lev2_Fire2.unr` imports its gameplay actors from ordinary script packages,
principally Engine, HPBase, HarryPotter, Hog2, Hub2, HProps, and HPParticle.
The previous structured actor scan was extended to serialized string
properties so name-driven cutscene helpers are covered as well: neither local
nor mounted map contains `rolllog`/`rolllog1` in another actor's `Name`,
`Object`, `Class`, or `Str` property.

The remaining generic pawn iterators in these imported packages were inspected.
The two `HPBase` cutscene helpers only implement `findPawn(string name)` and
return a name match to an explicitly started cutscene; no placed serialized
cutscene string names this log. Engine's generic Pawn loops remain the network
prediction, HUD-registration, and game-ended paths listed in section 12.
Hog2, Hub2, HProps, and HPParticle contain no generic map-start Pawn loop that
changes Physics, CollideType, collision/alignment flags, state execution, or
tick eligibility. Their external `SetPhysics` calls have explicit receivers
such as spawned smoke/trails/fragments, the player, Draco, or the Snitch.

Consequently the executable, config, LevelInfo, and imported-package startup
surfaces add no pre-stable-draw state change beyond the Engine.dll lifecycle
and compiled auto-state path already traced. This is another negative result,
not an implementation rule: it does not explain the original game's visible
log and authorizes no OpenHP1 change.

### 14. `bAssimilated` is moving-brush state, not a skeletal-actor render cache

The transient `Actor.bAssimilated` flag was investigated as a possible missing
state boundary. Its source comment is superficially promising: "Actor dynamics
are assimilated in world geometry." If retail captured a skeletal actor's
transform while Physics was still none and retained that transform until the
actor moved, such a flag could have explained both the initially visible log
and its post-Flipendo transition. The shipped package metadata and both native
binaries reject that interpretation.

#### Property identity and native bit are exact

`Engine.u` export 3674 is the `BoolProperty` `Actor.bAssimilated`. Its compiled
property flags are `0x2002`: it is transient and is not a serialized/config
field. The embedded Actor declaration places it immediately after the first
ten Actor booleans:

```uc
var const bool            bDeleteMe;
var transient const bool  bAssimilated;  // Actor dynamics are assimilated in world geometry.
var transient const bool  bTicked;
```

The native access confirms both the storage word and the bit rather than
inferring them only from declaration order. Engine.dll loads the Actor flags
at `actor+0x28`, and its moving-brush code tests/sets `AH & 0x04`, which is
whole-word mask `0x00000400`:

```text
0x1038c334  mov eax,[esi+0x28]
0x1038c337  test ah,0x04
0x1038c340  or   ah,0x04
0x1038c343  mov [esi+0x28],eax
```

The same routine clears exactly that bit with `and ah,0xfb` at
`0x1038c561..0x1038c56a`; the brush rebuild entry at
`0x1038d5e0..0x1038d5fb` performs the same clear. A second insertion path at
`0x1038e533..0x1038e542` performs the same test/set. Generated Actor and
derived-class copy routines also copy mask `0x400`, but they contain the
standard source/destination `xor; and 0x400; xor` bitfield-copy sequence and
are not lifecycle consumers.

#### Every lifecycle writer is gated to dynamic `ABrush` actors

The class gate around those flag writes is explicit. The native static-class
object at `0x105e91a0` is `ABrush`: Engine.dll exports
`ABrush::InternalConstructor`, and the registration body at `0x1038c010`
associates that constructor with this class object. The moving-brush tracker
constructor at `0x1038cb80` scans `ULevel.Actors`, but admits an actor only
after all of these checks:

1. `actor+0x168` is non-null (`Brush`, an Engine `UModel`);
2. the class chain reaches the `ABrush` class object `0x105e91a0`;
3. Actor flag bit zero, `bStatic`, is clear.

The corresponding scan/update body at `0x1038c260` repeats the same non-null
Brush, `ABrush`, and non-static gates before it reaches the `bAssimilated`
test/set at `0x1038c334`. Its single-brush path stores snapshots of live
`Location` (`actor+0xfc`) at `actor+0x3f8` and live `Rotation`
(`actor+0x108`) at `actor+0x41c`; later calls compare those snapshots and queue
the brush for a world-model rebuild when the transform differs. During that
rebuild the routine clears `bAssimilated`, transforms the Brush model's
polygons, edits the level model's nodes/surfaces, and re-enters the tracker's
brush insertion paths. These are moving-BSP-brush operations, not mesh pose or
skeletal-coordinate operations.

The rolllog cannot enter this code. Its compiled inheritance chain is
`rolllog -> baseChar -> Pawn -> Actor`, not `ABrush`. Its class defaults set
`DrawType=DT_Mesh` and `Mesh=sklogMesh`; neither the `rolllog` defaults nor
map export 2132 serialize a `Brush`, and the exact map instance has no Brush
model reference. Therefore both the class and non-null-model prerequisites
fail before any `bAssimilated` access.

#### Ordering and invalidation do not supply a hidden pre-spell mesh state

The moving-brush tracker is created through the exported
`GNewBrushTracker(ULevel*)` thunk (`0x103023e7 -> 0x1038e810`). In the traced
`UGameEngine::LoadMap` route, that call is at `0x1039e5a0`. It is later in the
same function than all four new-level actor lifecycle dispatches:

```text
0x1039dec1  ENGINE_PreBeginPlay
0x1039df00  ENGINE_BeginPlay
0x1039df72  ENGINE_PostBeginPlay
0x1039dfb1  ENGINE_SetInitialState
0x1039e5a0  GNewBrushTracker(new level)
```

Thus even applicable dynamic brushes are collected after `SetInitialState`,
not before it. More importantly, the rolllog is excluded from collection.

`AActor::setPhysics` at `0x103e5140..0x103e523b` contains no access to
`actor+0x28`, no `0x400` mask operation, and no moving-brush-tracker access;
its only calls in this range are `SetBase` and `FindBase`. `ULevel::MoveActor`
at `0x103aa3a0` likewise contains no `bAssimilated` mask operation. Movement
of an applicable brush is noticed later by the tracker through the explicit
live-versus-saved Location/Rotation comparisons above. There is no
Physics-transition invalidation and no generic mesh-movement invalidation
associated with this flag.

Finally, a complete instruction-form audit of Render.dll found no read or
write of Actor mask `0x400`. The one `test ...,0x400` in Render.dll operates on
a locally shifted byte in texture/span decoding (`0x10b02a0a..0x10b02a22`),
and its lone `test ah,0x04` reads a texture field at `[ebx+0x4]`
(`0x10b2598d..0x10b2599c`), not Actor flags. The shipped skeletal drawing path
therefore never branches on `bAssimilated` and cannot retain an initial
Physics-none skeletal transform through it.

This lead is rejected by primary evidence. Implementing an Actor assimilation
cache for the log, invalidating such a cache on `SetPhysics` or `MoveActor`, or
using `bAssimilated` to bypass fresh skeletal coordinates would invent behavior
that is absent from the shipped Engine.dll/Render.dll and would also conflate
skeletal actors with the engine's dynamic BSP-brush tracker.

Reproduction commands:

```sh
strings -a res/System/Engine.u | rg -n -A5 -B8 'bAssimilated'
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/Engine.u property 3674
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HarryPotter.u classprops 839
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/Maps/Lev2_Fire2.unr props 2132
/opt/homebrew/opt/llvm/bin/llvm-objdump -p res/System/Engine.dll | \
  rg 'ABrush|GNewBrushTracker'
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1038c260 \
  --stop-address=0x1038c580 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1038cb80 \
  --stop-address=0x1038cd10 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1038d500 \
  --stop-address=0x1038d620 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1038e4a0 \
  --stop-address=0x1038e610 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1039dec0 \
  --stop-address=0x1039e5b0 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103e5140 \
  --stop-address=0x103e5240 res/System/Engine.dll
```

#### Independent `FCoords`, root-pose, culling, and LOD sign audit

Because the observed retail recording contradicts the static placement model,
the complete coordinate sign was independently re-derived from the shipped
`Core.dll`, not inferred from variable names or from OpenHP1's transform code.
This audit confirms that the negative mesh adjustment reaches the actual
vertex path as a **world-space downward** translation.

The relevant imported Core functions were identified from the shipped PE IAT,
whose Core table begins at `0x10608b28`:

| Engine IAT slot | Imported Core symbol | Core body |
| --- | --- | --- |
| `0x106092ac` | `FCoords::operator/=(FCoords const&)` (IAT index 481) | `0x1014e900` |
| `0x106092bc` | `FCoords::Inverse()` (IAT index 485) | `0x1014e700` |
| n/a, export thunk `0x10101c8f` | `FVector::TransformPointBy(FCoords const&)` | `0x1011c620` |
| n/a, export thunk `0x1010296e` | `FCoords::operator*=(FCoords const&)` | `0x1011d1c0` |

The machine code fixes the otherwise confusing stored-`Origin` convention:

- `TransformPointBy` at `0x1011c63a..0x1011c69a` computes
  `(point - coords.Origin)` dotted with the three stored axes.
- `operator*=` begins with the identical origin transform at
  `0x1011d1dc..0x1011d23c`, then composes the axes.
- `operator/=` is not a sign-flipping subtraction. At
  `0x1014e911..0x1014ec7d` it computes each new axis as the right-hand axes
  times the corresponding old axis. At `0x1014ec82..0x1014ed01` it computes
  `new_origin = right.origin + right.axes * old_origin`. This is the forward
  affine composition used after taking an inverse.

In the real skeletal call, `GetFrame` first calls `caller_coords.Inverse()` at
`0x1041e002..0x1041e010`, copies the freshly cached `GetMeshCoords` result, and
then calls that result's `operator/=` with the inverse at
`0x1041e018..0x1041e02e`. For the translation-only reduction, if the camera is
at `Q` and the mesh origin is at `W`, `caller_coords.Inverse()` has origin
`-Q`, and the composition produces `W-Q`. Thus the cached mesh origin is a
world point converted to render/view space; it is not negated a second time.
For `rolllog1`, whose authored rotation is yaw-only, the actor transform leaves
the world Z axis unchanged and

```text
W.z = Location.z + PrePivot.z + MeshAdjust.z
```

before camera conversion. An arbitrary pitched camera can distribute a world-Z
change among view components, but projection still depicts the same world
point; it cannot cancel that actor-specific translation.

Both shipped skinning output branches then use the composed coordinates in the
forward form `origin + XAxis*x + YAxis*y + ZAxis*z`. The one-influence path is
at `0x1041e3a1..0x1041e40f`; the weighted path performs the same operation per
influence at `0x1041e4d3..0x1041e526`. Consequently `A = -92.4435` contributes
`+A` to world Z. Using the shipped `Stop` pose bounds, the reduction remains:

```text
minimum world Z ~= 6.8568 - 92.4435 + 0.0460 = -85.5407
maximum world Z ~= 6.8568 - 92.4435 + 89.8975 = 4.3108
```

The shipped pose also contains no hidden root lift that cancels this value. A
read-only decode of `HPModels.u` exports 609/496 gives the single `Stop` root
bone at local Z `44.97175`; the skinned result remains local Z
`0.046028..89.89747` at every sampled phase. The bone is simply near the
log's center, with skin-local vertices on both sides. `Roll` phase zero has
the same relationship (root Z `44.97147`, skinned local Z
`0.045731..89.89721`). Neither sequence supplies a `+92.4435` translation,
and extracted root motion is zero.

Two neighboring paths were checked for a late placement change:

- `USkeletalMesh::GetRenderBoundingBox` is exported through `0x10303f35` to
  `0x1041b2f0`. It calls `ApplyAnim(actor, ..., true)` at `0x1041b325`, obtains
  the current pose box, and returns bounding coordinates through
  `0x1041b37b..0x1041b436`. It does not write `Actor.Location`, the caller's
  rendering coordinates, or the vertex buffer. It supplies culling/extent
  data only.
- The actual drawing route is `DrawMesh`'s `ULodMesh` class dispatch at
  `0x10b0e963..0x10b0e991`, then `DrawLodMesh` at `0x10b0ff00`. LOD selection
  changes the requested vertex count (`0x10b10001..0x10b1011d`). At
  `0x10b10243..0x10b10252`, Render chooses either the caller's view coordinates
  or `GMath+0x18`; Render's IAT identifies `0x10b7450c` as the imported
  `Core.GMath`, and the shipped `FGlobalMath` constructor at
  `0x1014e0b5..0x1014e1bf` initializes `+0x18` to identity coordinates. The
  choice is therefore view-space CPU vertices versus world-space vertices for
  a later device transform, not an actor offset. The original actor and chosen
  coordinates are passed to `USkeletalMesh::GetFrame` at
  `0x10b10252..0x10b10270`. LOD changes sampling density and output space, not
  actor placement; both choices retain `A` as the same world-space shift.

This re-audit rejects a sign reversal, root-bone compensation, bounding-box
placement, or LOD-specific transform as the missing retail condition. It also
corrects the earlier route description: the generic slot-`+0x7c` wrapper exists
and reaches the same five-argument body, but the log actually enters that body
directly through `DrawLodMesh`'s slot-`+0xa4` call.

Reproduction commands:

```sh
/opt/homebrew/opt/llvm/bin/llvm-readobj --coff-imports \
  res/System/Engine.dll | \
  awk '/ImportAddressTableRVA: 0x308B28/{p=1;i=0;next} \
       p && /^  Symbol:/{if(i>=475 && i<=485) print i,$0;i++} p && /^}/{exit}'
objdump -d --start-address=0x1011c620 --stop-address=0x1011c6c0 \
  res/System/Core.dll
objdump -d --start-address=0x1011d1c0 --stop-address=0x1011d340 \
  res/System/Core.dll
objdump -d --start-address=0x1014e700 --stop-address=0x1014ed10 \
  res/System/Core.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041df50 \
  --stop-address=0x1041e610 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041b2f0 \
  --stop-address=0x1041b440 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10b0e920 \
  --stop-address=0x10b10280 res/System/Render.dll
/opt/homebrew/opt/llvm/bin/llvm-readobj --coff-imports \
  res/System/Render.dll | \
  awk '/ImportAddressTableRVA: 0x74460/{p=1;i=0;next} \
       p && /^  Symbol:/{if(i>=40 && i<=45) print i,$0;i++} p && /^}/{exit}'
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1014e080 \
  --stop-address=0x1014e270 res/System/Core.dll
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPModels.u sample 609 496
```

### Collision-size lifecycle: the stable retail height remains `89.891426`

The possibility that `Pawn.PreBeginPlay` or another startup hook shrinks the
log's collision cylinder before the first stable skeletal draw is rejected by
compiled package data and both shipped `Engine.dll` builds.

The map instance carries all three dimensions as explicit tagged overrides:

| field | serialized bytes | value |
| --- | --- | --- |
| `CollisionRadius` | `00 00 20 42` | `40.0` |
| `CollisionWidth` | `00 00 e6 42` | `115.0` |
| `CollisionHeight` | `69 c8 b3 42` | `89.891426` |
| `CollideType` | `02` | `CT_Box` |

The local package stores these on map export 2132. The independently mounted
original installation stores identical bytes on its corresponding `rolllog1`
export 2126. The export-number difference is package-table layout, not actor
data.

The embedded `Pawn.PreBeginPlay` text contains a tempting scale-dependent
line:

```uc
/*if ( DrawScale != Default.Drawscale )
    SetCollisionSize(Default.CollisionRadius*DrawScale/Default.DrawScale,
                     Default.CollisionHeight*DrawScale/Default.DrawScale);
```

It is inactive source, not retail behavior. `SetCollisionSize` is native 283,
whose extended bytecode token is `61 1b`. The decoded token stream of local
`Engine.u` contains no native-283 call in any class, state, or function;
Pawn's compiled `PreBeginPlay` is export 1295 and has no such token. The
mounted `Engine.u` independently contains no native-283 call either.

The descendant hooks also do not supply one:

- local `baseChar.PreBeginPlay` is HPBase export 2693; mounted it is export
  2922. Its compiled bytecode contains no native-283 token;
- `rolllog` has no `PreBeginPlay`, `BeginPlay`, or `PostBeginPlay` member. Its
  only own executable members are the already documented states and their
  functions;
- scanning every compiled script export in local `res/System/*.u` and every
  map export in `res/Maps/*.unr` finds only seven native-283 call sites: two
  in `spellTrollRock`, two in `Harry.MountFinish`, and three in the Hub2
  `SpikyBush`/`SpikyBushNoThorns` classes. None is in the log's
  `rolllog -> baseChar -> Pawn -> Actor` ancestry, and the map scan finds none.
  The mounted primary packages agree for the corresponding
  `spellTrollRock` and `Harry.MountFinish` sites.

The native implementation fixes what a call could change. In the local DLL,
the exported `AActor::SetCollisionSize` thunk at `0x103041a1` enters
`0x10379c80`; in the mounted DLL, thunk `0x10304197` enters `0x10379c40`.
Both bodies are instruction-for-instruction equivalent:

1. if `bCollideActors` is set, remove the actor from the level collision hash;
2. unconditionally store the first float argument at actor `+0x1cc`
   (`CollisionRadius`) and the second at `+0x1d4` (`CollisionHeight`);
3. if `bCollideActors` is set, add the actor back to the hash;
4. return without changing `Location`, `Physics`, `CollideType`, or any
   alignment flag.

The script wrapper is equally explicit. Local `execSetCollisionSize` is
`0x1040a870..0x1040a91a`; mounted it is
`0x1040a550..0x1040a5fa`. It decodes radius and height, initializes the
optional width argument from the actor's current `+0x1d0`, decodes an override
if supplied, writes that width back at `+0x1d0`, and calls the two-argument
native body. Therefore an omitted width preserves the current width; there is
no hidden dimension derivation or mesh-height substitution.

The remaining direct native writers do not alter this placed actor before a
stable draw:

- the `AActor` and `APawn` copy constructors and assignment operators copy
  `+0x1cc/+0x1d0/+0x1d4` from their source object unchanged;
- generic tagged-property deserialization supplies the explicit map values
  above. `AActor::Serialize` (`0x1037a540..0x1037a582`) merely delegates to
  its superclass and has no collision-dimension rewrite;
- `AActor::PostLoad` (`0x10379910..0x103799ba`) calls primitive
  `ValidateActor` only when `CollideType == 3`. This actor is explicitly type
  2, so it cannot reach `UBoxPrim::ValidateActor`
  (`0x103feb50..0x103febaa`), the native routine that derives dimensions from
  primitive bounds;
- `AActor::PreNetReceive`/`PostNetReceive` save and restore replicated radius
  and height around network property receipt. The log is an authority-role
  actor in standalone play, not a receiving network proxy; this is not a
  startup mutation. The relevant local `PostNetReceive` stores are at
  `0x10378a64..0x10378a82`;
- an exhaustive direct-offset store scan finds no other `AActor` lifecycle
  writer. Apparent matches outside these bodies operate on unrelated mesh,
  channel, connection, stack, or table structures.

Consequently the stable live tuple remains radius `40.0`, width `115.0`, and
height `89.891426`. The exact value read by `USkeletalMesh::MeshAdjust` is the
serialized `actor+0x1d4 = 89.891426`. The local read is at `0x1041ae50`; the
mounted read is at `0x1041ab00`. With the decoded mesh values `Origin.Z=0`, primitive
minimum Z approximately `0.052073`, mesh Z scale 1, and actor `DrawScale=1`,
both DLLs calculate:

```text
adjust_z = (0 - 0.052073) * 1 * 1 - (89.891426 + 2.5)
         = -92.443499 (approximately)
```

There is no evidence-backed smaller collision height that would suppress or
reduce that adjustment. Inventing one in OpenHP1 would be an actor-specific
workaround and is not authorized by this trace.

Reproduction commands:

```sh
/tmp/rollprobe/target/debug/rollprobe res/Maps/Lev2_Fire2.unr props 2132
/tmp/rollprobe/target/debug/rollprobe \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr props 2126
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u nativecalls 283
/tmp/rollprobe/target/debug/rollprobe res/System/HPBase.u nativecalls 283
/tmp/rollprobe/target/debug/rollprobe res/System/HarryPotter.u nativecalls 283
for p in res/System/*.u; do
  /tmp/rollprobe/target/debug/rollprobe "$p" nativecalls 283
done
for p in res/Maps/*.unr; do
  /tmp/rollprobe/target/debug/rollprobe "$p" nativecalls 283
done
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10379c70 \
  --stop-address=0x10379d20 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1040a860 \
  --stop-address=0x1040a920 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10379910 \
  --stop-address=0x103799c0 res/System/Engine.dll
objdump -d res/System/Engine.dll | \
  rg '0x1cc\\(|0x1d0\\(|0x1d4\\('
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041aab0 \
  --stop-address=0x1041ab30 /Volumes/HARRY_POTTER_EFG/System/Engine.dll
```

### 15. Retail local-pose output matches OpenHP1 and is rejected

The 2026-08-13 continuation identified and then closed one overstatement in
the preceding pose discussion. The quoted `Stop` bounds
`0.046028..89.89747` are the output of OpenHP1's
`Mesh::sample_skeletal_vertices`, not a direct retail dump. A reduction of the
retail one-bone path now independently reproduces the same result.

The exact shipped inputs for the one-bone log are:

- `sklogMesh` contains one reference bone, `Cylinder02`, at approximately
  `(0.19893, 0.06108, 44.97175)` with a quarter-turn X orientation;
- all 24 points have one influence of weight 1.0 on that bone;
- the `Stop` move has one track and one key, at approximately
  `(0.19901, 0.06176, 44.97175)`, with the equivalent quarter-turn
  orientation; and
- the decoded static points and serialized mesh bounds span local Z about
  `0.052..89.891`.

The shipped binary supplies the missing steps:

- `ApplyAnim`'s one-position-key branch at `0x1041bf86..0x1041bfd5`
  decompresses the three signed 16-bit components with the track scale, and
  its one-rotation-key branch at `0x1041bfdd..0x1041c039` reconstructs the
  quaternion. It copies that quaternion plus position as the current 28-byte
  `FPlace` at `0x1041c03f..0x1041c093`.
- The log's sole bone is the root. The parent-self check reaches
  `0x1041cabc`; actor flag byte `+0x28` bit `0x20` is `bAnimMove`, and the
  false branch jumps directly to the common store at `0x1041d40b`. Thus no
  root-motion correction changes this key. The seven dwords are stored in the
  skeletal cache at `0x1041d522..0x1041d540`.
- `GetFrame` calls `ApplyAnim` at `0x1041dfad`, constructs an `FCoords` from
  the cached root `FPlace` at `0x1041e06f..0x1041e095`, and takes its
  single-influence fast path because the mesh has exactly one bone. That path
  at `0x1041e35b..0x1041e40f` transforms every vector in the mesh's local-point
  array by the root coordinates.
- The imported `FCoords::FCoords(FPlace const&)` resolves to shipped
  `Core.dll` body `0x1014f7a0..0x1014f890`. It copies the `FPlace` position as
  the coordinate origin and expands the quaternion into the standard 3x3
  rotation matrix. For this key's approximately
  `(-0.70709, 0, 0, 0.70712)` quaternion, the retail point transform has
  `output_z = local_y + 44.97175` (within the serialized key's quantization).
  The local-point Y extrema are approximately `-44.9257..44.9257`, yielding
  retail local Z approximately `0.046..89.897`: the same bounds independently
  produced by OpenHP1.

Retail therefore does not lift the `Stop` pose relative to OpenHP1. The local
skeletal sampler is rejected as the missing pre-spell condition; the downward
`MeshAdjust` remains present after this pose is composed. No implementation
change is authorized by this closed avenue.

Reproduction commands:

```sh
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPModels.u 496
cargo run -q --manifest-path /tmp/rollprobe/Cargo.toml -- \
  res/System/HPModels.u sample 609 496
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041ba60 \
  --stop-address=0x1041bd10 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041df50 \
  --stop-address=0x1041e420 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041f3c0 \
  --stop-address=0x1041f550 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1014f7a0 \
  --stop-address=0x1014f8a0 res/System/Core.dll
```

### 16. The next decisive artifact is a pre-spell retail save

After closing the local-pose boundary, the contradiction is reduced to live
retail actor state. The shipped render gate can omit the approximately
`-92.4435` adjustment only if at least one of these inputs differs at the
retail draw: `Physics`, `CollideType`, `CollisionHeight`, `bAlignBottom`, or
`bCollideWorld`. The static default chain and normal lifecycle trace say the
draw should see walking, type 2, height `89.891426`, and both booleans true;
the retail recording says that model is either incomplete or version-specific.

Two final static checks do not reveal another branch:

- `Actor.SetInitialState` in the shipped `Engine.u` tests `InitialState` and
  otherwise calls `GotoState('Auto')`. Raw class-default scans of
  `Actor`, `Pawn`, `baseChar`, and `rolllog`, plus the map-instance tag scan,
  contain no `InitialState` override. Thus the earlier auto-state selection
  was not accidentally assuming an empty inherited name.
- The shipped console path has no per-instance `DISPLAYALL`, `GETALL`, or
  `EDITACTOR` command. `UObject::StaticExec` (`Core.dll`
  `0x1015cde0`) does expose `GET`, but its implementation resolves a class and
  reads that class's default property; it cannot report `rolllog1`'s live
  fields. `UEngine::Exec` and `UGameEngine::Exec` add no per-instance property
  display path.

A standard retail save can capture the missing state without modifying the
game or adding instrumentation. The shipped `UGameEngine::Exec` recognizes
`SAVEGAME` at `0x10399d4f..0x10399d92`, accepts a decimal slot, and dispatches
virtual `SaveGame`. `UGameEngine::SaveGame` is exported through
`0x1030333c` to `0x103a2280`; its format strings at `Engine.dll` file offsets
`0x1d7fd8` and `0x1d8908` are `%s\\Save%i%i.usa` for temporary level data and
`%s\\Save%i.usa` for the slot package. A save made while the log is visibly
correct and before Flipendo should therefore preserve the actor's live
property tags and state frame in `Save<slot>.usa`.

No existing `.usa` file was found under either shipped installation, the
local CrossOver bottles, or the Spotlight index. The investigation must not
run the original game under its stated constraints, so obtaining a fresh
pre-spell retail save is the shortest non-speculative way to distinguish the
five remaining gate inputs. Once supplied, it can be decoded read-only with
the same package probe already used for the map; no engine patch is justified
before that comparison.

#### Retail save route is unavailable

On 2026-08-13 the user confirmed that the original game cannot be run in the
available environment; only previously recorded retail video is available.
Consequently a fresh pre-spell `.usa` capture cannot be produced, and the
save-based live-property comparison above is an identified but unavailable
diagnostic rather than the next actionable step. Do not request another
retail run or save in a later session unless the available environment
changes.

The remaining direct behavioral evidence is the existing video. A useful
frame-level comparison must establish:

- the log's complete visible bounds relative to the floor before Flipendo,
  rather than merely that some pixels are visible;
- whether its placement changes at any time before Harry or the spell effect
  reaches it;
- the first frame on which its placement changes after Flipendo, relative to
  the spell hit, `Roll` animation start, and horizontal travel; and
- a scale reference that permits the pre/post vertical displacement to be
  estimated in Unreal units.

Those observations can distinguish a stable bypass of `MeshAdjust` from an
earlier untraced movement and can test whether OpenHP1's approximately
25-unit `stepUp` recovery actually matches the retail transition. Video alone
cannot directly reveal the five live property values, so it must not be used
to select one by guess.

Reproduction commands:

```sh
/tmp/rollprobe/target/debug/rollprobe res/System/HarryPotter.u \
  classprop 839 InitialState
/tmp/rollprobe/target/debug/rollprobe res/System/HPBase.u \
  classprop 4 InitialState
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u \
  classprop 0 InitialState
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u \
  classprop 1 InitialState
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1015cde0 \
  --stop-address=0x1015e400 res/System/Core.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10399d20 \
  --stop-address=0x10399da0 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103a2280 \
  --stop-address=0x103a2a50 res/System/Engine.dll
```

### 17. Serialized `OldLocation` is not a retail mesh-placement input

The map instance's nonzero `OldLocation=(0,-1136,23.6)` was reopened as a
possible render-interpolation source. If retail drew the stationary log from
that history vector while OpenHP1 drew from current `Location`, it could have
provided a missing pre-movement placement difference. The shipped native
layout and render call path reject that seam.

`AActor`'s generated copy constructor identifies the adjacent fields without
relying on a guessed property order: it copies current `Location` from
`actor+0xfc` at `0x1031cbdd..0x1031cc02`, `Rotation` from `actor+0x108` at
`0x1031cc05..0x1031cc22`, and the next three-vector, `OldLocation`, from
`actor+0x114` at `0x1031cc25..0x1031cc42`. `Engine.u` independently identifies
export 778 as the `StructProperty` named `OldLocation`.

None of the actual skeletal render stages reads actor offset `+0x114`:

- `URender::DrawActor` (`0x10b33980..0x10b34000`), which creates the dynamic
  sprite, has no `+0x114` operand;
- `DrawActorSprite` (`0x10b32850..0x10b33520`) has no `+0x114` operand; and
- `DrawMesh` through `DrawLodMesh` (`0x10b0e920..0x10b10280`) has no
  `+0x114` operand.

The `+0x114` loads in `USkeletalMesh::MeshAdjust`/`GetMeshCoords` are not actor
history accesses: at `0x1041ae27` the register is the mesh `this` pointer, and
at `0x1041afab`/`0x1041b099` it is the saved mesh `this` pointer in `ebp`.
Those loads are the already-proven mesh primitive-bound/origin inputs. The
actor argument is held in `eax`/`ebx`; final placement adds only its current
`Location` at `actor+0xfc..+0x104` at `0x1041b1cf..0x1041b1f5`.

Therefore retail does not interpolate or substitute `OldLocation` for this
skeletal draw. Using the map's historical Z in OpenHP1 would be an invented
placement rule, not original-engine behavior, and is rejected.

Reproduction commands:

```sh
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u find OldLocation
/tmp/rollprobe/target/debug/rollprobe res/System/Engine.u property 778
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1031cb95 \
  --stop-address=0x1031cc46 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10b33980 \
  --stop-address=0x10b34000 res/System/Render.dll | rg '\\+ 0x114\\]'
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10b32850 \
  --stop-address=0x10b33520 res/System/Render.dll | rg '\\+ 0x114\\]'
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x10b0e920 \
  --stop-address=0x10b10280 res/System/Render.dll | rg '\\+ 0x114\\]'
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x1041ae00 \
  --stop-address=0x1041b220 res/System/Engine.dll
```

The three Render.dll filters intentionally produce no output.

### 18. Retail video shows a stable visible start and only later path motion

The supplied recording is a CleanShot capture of YouTube playback, not a
direct game capture. Its SHA-256 is
`45cf982b4db06c68bb171e40fe92c9927716fefa55922f675c62a9f1fe23ce29`.
The H.264-only file is 1832x1080, contains 479 decoded frames over
19.166667 seconds, and has no audio stream. Although its nominal stream rate
is 60 fps, its average is `28740/1151` (about 24.97 fps) and its frame timing
is variable. The presentation timestamps below are therefore authoritative;
they were not calculated from the nominal rate.

The actor distinction is visible in the recording. Harry first traverses the
very large hollow log, which remains at screen right while the camera turns
through about 14.67 seconds. The smaller solid log across the grassy passage
becomes identifiable around 14.70 seconds and is clearly framed by about
14.90 seconds. This second object is the affected rolling log.

Frame-by-frame observations:

- during the relatively settled pre-cast interval from 15.266667 through
  15.950000 seconds, the target log's complete thickness is visible above the
  grass/floor and its placement is stable relative to the back and right
  walls;
- the first casting ring appears at 16.000000 seconds, and the strongest
  blue/orange contact effects span about 16.300000..16.600000 seconds. These
  particles obscure the bark and are unsuitable for precise edge tracking;
- after the effects clear, the log is still at its resting placement through
  17.433333 seconds. The first unambiguous translation is at 17.483333
  seconds: its left edge moves relative to the stationary circular log ends
  and back wall; and
- the subsequent response combines rolling/travel with a progressive rise.
  By about 17.666667 seconds the vertical separation from the stationary log
  ends and floor is visibly increasing, and it continues across multiple
  captured frames while horizontal travel is already under way. It is not an
  isolated one-frame placement correction at spell contact.

This strengthens both sides of the established distinction. Retail already
renders the log at its correct visible height before Flipendo, so the later
movement is not the missing initial alignment suddenly being applied. Retail
does then rise while moving toward the higher authored path, which is
qualitatively compatible with the traced `MoveTo`/blocked-walking/`stepUp`
route. The moving third-person camera, perspective, variable sampling,
YouTube recompression, and impact particles prevent a defensible conversion
of the image displacement into Unreal units or proof that any individual
step is exactly 25 units. The video exposes no live actor property and cannot
select one of the five remaining `MeshAdjust` gate inputs.

Reproduction commands:

```sh
ffprobe -v error -show_format -show_streams -of json \
  '/Users/splitty/Library/Application Support/CleanShot/media/media_K4OVPdRrBY/CleanShot 2026-08-13 at 20.39.18.mp4'
ffprobe -v error -select_streams v:0 \
  -show_entries frame=best_effort_timestamp_time -of csv=p=0 \
  '/Users/splitty/Library/Application Support/CleanShot/media/media_K4OVPdRrBY/CleanShot 2026-08-13 at 20.39.18.mp4' | \
  nl -v 0 -ba | sed -n '380,455p'
ffmpeg -hide_banner -loglevel error \
  -i '/Users/splitty/Library/Application Support/CleanShot/media/media_K4OVPdRrBY/CleanShot 2026-08-13 at 20.39.18.mp4' \
  -vf "select='between(n,433,439)',scale=600:-1:flags=lanczos,tile=7x1:padding=3:margin=3" \
  -frames:v 1 /tmp/rolllog-retail-433-439.png
```

### 19. The shipped corpus contains one controlled same-class contrast

Every export in all 41 shipped `res/Maps/*.unr` packages was decoded and its
resolved class chain checked, rather than filtering actor names. Exactly two
exports resolve to `HarryPotter.rolllog`, and both are active actors in
`Lev2_Fire2.unr`: affected `rolllog1` (export 2132) and `rolllog5` (export
2134). There is no differently named same-class instance in another map.

The two actors share the evidence-critical inputs: the same class, inherited
mesh/draw scale, no `Physics`, `bCollideWorld`, `bAlignBottom`, `PrePivot`,
mesh, draw-type, or animation override, and `CollideType=CT_Box`. Both state
frames have the same class/state imports, all-one probe mask, and null
instruction pointer. `rolllog5` serializes latent value `0x3f000000`, but
normal `GotoState` clears the old latent action before selecting the auto
state, as established in section 8. The placement-relevant differences are:

| Input | `rolllog1` | `rolllog5` |
| --- | --- | --- |
| Location | `(-2.5672,-1125.5870,6.8568)` | `(-2211.4138,-3704.5896,98.1577)` |
| Yaw | `-16384` | `+16384` |
| Collision height | `89.891426` | `80.0` |
| SizeModifier | inherited `1.0` | `0.8` (baseChar targeting/path size, not `DrawScale`) |
| First path | `HPath_A1`, Z `43.1883` | `HPath_A4`, Z `43.1883` |
| Destination | `baseStation5`, Z `43.2868` | `baseStation9`, Z `43.2868` |

Applying the already-proven retail `MeshAdjust` and `Stop` pose bounds gives:

```text
rolllog1 adjustment = -0.052073 - 89.891426 - 2.5 = -92.443499
rolllog1 world Z bounds ~= -85.5407 .. 4.3108

rolllog5 adjustment = -0.052073 - 80.0 - 2.5 = -82.552073
rolllog5 world Z bounds ~= 15.6516 .. 105.5031
```

Thus the same shared formula puts each mesh bottom approximately 2.5 units
below its collision bottom, as intended. The decisive authored contrast is
the collision box itself: `rolllog1` has bottom Z about `-83.0346` across the
local horizontal floor near Z zero, while `rolllog5` has bottom Z about
`18.1577`. The decoded BSP query agrees. At `rolllog1`, a vertical point trace
hits the horizontal floor and its box is already overlapped. At `rolllog5`, a
vertical point trace across 600 units finds no surface, while its box overlap
is a time-zero side-plane hit (node 2112, normal `(0,-1,0)`), not the same
floor contact.

This does not prove how `rolllog5` appeared in retail; no supplied video shows
it. It does provide the next controlled OpenHP1 experiment: observe/report
`rolllog5` before its spell and compare its visual bounds, collision, and
first movement with `rolllog1`. If `rolllog5` follows the predicted
`15.65..105.50` bounds while `rolllog1` remains buried, the shared skeletal
formula is behaving consistently and the unresolved semantic is specifically
how retail handles `rolllog1`'s initially floor-overlapping walking Pawn. If
both violate their predicted bounds, the investigation returns to the shared
render gate. Neither outcome authorizes a map-specific adjustment.

Reproduction commands:

```sh
find res/Maps -type f -iname '*.unr' \
  -exec /tmp/rollprobe/target/debug/rollprobe '{}' rolllogs ';'
/tmp/rollprobe/target/debug/rollprobe res/Maps/Lev2_Fire2.unr props 2132
/tmp/rollprobe/target/debug/rollprobe res/Maps/Lev2_Fire2.unr props 2134
/tmp/rollprobe/target/debug/rollprobe res/Maps/Lev2_Fire2.unr collisionat \
  -2.5671997 -1125.587 6.8567886 40 115 89.891426
/tmp/rollprobe/target/debug/rollprobe res/Maps/Lev2_Fire2.unr collisionat \
  -2211.4138 -3704.5896 98.157684 40 115 80
```

### 20. A visible direct-load result was a stale reverted-workaround artifact

On 2026-08-13, launching the existing debug executable directly with

```sh
target/debug/openhp1-game --level res/Maps/Lev2_Fire2.unr
```

appeared to contradict the active report: `rolllog1` was fully visible at its
initial location. No save was selected or implicitly restored; the associated
log recorded a fresh map load, `InitGame`, 2,344 startup events, and normal
player initialization. This briefly suggested a direct-load versus level-
travel difference.

That interpretation is rejected. Before it was rebuilt, the debug executable
was timestamped `2026-08-12 21:59:49 +0700`, while the current release
executable was built on 2026-08-13. The repository history places the
unproven `Place walking actors through BSP` change at `cde89e8` on 2026-08-12
22:29:14 and its revert at `f1a62c8` five minutes later. Although the debug
artifact predates the commit, it can represent the same working-tree code
before that code was committed. The old executable was overwritten before it
could be preserved: its initially observed SHA-256 was
`14a4768514020eddac8ea02f5a8c0f4af84b64a2a04798276f60caa5fcd7e2da`, while
the subsequent rebuilt debug executable has a different hash.

The decisive observation is behavioral: the user rebuilt the current debug
target and `rolllog1` disappeared again, matching the freshly built release
target. Therefore the visible screenshot was produced by the stale artifact
of the reverted BSP-placement workaround, not by current engine behavior, a
save, build-mode semantics, or direct map loading. It supplies no original-
engine evidence and does not reopen that workaround.

Reproduction commands:

```sh
stat -f '%N | modified=%Sm | size=%z' \
  -t '%Y-%m-%d %H:%M:%S %z' \
  target/debug/openhp1-game target/release/openhp1-game
shasum -a 256 target/debug/openhp1-game target/release/openhp1-game
git show -s --format='%H%n%ci%n%s' cde89e8 f1a62c8 b01b95f
git show cde89e8 -- crates/openhp1-runtime/src/world/physics.rs \
  crates/openhp1-runtime/src/world/save.rs
```

### 21. The retail recording's exact regional build is unproven

The supplied footage proves original PC-game behavior, but it does not prove
that the recorded installation used byte-identical packages and binaries to
the active OpenHP1 `res` tree. The local files are the North American
"Sorcerer's Stone" Version 1.0 build dated 2001-10-29. The mounted original
media at `/Volumes/HARRY_POTTER_EFG` is the European "Philosophers Stone"
Version 1.0 build dated 2001-10-22. Their relevant files have different hashes:

| File | active `res` SHA-256 | mounted European SHA-256 |
| --- | --- | --- |
| `Engine.dll` | `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc` | `9207af078045adbd672adfa54f91b177013b80afbd730c246a87c19e2ecf6d0e` |
| `Engine.u` | `b3661a1d2afb1f730ba5bb3cbd2a6716efaf55fd0d8cefa753b69026a8bc5a85` | `dd7f0890f10f7377d1adc0333001e697b83988e3f43803a3c96e75348ae37195` |
| `HPBase.u` | `0cec62e098ded3a16024ee15dbc982bf9662b443f630cd19890b7b5d325bf503` | `bf99ff125ab3062ad38fbfe122a37b385944e546ad9d262d91f8723f4ed323ad` |
| `HarryPotter.u` | `5f18066ac7d6a64ba315a19753308613c0819b3944da551a17bd0f710560cf60` | `8780db8743ed8cd957a0bc21eba05ceeee471181d4a4135ce754b967534a4c7f` |
| `HPModels.u` | `45ecb483a5b2a52e8f17c92326e21cde06eab2b501fb7d539052beb4408c9b65` | `5645656dd1a3675456e438307e20aa09d497737ee7612c5d253a38b03e1293cf` |
| `Lev2_Fire2.unr` | `8c3b03e160bb538e8e89fc40e5a2321cb03b235c2292b970f4c5e9ed34b3660f` | `8e92344e43d03d2243994f37b16394db8271e9489800d6eac5e24c9d3086e260` |

This does not yet explain the visual difference. The earlier binary and
package comparisons already found equivalent `MeshAdjust`, collision-size,
actor-default, and compiled-state behavior in both local builds. A fresh BSP
comparison also rejects a regional floor-height explanation: both maps put
the horizontal surface at approximately Z 0 under the identical authored
`rolllog1` location, and both report the same initial box overlap and
time-zero sweeps. The European map uses zero-based export 2126 instead of
2132, but its placement, rotation, collision dimensions, `CollideType`,
`OldLocation`, and state-frame values match the North American actor.

The direct class/asset comparison also finds no placement-relevant regional
difference. North American `HarryPotter.u` export 839 and European export 835
decode to the same `rolllog` class flags, state probe masks, default draw type,
zero collision height, `CT_Shape` class default, and imported `sklogMesh`.
Their `waitforspell` bodies have the same 42-byte execution stream after one
renumbered name reference, including the leading `SetPhysics(PHYS_Walking)`;
`atStation` and `justwaitforever` are likewise semantically identical after
package-reference renumbering. `HPModels.u` export 609 is at the same offset
and size in both packages and decodes to identical bounds
`(-44.720745,-127.06107,0.052073)..(45.118603,126.938896,89.891426)`, zero
origin and rotation, unit scale, and default animation export 496.

The precise build used by the YouTube recording cannot be recovered from the
CleanShot clip or public video metadata inspected here. Accordingly,
"same-files retail" is withdrawn as an evidentiary claim. The stronger and
still valid statement is that at least one original PC retail installation
renders this authored scenario correctly. A version-specific difference
remains possible only in an input not yet compared; it must be demonstrated,
not assumed.

A second public walkthrough is now inspected below. Its uploader describes it
as an original-disc July 2023 run on Windows 7, with only widescreen fixes and
dgVoodoo 2. It independently reproduces the visible pre-spell placement.

Reproduction commands:

```sh
shasum -a 256 \
  res/System/{Engine.dll,Engine.u,HPBase.u,HarryPotter.u,HPModels.u} \
  res/Maps/Lev2_Fire2.unr
shasum -a 256 \
  /Volumes/HARRY_POTTER_EFG/System/{Engine.dll,Engine.u,HPBase.u,HarryPotter.u,HPModels.u} \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr
/tmp/rollprobe/target/debug/rollprobe \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr rolllogs
/tmp/rollprobe/target/debug/rollprobe \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr collisionat \
  -2.5671997 -1125.587 6.8567886 40 115 89.891426
/tmp/rollprobe/target/debug/rollprobe \
  /Volumes/HARRY_POTTER_EFG/Maps/Lev2_Fire2.unr vertical \
  -2.5671997 -1125.587
/tmp/rollprobe/target/debug/rollprobe \
  res/System/HarryPotter.u 839
/tmp/rollprobe/target/debug/rollprobe \
  /Volumes/HARRY_POTTER_EFG/System/HarryPotter.u 835
/tmp/rollprobe/target/debug/rollprobe \
  res/System/HPModels.u 609
/tmp/rollprobe/target/debug/rollprobe \
  /Volumes/HARRY_POTTER_EFG/System/HPModels.u 609
```

### 22. A second original-disc recording independently shows the initial log

The independent YouTube upload [*Harry Potter and the
Philosopher's/Sorcerer's Stone (PC): Full Game Walkthrough, No Commentary,
60p*](https://www.youtube.com/watch?v=xNSrmkQhNTw), by Pvt. Philippe, contains
the same `Lev2_Fire2` passage at approximately `1:41:38..1:41:43`. YouTube's
oEmbed response identifies the title and uploader. The uploader's own video
description says that this is the PC **Sorcerer's Stone** KnowWonder game,
played from an original disc in July 2023 on Windows 7; the stated changes are
minimal 16:9 fixes and dgVoodoo 2, with the game otherwise in its original
form.

Frame inspection supplies a second, independent placement result:

- at `1:41:38.0`, while Harry is still walking through the preceding very
  large hollow log, the smaller target log is already fully visible across the
  opening ahead;
- it remains at that resting placement through at least `1:41:39.5`, before
  the casting reticle and spell animation appear;
- casting begins around `1:41:40.0`, impact effects are visible around
  `1:41:42.0`, and the log has clearly translated along its route by about
  `1:41:43.0`.

There is no pre-impact rise from below the floor in this recording. It therefore
independently confirms that the intended retail behavior is a visible resting
log followed by post-impact movement, rather than OpenHP1's buried resting
placement followed by recovery.

The description is also the strongest regional clue currently available: it
explicitly calls the played game *Sorcerer's Stone*, the North American title,
rather than only using the combined title in the upload name. It still does not
publish file hashes, executable version resources, or disc identifiers, so it
does **not** establish byte identity with the active North American `res` tree.
It does, however, make a Europe-only behavior difference substantially less
plausible; two independent retail observations now show the visible result.

The inspected temporary source clip is a 120.066667-second 640x360 H.264/AAC
segment beginning at video time `1:40:20`, SHA-256
`1195a0693d71a14cec28cb41ed015d02659eb5f1183afd4d59a2a3515cf72f43`.
The half-second contact sheet covering `1:41:38..1:41:43.5` has SHA-256
`617d4db11f6ad4a935540568cbd9a15ddc6b46218874b58cb7cc37a18c5b8fc6`.
Both are temporary inspection artifacts and are not repository inputs.

Reproduction commands:

```sh
curl -L --max-time 30 \
  'https://www.youtube.com/oembed?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3DxNSrmkQhNTw&format=json'
curl -L --max-time 30 -A 'Mozilla/5.0' \
  -o /tmp/xNSrmkQhNTw.html \
  'https://www.youtube.com/watch?v=xNSrmkQhNTw'

# The watch-page player metadata located the passage through YouTube's
# 10-second storyboard frames. A temporary itag-18 URL from YouTube's player
# response supplied the short inspection clip.
ffmpeg -hide_banner -loglevel warning -ss 6020 -i "$video_url" \
  -t 120 -c copy /tmp/xns-retail-video-6030/clip.mp4
ffmpeg -hide_banner -loglevel error -ss 78 \
  -i /tmp/xns-retail-video-6030/clip.mp4 -t 6 \
  -vf 'fps=2,scale=480:-1:flags=lanczos,tile=4x3:padding=2:margin=2' \
  -frames:v 1 /tmp/xns-retail-video-6030/contact-78-84-halfsec.jpg
shasum -a 256 /tmp/xns-retail-video-6030/{clip.mp4,contact-78-84-halfsec.jpg}
```

### 23. The shipped retail game can enter this map through its own level selector

The local retail packages provide a direct way to reproduce the scene in the
86Box installation without playing through the preceding chapters. While
controlling Harry, type `HarryDebugModeOn` (there is no text field or visible
typing prompt). `baseHarry` compares the accumulated typed string
case-insensitively with that exact token and calls `TurnDebugModeOn`; the
`HarryPotter` override sets `HPConsole(player.console).bDebugMode = true`.
This path does not depend on the separate `Version.bDebugEnabled` gate used by
the F7 debug toggle.

Opening the game's menu after that exposes its normally hidden **Level
Select** button. The compiled `FELevSelectPage` defaults contain the exact
entry **Forest Edge** -> `Lev2_fire2.unr`. Selecting it runs
`FEBook.RunURL(..., true)`. On the next menu tick, the shipped code assigns
`HPConsole.bInHubFlow = bTravelItemsOnLoad` and travels to the URL with item
travel enabled. The source comment explicitly says that Level Select is meant
to simulate normal in-hub flow during testing. This therefore gives a closer
retail comparison than launching `HP.exe Lev2_fire2.unr` or issuing a raw
`open` command, both of which bypass that menu-owned hub-flow setup.

Practical 86Box procedure:

1. Start or load any game far enough to control Harry.
2. Type `HarryDebugModeOn` on the keyboard; nothing is displayed while typing.
3. Press Escape, return to the main-menu page if necessary, and choose
   **Level Select**.
4. Choose **Forest Edge**. The requested map is `Lev2_fire2.unr`, not the
   adjacent **Fireseed Caves** entry (`Lev2_fire1.unr`).

This is a shipped UnrealScript/debug-menu facility and requires no executable,
map, INI, or save-file modification. It has not yet been exercised inside the
user's VM; the next primary observation is whether the untouched retail log is
visible immediately after this Level Select load and, ideally, a save made
before casting Flipendo.

Reproduction commands for the package audit:

```sh
strings -a -n 4 res/System/HPBase.u | \
  rg -n -A30 -B10 '^function TurnDebugModeOn|HarryDebugModeOn'
strings -a -n 4 res/System/HarryPotter.u | \
  rg -n -A25 -B8 '^function TurnDebugModeOn'
/tmp/rollprobe/target/debug/rollprobe res/System/HPMenu.u classprops 835
strings -a -n 4 res/System/HPMenu.u | \
  rg -n -A35 -B12 '_URLToLoad !=|bTravelItemsOnLoad|bInHubFlow = true'
```

### 24. Live local retail reproduction confirms correct pre-spell placement

The user ran the installed European retail game in the existing Windows XP
86Box VM, enabled the shipped debug mode, and selected **Forest Edge** through
the shipped Level Select described above. Before casting Flipendo, `rolllog1`
was fully visible at its expected resting placement. This is the first direct
observation from the local executable/package installation, rather than a
third-party video. It proves that the same local retail installation produces
the correct pre-spell result through its intended test-level hub-flow path.

The supplied 1920x1216 screenshot is SHA-256
`09560b0e69c9d10bb7141a0a8dd4043fdb58e099473e295f04dbf36c0c536126`.
It shows Harry standing before the fully visible log in 86Box; the user had not
cast Flipendo. The screenshot is conversational evidence and is not copied
into the repository.

The shipped Level Select selects save slot 9 before calling
`RunURL("Lev2_fire2.unr", true)`. `HPConsole.Tick` detects completion of the
travel on the first tick in the new level and calls `SaveSelectedSlot`, which
reaches `DoLevelSave(9)` and issues the native console command `SaveGame 9`.
This produced the automatic first-post-travel `save9.usa`. It proved useful as
an early endpoint, but was not by itself a settled pre-spell sample; the later
manual save below was required. The earlier advice that no manual save would
be needed was therefore incorrect.

### 25. The extracted automatic retail save preserves authored log placement

The Level Select run's automatic `Save9.usa` and `GameSaveInfo9` were copied
out of the Windows XP VM through a FAT12 floppy image. They remain local,
gitignored copyrighted artifacts under `.retail-save/` and are not repository
inputs. Their identities are:

- `Save9.usa`: 1,115,436 bytes, SHA-256
  `4bf3ac1e983b7f3c8674fe43242b9a22f9882b03edd06aadd44ee1a8ae62cc3b`;
- `GameSaveInfo9`: 28 bytes, SHA-256
  `58a9291b2afceeca95459ff023450d173135b4f692a8b5a3b0cb95df14c3971a`.

`Save9.usa` is a normal Unreal package, version 76, with 1,210 names, 203
imports, and 662 exports. Its `rolllog1` is zero-based export 255. The saved
actor has exactly the authored `Location = (-2.5671997, -1125.587,
6.8567886)`, `CollisionHeight = 89.891426`, `CollisionRadius = 40`,
`CollisionWidth = 115`, `CollideType = 2`, yaw `-16384`, and the same
serialized `OldLocation = (0, -1136, 23.6)`. `ColLocation` equals current
`Location`. There is no upward actor-location correction, `PrePivot`, scale,
mesh override, `Physics`, `bAlignBottom`, or `bCollideWorld` override in the
saved actor. This conclusively rejects a hidden retail actor-location lift as
the reason the log is visible.

The save's object stack adds a timing boundary. `rolllog1` is in imported
state `HarryPotter.rolllog.waitforspell`, has `bScriptInitialized=true`, no
latent action, and bytecode offset 0. Thus the automatic first-post-travel
save was written after state selection but before that state had executed its
body and reached the stable pre-spell wait. It is decisive for initial actor
placement and supplies the early endpoint for the settled comparison below.

`GameSaveInfo9` independently names `Lev2_fire2`. The extraction image was
mounted read-only; both copied files were byte-compared with their mounted
sources before the image was unmounted and detached.

Reproduction commands:

```sh
file .retail-save/Save9.usa .retail-save/GameSaveInfo9
shasum -a 256 .retail-save/Save9.usa .retail-save/GameSaveInfo9
xxd .retail-save/GameSaveInfo9
/tmp/rollprobe/target/debug/rollprobe .retail-save/Save9.usa find rolllog
/tmp/rollprobe/target/debug/rollprobe .retail-save/Save9.usa rolllogs
/tmp/rollprobe/target/debug/rollprobe .retail-save/Save9.usa import 124
```

### 26. Fresh Level Select startup grounds both logs before Flipendo

The user then entered **Forest Edge through Level Select again**, walked Harry
up to the untouched log, and issued `SaveGame 8` before casting Flipendo. The
log itself was never hit or moved. This was a fresh normal debug-menu level
load and did not load or otherwise rely on `Save9.usa`; elapsed normal startup
ticks, not save restoration, separate the early and settled observations.

The extracted `Save8.usa` is 1,133,777 bytes, Unreal package version 76, with
1,340 names, 214 imports, and 718 exports. Its SHA-256 is
`02cbd3acdad23daa4268e71373c32ea683bf90169c0db2f08db19c497478c2c9`.
The local copy was byte-compared with the floppy source before the read-only
image was unmounted.

In the settled save, `rolllog1` (zero-based export 258) has:

```text
Location = OldLocation = ColLocation
         = (-2.5671997, -1125.587, 92.491425)
Physics = 1 (PHYS_Walking)
CollisionHeight = 89.891426
Base = Level
AnimSequence = Stop
state = HarryPotter.rolllog.waitforspell
latent action = 384
bytecode offset = 21
```

Compared with the automatic early save's Z `6.8567886`, ordinary retail
startup raised the center by `85.6346364` units. The settled collision-box
bottom is `92.491425 - 89.891426 = 2.599999`, matching the floor-resting
clearance. This is the direct runtime fact the static trace was missing.

The same save supplies an internal same-class control. `rolllog5` settles from
early Z `98.157684` to Z `82.4989`; with `CollisionHeight=80`, its bottom is
`2.4989`. Retail therefore does not apply a special lift or fixed coordinate
to `rolllog1`: before Flipendo, ordinary startup grounds both `rolllog`
actors in the appropriate direction until their collision bottoms rest at
about Z 2.5--2.6.

This withdraws the prior conclusion that idle startup cannot relocate this
actor upward. The disassembled ordinary downward `MoveActor` request was
bounded correctly, but it was not the complete startup-grounding mechanism.
The live save pair proves such a shared retail mechanism exists and fixes its
exact output. It does not by itself authorize restoring the earlier generic
spawn-placement workaround; the responsible native query/call seam must still
be identified first.

Reproduction commands:

```sh
file .retail-save/Save8.usa
shasum -a 256 .retail-save/Save8.usa
/tmp/rollprobe/target/debug/rollprobe .retail-save/Save8.usa find rolllog
/tmp/rollprobe/target/debug/rollprobe .retail-save/Save8.usa rolllogs
```

### 27. Root cause: the omitted idle-walking floor-clearance branch

The save pair made it possible to revisit `APawn::physWalking` with the actual
retail direction and final displacement known. The earlier reading of the
floor-maintenance tail was incomplete. It correctly identified the downward
floor sweep and bounded `MoveActor`, but stopped before a second branch at
`0x103e9b27..0x103e9bc8` that can issue an upward request even while the Pawn
has no horizontal movement.

After `SingleLineCheck` returns the floor hit, retail computes

```text
floor_distance = (MaxStepHeight + 2.0) * Hit.Time
```

For `rolllog1`, `MaxStepHeight=25`, so the sweep is 27 units rather than
OpenHP1's previous `25 * 1.3 = 32.5`. The subsequent branch is conditional on
the hit actor still matching `Base`:

```text
if floor_distance > 2.4 or hit actor != Base:
    MoveActor(down 27)
else if floor_distance < 1.9:
    MoveActor(up (2.1 - floor_distance))
else:
    do not move
```

The constants are byte-proven in the local shipped `Engine.dll`: float `2.0`
at VA `0x104770ec`, float `2.1` at `0x1047a5a0`, double `1.9` at
`0x1047a5a8`, and float `2.4` at `0x1047a5b4`. At
`0x103e9b41..0x103e9b5c`, the binary calculates `2.1-floor_distance`; at
`0x103e9b84..0x103e9bc8`, it constructs `(0,0,+result)` and calls the virtual
`ULevel::MoveActor` slot `+0x8c`. The `Hit.Time=0` overlap observed at the
authored log location therefore requests `+2.1 Z` on each idle walking tick.
It repeats until the box clears BSP and enters the native 1.9--2.4 clearance
band. This is the missing progressive startup lift.

The required Base precondition is also native, not inferred from the save.
When the compiled auto-state calls `SetPhysics(PHYS_Walking)`, retail
`AActor::setPhysics` at `0x103e5178..0x103e51c3` writes the new mode and calls
`FindBase`. `FindBase` at `0x103e4ff5..0x103e5091` performs a 10-unit downward
`SingleLineCheck` with the actor extent, then `SetBase`s the hit. That is why
Save8 has `Base=Level` before the idle branch runs. OpenHP1 previously emitted
the physics action without reproducing this `FindBase` side effect.

This mechanism predicts the two retail endpoints without any map or actor
exception:

- deeply overlapped `rolllog1` repeatedly takes the upward `2.1-distance`
  branch;
- initially high `rolllog5` takes the ordinary downward branch;
- both stop with their collision bottoms in the same narrow floor-clearance
  region measured in Save8.

OpenHP1's corresponding `walk_to_floor` previously performed only an ordinary
padded `MoveActor` query and then always tried the full downward delta. The
shared fix now reproduces the retail `MaxStepHeight+2`, unpadded
`SingleLineCheck`, same-Base clearance band, and `FindBase` transition when
entering walking. It does not call `FindSpot`, alter a map actor, or recognize
the log/map/coordinates.

A one-second local-corpus runtime scan after the change settles `rolllog1` at
Z `92.95675`, versus retail Save8 Z `92.491425`; both lie in the native
clearance band and render the complete mesh above the floor. The remaining
0.465-unit difference is collision-query precision/margin, not the previous
85.6-unit missing relocation, and is deliberately not tuned with an
actor-specific offset. The clean scan completes with no destroyed actors or
runtime failure.

Reproduction commands:

```sh
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103e9981 \
  --stop-address=0x103e9c40 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump --disassemble \
  --x86-asm-syntax=intel --start-address=0x103e4fd0 \
  --stop-address=0x103e5220 res/System/Engine.dll
python3 - <<'PY'
import struct
data = open('res/System/Engine.dll', 'rb').read()
for va, kind in [(0x104770ec, 'f'), (0x1047a5a0, 'f'),
                 (0x1047a5a8, 'd'), (0x1047a5b4, 'f')]:
    offset = 0x16d000 + va - 0x1046d000
    print(hex(va), struct.unpack('<' + kind, data[offset:offset + struct.calcsize(kind)])[0])
PY
cargo nextest run -p openhp1-runtime
cargo run -q -p openhp1-scene --example runtime_scan -- \
  res/Maps/Lev2_Fire2.unr 1
```

## Current conclusion

The affected package object is conclusively identified as zero-based export
2132, `rolllog1` (`rolllog`), in shipped `Lev2_Fire2.unr`. Its full authored
route and compiled script transition are now established. The pre-spell auto
state requests walking and `Stop`; after Flipendo the patrol state requests
walking again, switches to `Roll`, and begins native `MoveTo` toward an
authored path whose Z is about 36.3 units above the actor's initial center.
The first possible post-travel draw can precede the new level's first tick and
use Physics none, but normal ticking changes it to walking after at most one
parity-skipped tick. The separate-looking Flipendo target is a runtime effect
derived from and trailed to the same actor.

Idle walking is the missing pre-spell mechanism. When the floor hit matches
Base and measures below 1.9 units, retail requests an upward correction toward
2.1 units. A time-zero overlapping hit therefore raises `rolllog1` by 2.1
units per tick until its collision box clears the floor. Flipendo's later
horizontal `MoveTo`/`stepUp` path is separate and is no longer needed to
explain the initial placement.

The earlier conclusion that "the requested visible pre-spell placement is not
produced by the original engine" is **withdrawn**. Two independent original PC
retail recordings prove the opposite, including one uploader-described
original-disc *Sorcerer's Stone* run. Neither recording publishes sufficient
disc or file identity to establish byte equality with a local Version 1.0
installation. The static trace is therefore incomplete even though its
individual package values, addresses, gates, and coordinate arithmetic remain
reproducible in the two local builds.

The earlier render, cache, proxy/owner, configuration, `OldLocation`, root
motion, and `FindSpot` hypotheses remain rejected. They were useful for
isolating the issue, but the fresh retail save pair now closes the previous
static-versus-live contradiction: current actor `Location`, not a hidden draw
transform, changes during normal idle walking. The decisive omission was in
the already-identified native movement function itself.

The implementation is shared UE1 walking physics. It recognizes no map,
actor, asset, coordinate, or spell. `rolllog5` provides the same-class
opposite-direction control, while Save8 supplies the final retail state for
both actors. The stale debug executable remains excluded because it used the
different reverted `FindSpot` workaround, not this proven floor-clearance
path.

## Disposition

The root cause is fixed in shared walking physics and covered by a focused
floor-clearance-band regression test. The following alternative changes
remain prohibited by evidence:

- changing this map actor's location, collision, `PrePivot`, physics, or
  `bAlignBottom` is an actor/map workaround;
- finding a spawn spot on `SetPhysics` contradicts the shipped native call
  graph; the implemented transition only reproduces retail `FindBase`;
- suppressing coordinate recomputation merely to retain a stale transform is
  contradicted by retail `ApplyAnim`;
- caching the log through `bAssimilated`, or invalidating that flag on physics
  or skeletal movement, contradicts its shipped moving-`ABrush` ownership and
  absence from Render.dll;
- reversing or conditionally bypassing the negative mesh adjustment
  contradicts `USkeletalMesh::GetMeshCoords` and its final vertex composition.

The video still prohibits using the first post-spell rise as evidence for an
initial-placement correction: retail begins fully visible, and its later rise
is progressive and concurrent with travel rather than a contact-time pop.
The stale debug screenshot likewise does not authorize restoring the reverted
walking-actor placement code; that implementation used `FindSpot`, whereas the
retail-supported fix is the `FindBase` plus idle `physWalking` clearance band
documented above.

The original-engine evidence now authorizes this implementation. No renderer,
map data, save data, or actor-specific adjustment is changed.
