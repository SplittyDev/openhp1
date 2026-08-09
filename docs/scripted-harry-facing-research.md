# Scripted Harry facing: original-game evidence

This note scopes the remaining visual problem to on-foot scripted movement. It
uses the shipped packages as the authority; the local SurrealEngine comparison
is deliberately last.

## Finding

An authored cutscene destination's `Rotation` is not Harry's facing source.
During `MOVETO`, `baseHarry.CutMovingTo.PlayerTick` recomputes a horizontal
heading from `CutWalkDest.Location - Location`, writes that heading to
`DesiredRotation`, and moves with `MoveSmooth`. A later `FACE` snaps both
`DesiredRotation` and `Rotation`; a later `TURNTO` performs a latent turn.

Two shared runtime gaps produced the longer visible stall in the supplied
`CutScene60` sequence. OpenHP1's `MoveSmooth` stopped after one wall slide,
while the shipped native performs `TwoWallAdjust` and a third movement attempt
when that slide also hits. OpenHP1 also swept an aligned pawn cylinder against
world BSP as a rounded cylinder, while the shipped native passes
`(CollisionRadius, CollisionRadius, CollisionHeight)` through the model's box
extent trace. Matching both native paths lets Harry cross the authored bench
route instead of timing out and teleporting.

A separate facing gap is broader than `CutMovingTo`: shipped `Engine.u`
explicitly says both latent `MoveTo` and `MoveToward` rotate the pawn toward
their destination, but the runtime's existing `PlayerPawn` rotation gate only
yields to `CutMovingTo` or matching latent `TurnTo`/`TurnToward`. Harry has
authored latent `MoveTo` and `MoveToward` paths outside `CutMovingTo`. The
source-backed facing fix is to make that gate also yield for a matching latent
`MoveTo` or `MoveToward`.

## Original cutscene ownership chain

- `HPBase.u`, `CutScene` class export 265, `ScriptText` export 3476 at serialized
  offset `0x2c9f96`, and compiled `handleCast` export 3563 at `0x30c7d3`:
  `MOVETO` dispatches `baseHarry.CutMoveTo`; `FACE` calculates the actor-to-target
  heading and calls both `SetRotation` and a `DesiredRotation` assignment;
  `TURNTO` dispatches `baseHarry.CutTurnTo`.
- `HPBase.u`, `baseHarry` class export 0 and `ScriptText` export 3547 at
  `0x2e285f`: `CutMoveTo` calls `SetupMoveTo` and enters `CutMovingTo`.
  Compiled exports 2815 and 2814 confirm those calls. `CutMovingTo` is compiled
  state export 2807; its active `PlayerTick` is export 2806. That tick flattens
  the destination to Harry's current Z, computes `normal(dest-curLoc)`, assigns
  `rotator(heading)` with pitch zero to `DesiredRotation`, and advances at
  `GroundSpeed` with `MoveSmooth`. Its state body selects `PHYS_Walking` and the
  `run` animation.
- `HPBase.u`, compiled `CutTurningTo` state export 2792 and `CutTurnTo` export
  2794: explicit authored turns select `PHYS_Rotating` and use latent `TurnTo`.
- `HPBase.u`, `CutRelease` export 2850 copies `Rotation` to both `ViewRotation`
  and `DesiredRotation` before returning to `PlayerWalking`.
  `CutMovingTo` itself never updates `ViewRotation`; consequently, its stale
  value during capture is authored and must not be used as Harry's body
  orientation.

The source text also contains an older latent-`MoveTo` implementation after the
active `CutMovingTo` path. Its comments mention a facing bug and a forced final
heading, but compiled `PlayerTick` export 2806 proves that the shipped active
movement is the manual `MoveSmooth` implementation above. Source comments alone
must not select the compatibility behavior.

### `CutWalkDest.Location` evaluation

There is no compiled-property split between the normal and timeout reads.
Within `PlayerTick` export 2806, the assignment
`dest = CutWalkDest.Location` is the `Context` expression at decoded bytecode
offsets `0x001c..0x0029`; the timeout's `SetLocation(CutWalkDest.Location)` uses
the second `Context` at `0x0138..0x0145`. Both encode `baseHarry.CutWalkDest`
export 77 and `Engine.Actor.Location` import 7. `CutMoveTo` export 2815 assigns
that same `CutWalkDest` before calling `SetupMoveTo`; `SetupMoveTo` export 2814
assigns it again and immediately reads its `Location` to calculate the timeout.

The current VM does not keep `dest` as a late-bound property reference. Its
`Context` handler first resolves the outer object, captures that object handle
in the inner instance-variable slot, and restores the parent context
(`crates/openhp1-runtime/src/frame/execute.rs`, `expression_inner`). The
enclosing `Let` then reads the slot and copies the resulting `Value::Vector`
into a local. Script functions start with a new `Frame`, and local zero values
are rebound for each call (`crates/openhp1-runtime/src/world/execution.rs`,
`execute_function`; `crates/openhp1-runtime/src/world/instance.rs`,
`bind_frame_zero_values`). On the inspected path, neither a retained local nor
a receiver chosen after context restoration can explain an old destination.

### Capture does not change Harry's collision

The shipped cutscene transitions do not disable or resize Harry's collision.
In `HPBase.u`, compiled `CutScene.handleCast` export 3563 implements `CAPTURE`
for a `baseHarry` by changing the HUD cutscene fields and calling
`playerHarry.CutDoIdle`. `CutDoIdle` export 2849 only enters `CutIdleing` state
export 2848. That state selects `PHYS_Rotating`, moves downward to the floor,
plays the idle animation, and uses `MoveTo(self.Location)`; it does not call
`SetCollision` or `SetCollisionSize`, or assign `bCollideWorld`.

The following `MOVETO` path is likewise collision-neutral: `CutMoveTo` export
2815 calls `SetupMoveTo` export 2814 and enters `CutMovingTo` export 2807,
whose `PlayerTick` is export 2806. None of those compiled exports calls
`Engine.Actor.SetCollision` native 262 or `SetCollisionSize` native 283, and
their shipped source has no collision flag or size assignment. `CutRelease`
export 2850 only synchronizes `ViewRotation`/`DesiredRotation` with `Rotation`
and returns to `PlayerWalking`.

A package-backed runtime read found the same effective Harry values before
capture and during `CutMovingTo`: `bCollideWorld=true`,
`CollisionRadius=15`, and `CollisionHeight=42`. This rules out a cutscene
collision-mode transition as the retail/OpenHP1 difference; it does not claim
that every Harry state preserves those values.

## Other shipped on-foot movement owners

| Path | Shipped behavior | Facing owner |
| --- | --- | --- |
| `Harry.PlayerWalking` | Input-driven walking/running; compiled state export 489, `PlayerTick` 498, `PlayerMove` 503 | `Harry.UpdateRotation` export 554 calls `SetRotation` from `ViewRotation`; this is player-input ownership. |
| `baseHarry.CutMovingTo` | Cutscene run using per-tick `MoveSmooth`; exports 2807/2806 | Per-tick `DesiredRotation` from the destination vector. |
| `Harry.ChessMode` | Authored `PlayWalking(); MoveTo(ChessTargetLocation)`; state export 975 | Latent `MoveTo` owns destination rotation according to `Engine.u`; ordinary player rotation can also run from this state's `PlayerTick`, so this path needs an exact replay before changing precedence. |
| `Harry.waitForDeath` | `MoveToward(bustedBy)` with `run`, or `MoveTo(Location)` when close; state export 789 | Latent movement owns destination rotation according to `Engine.u`. |
| `Harry.FallingMount` / `Harry.Mounting` | Turn toward the mount, then animation/root movement; state exports 740 and 774 | Latent `TurnTo`; these are scripted traversal but not running. |
| `baseHarry.lookatActor` / `wingspell` | Stationary repeated turns; state exports 2864 and 2760 | Latent `TurnTo`. |
| `baseHarry.CutIdleing` / `stateDead` | `MoveTo(self.Location)` is only a stop/grounding operation; state exports 2848 and 2907 | No travel heading. |

`Engine.u`, `Pawn` `ScriptText` export 4863 at `0xbbe8c`, declares the latent
movement functions and states that `MoveTo` sets `Destination`, `MoveToward`
sets `MoveTarget`, and the actor rotates toward the destination. The underlying
rotation contract is defined on `Actor`: `DesiredRotation` export 199,
`RotationRate` export 61, `bRotateToDesired` export 1157, and
`bFixedRotationDir` export 1155. `Actor` `ScriptText` export 5401 at `0xd98a9`
describes `DesiredRotation` as the pawn rotation target.

## Offline Lev_Tut1 check

A non-interactive 60 Hz replay of the original `Lev_Tut1.unr` startup sequence
resolved Harry as map export 602 and its first three destinations as
`CutMark90` export 1545, `CutMark91` export 821, and `CutMark0` export 842.
After the existing `CutMovingTo` change:

- the first leg reached actor/desired yaw `18021`;
- the second changed desired yaw to `31863` and actor yaw caught up in about
  0.2 seconds;
- the third reported desired yaw `-29930` and actor yaw `35606`, which are the
  same UE1 angle modulo 65536.

`ViewRotation` remained at its pre-cutscene yaw throughout, matching the
`CutRelease` contract above. This replay does not reproduce a persistent facing
error on those three legs, and a signed-versus-unsigned rotator display alone is
not evidence of a visual error.

A broader non-interactive corpus audit discovered 41 original `.unr` maps and
simulated each map's first 20 seconds. Only maps that entered `CutMovingTo`
emitted facing samples. Across those samples there was no sustained mismatch;
the longest continuous interval with actor yaw more than 45 degrees from
`DesiredRotation` was 0.18 seconds.

The same 41-map, 20-second headless scan completed after broadening the latent
movement gate, including `Lev5_Chess` and `Lev_Tut1`.

### `CutScene60` trace correction

An early diagnostic forced `Lev_Tut1.CutScene60` while the startup cutscene was
still moving Harry toward `CutMark90` export 1545. Comparing that in-flight
`PlayerTick` local with `CutWalkDest` after the forced command crossed two
authored command phases; it did **not** demonstrate a stale VM receiver or
value. OpenHP1 also returns event side effects as `ActorAction`s and drains
queued `DispatchEvent` actions afterward
(`crates/openhp1-scene/src/runtime.rs`, `apply_runtime_actions_with`), so a
property sampled after that drain is not a same-expression snapshot of a
vector logged inside the preceding `PlayerTick`.

A delayed probe also cannot begin Harry's movement by merely touching
`CutScene60`. The actor is map export 2139 at serialized offset `0x771e2`, and
its shipped cast data orders `WAITFOR HarryInFront` before `MOVETO HPpath1` and
`MOVETO HPpath2`. Until Quirrell issues `CUE HarryInFront`, Harry remains in
`CutIdleing`; seeing its retained prior `CutWalkDest` (`CutMark0` export 842) is
therefore expected. The active `CutMovingTo` state never clears `CutWalkDest`
when it enters `CutIdleing`.

An isolation that started after the startup cast and then delivered
`HarryInFront` resolved `HPpath1` to map export 942 at
`(1527.6072,-6602.846)` and `HPpath2` to export 768 at
`(1443.412,-6723.6416)`. The corresponding `PlayerTick` vector operations read
those current locations in order. This invalidates the stale-receiver premise;
the remaining failure was a stall short of `HPpath1`, not a wrong
`CutWalkDest.Location` read.

### `HPpath1` is a serialized actor reference, not an actor search

`HPBase.u` defines `CutScene.CutLoc` as an editable `actor locName` plus a
string `alias`; `CutScene` owns a fixed `Locs[40]` array. The shipped
`lookupTarget` source in `CutScene` `ScriptText` export 3476 at `0x2c9f96`
uppercases the command argument, scans `Locs` from index 0 through 39, and
returns `locs[i].locName` directly when the alias matches. Compiled
`lookupTarget` export 3541 at `0x2de504` confirms that active order: the
`locName != None` test is at decoded bytecode `0x0081..0x0096`, the
case-insensitive alias comparison is at `0x0097..0x00b2`, and the return reads
the same array element's `locName` at `0x00b3..0x00c4`. It does not enumerate
actors or compare an actor name, `Tag`, class, or export order for a location
alias. `handleCast` then passes the returned actor directly to
`baseHarry.CutMoveTo`.

The original `Lev_Tut1.unr` data makes this unambiguous. `CutScene60` export
2139 at `0x771e2` serializes `Locs[7]` at payload offset `0x772b7` as the compact
object reference to export 942 followed by the string `HPpath1`. That export is
`CutMark44`, class `CutMark`, at `0x2738d`; its serialized location is
`(1527.6072,-6602.846,821.57446)`. `Locs[0]` similarly stores alias `HPpath2`
and a direct reference to `CutMark45` export 768. Of the 17 populated
`CutScene60.Locs` entries, only index 7 has alias `HPpath1`. Some distinct
aliases deliberately share a target (`HPpath2` and `LocName8` both refer to
export 768), but there is no competing `HPpath1` entry.

The map contains 110 `CutMark` actors, and export 942 has the ordinary `Tag`
value `CutMark`, but neither fact participates in this lookup. The shipped
`CutMark` source is only `class CutMark expands NavigationPoint;`
(`HPBase.u` `ScriptText` export 3478 at `0x2cf610`). Consequently, retail cannot
select a different mark for this command merely because actors or exports are
visited in a different order: the authored reference is export 942. A target
difference would instead require mis-decoding that serialized object reference
or mutating `Locs[7]`; neither occurred in the isolated trace.

This conclusion is limited to the two compiled property reads and the current
VM evaluation path above. It does not prove the retail player tick/event order;
the binary inspection below is limited to the relevant `moveSmooth`,
`TwoWallAdjust`, and `HitWall` dispatch paths.

## Original native `MoveSmooth` collision contract

`Engine.u`, `Actor` `ScriptText` export 5401 at `0xd98a9`, declares
`MoveSmooth(vector Delta)` as native 3969, so its shipped implementation is in
`Engine.dll`, not UnrealScript. The original PE has image base `0x10300000` and
retains the following decorated exports:

- `?moveSmooth@AActor@@QAEHVFVector@@@Z`, RVA `0x13d4`: its thunk at
  `0x103013d4` jumps to the body at `0x103e4c30`;
- `?TwoWallAdjust@AActor@@QAEXAAVFVector@@000M@Z`, RVA `0x1d2f`: its thunk at
  `0x10301d2f` jumps to the body at `0x1031c3e0`;
- `?MoveActor@ULevel@@UAEHPAVAActor@@VFVector@@VFRotator@@AAUFCheckResult@@HHHH@Z`,
  RVA `0x404d`.

The `moveSmooth` body performs an initial virtual `MoveActor` call at
`0x103e4cd7`. After a partial hit it projects the untraveled displacement onto
the first wall and performs a second `MoveActor` call at `0x103e4e46`. If that
slide also hits, the branch calls `AActor::TwoWallAdjust` at `0x103e4e72`, then
performs a third `MoveActor` call with the adjusted displacement at
`0x103e4eba`.

This behavior is necessary but not sufficient for the bench route. With only
the third attempt restored, the isolated path still stopped at the low BSP
ramp. Changing Harry's collision flags or cylinder size remains unsupported by
the shipped script and defaults.

### World BSP uses a box extent, including for cylinders

The second difference is below `MoveSmooth`, in the extent passed through
`MoveActor` to world-model collision. The shipped `Engine.dll` exports
`AActor::GetCylinderExtent`; its thunk at `0x103028d3` reaches the body at
`0x1037a900`. For an ordinary aligned cylinder such as Harry
(`CollideType=0`, radius `15`, height `42`), that body returns the vector
`(CollisionRadius, CollisionRadius, CollisionHeight)`.

The original `UModel::LineCheck` body at `0x10429c80` sends a non-zero extent
to `FBoxLineCheck` at `0x1042a480`. That path sweeps an axis-aligned box through
the BSP hull planes and bevel planes; it does not replace the horizontal
extent with a rounded cylinder. The local licensed SurrealEngine independently
uses the same model-level `TraceAABBModel` path, but the original binary is the
authority for this choice.

OpenHP1 already had the required BSP AABB sweep and already used it for
`CT_Box`; the incorrect branch was the default aligned-cylinder fallback in
world movement. Routing non-brush actors through the existing AABB sweep fixes
that shared seam. Actor-versus-actor collision and spawn placement remain on
their existing shape-specific paths.

### Isolated `Lev_Tut1` result

The supplied sequence resolves `HPpath1` to `CutMark44` at
`(1527.6072,-6602.846,821.57446)` and then `HPpath2` to `CutMark45` at
`(1443.412,-6723.6416,805)`. The blocking world geometry includes a 16-unit
step and adjoining ramp represented by BSP hull 2254. The compiled
`CutMovingTo.PlayerTick` supplies its own 15-unit up/across/down probes, so the
native collision and two-wall behavior determine whether that route advances.

In a non-interactive 60 Hz replay, the rounded-cylinder implementation stayed
at approximately `(1440.0005,-6639.1504,795)` until the authored timeout. With
the original box-extent sweep and third `MoveActor` attempt, Harry reached the
first target region at `(1521.4314,-6624.2813,827)` and continued to the second
target without the timeout relocation. Retail PC footage also shows Harry
crossing this ramp continuously. This replay validates simulation state and
movement; final rendered confirmation in OpenHP1 remains a separate visual
check.

### `HitWall` is dispatched, but is inert in `CutMovingTo`

The original `moveSmooth` also dispatches the script `HitWall` event after the
first-wall projection and before the second `MoveActor`. The PE exports
`?eventHitWall@AActor@@QAEXVFVector@@PAV1@@Z` at RVA `0x2707`; its thunk jumps
to `0x1031b350`, which loads the retained `ENGINE_HitWall` `FName`, calls
`UObject::FindFunctionChecked`, and invokes the actor's virtual `ProcessEvent`.
The same sequence is inlined in `moveSmooth`: it loads `ENGINE_HitWall` at
`0x103e4dd2`, calls `FindFunctionChecked` through the Core import slot at
`0x103e4de7`, and calls virtual `ProcessEvent` at `0x103e4df8`. The hit normal
and hit actor are the event parameters. This is script-event dispatch; the
separately exported native `?processHitWall@AActor@@...` at RVA `0x4d4a` is not
the direct call made at this point in `moveSmooth`.

That event cannot alter Harry's motion on the inspected cutscene leg. The
shipped `CutMovingTo` source defines only `PlayerTick`, `AnimEnd`, `AltFire`,
and `Fire`; compiled state export 2807 has the same four child functions
(exports 2806, 2805, 2804, and 2796). Neither `Harry` nor `baseHarry` has a
class-level `HitWall` override, and neither does `Engine.PlayerPawn` or
`Engine.Pawn`. `Harry.PlayerWalking` does have a state-local compiled `HitWall`
export 831, but `GotoState('CutMovingTo')` makes that other state's function
inactive. Resolution therefore falls through to `Engine.Actor.HitWall` export
2273 at `0x24642`, whose compiled body is only `Return; Nothing` (two bytes).
Omitting the event is a general native-semantic gap, but it does not adjust
acceleration, velocity, destination, or location in this `CutMovingTo` state
and does not explain this stall.

## SurrealEngine comparison and uncertainty

The local SurrealEngine calls virtual `TickRotating` after every non-`PHYS_None`
physics step. Its `UPawn::TickRotating` consumes `DesiredRotation`, but its
`UPlayerPawn::TickRotating` is effectively a no-op. That cannot satisfy the
shipped HP scripts above without another HP-specific path, so it is useful for
the general tick order but not authoritative for Harry's rotation ownership.

Its actor hierarchy also provides one concrete ordering reference:
`UPlayerPawn::Tick` calls `UPawn::Tick` first; `UPawn::Tick` reaches
`UActor::Tick`, which advances ordinary `Tick`, state code, and then physics;
only after that returns does `UPlayerPawn::Tick` dispatch `PlayerInput` and
`PlayerTick` (`SurrealEngine/UObject/UActor.cpp`). The shipped HPBase code is
compatible with that order: `CutMovingTo.PlayerTick` performs its own
`MoveSmooth` after the frame's physics. This comparison identifies a shared
engine seam to test, but remains licensed-reference evidence rather than proof
of the retail HP executable's native order.

SurrealEngine's `UActor::TryMoveSmooth` also stops after the first projected
`TryMove` and even marks its return with `// XXX: does this break anything?`
(`SurrealEngine/UObject/UActor.cpp`). It omits the original binary's
`TwoWallAdjust` and third move, so it is not authoritative for this collision
corner.

Only the relevant original `Engine.dll` smooth-movement and `HitWall` dispatch
paths were disassembled for this note. The package evidence establishes who
writes each rotation value, but not whether the retail executable gives HP's
`PlayerPawn` a native rotation or tick special case.
The source-backed runtime changes therefore stay at two existing seams:
matching latent movement passes through the `PlayerPawn` rotation gate, while
`MoveSmooth` and world BSP movement use the original third-attempt and box-
extent contracts. The latent-movement regression verifies the rotation gate,
and the world-collision regression distinguishes the native box extent from
the rounded cylinder that caused the ramp stall. An authored `ChessMode`
replay remains useful because its own `PlayerTick` may intentionally take
precedence.
