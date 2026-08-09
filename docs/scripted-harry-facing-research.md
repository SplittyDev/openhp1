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

Two native-semantic gaps were found along the way. OpenHP1's `MoveSmooth`
stopped after one wall slide, while the shipped native performs
`TwoWallAdjust` and a third movement attempt when that slide also hits. OpenHP1
also swept an aligned pawn cylinder against world BSP as a rounded cylinder,
while the shipped native passes Harry's axis-aligned collision bounds through
the model's box-extent trace. Both are real shared gaps, but neither explains
the remaining `CutScene59` trajectory by itself.

One conspicuous displacement occurs when `CutMovingTo` reaches its second
authored mark and immediately enters `CutIdleing`. That state's compiled Begin
executes `MoveSmooth(vect(0,0,-100))`. At the observed center, the shipped BSP
walker reaches the original map's invisible semisolid classroom ramp, and the
shipped native projection produces the same approximately `(0,-40,-20)` slide
as OpenHP1. Suppressing that slide would contradict the original binary.

The live `CutScene59` replay invalidated the zero-momentum headless replay as
an acceptance test: Harry passes the first aisle waypoint and runs toward the
wall before collision can explain the divergence. The headless setup had
started Harry with zero `Velocity` and `Acceleration`, while the authored touch
path begins as Harry runs into the cutscene trigger.

The missing transition is in native `SetPhysics`. `CutIdleing` selects
`PHYS_Rotating` before the first scripted leg. The shipped native clears both
motion vectors for that physics mode, but OpenHP1 previously changed only the
`Physics` byte. Its walking physics therefore added Harry's incoming player-run
momentum after `CutMovingTo.PlayerTick` had already called `MoveSmooth` toward
the authored mark. That extra displacement carries him past the waypoint and
leaves the cutscene timeout to teleport him back. Restoring the native reset is
the first source-backed fix that addresses the pre-collision divergence.

A separate facing gap is broader than `CutMovingTo`: shipped `Engine.u`
explicitly says both latent `MoveTo` and `MoveToward` rotate the pawn toward
their destination, but the runtime's existing `PlayerPawn` rotation gate only
yields to `CutMovingTo` or matching latent `TurnTo`/`TurnToward`. Harry has
authored latent `MoveTo` and `MoveToward` paths outside `CutMovingTo`. The
shipped native confirms that their initial calls and subsequent polls write the
receiving pawn's `DesiredRotation` after player input and before pawn rotation
physics. Making that gate also yield for a matching latent `MoveTo` or
`MoveToward` is therefore faithful while the latent remains active. It is not
fully exact on the poll that completes movement; that limit is detailed below.

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
| `Harry.ChessMode` | Authored `PlayWalking(); MoveTo(ChessTargetLocation)`; state export 975 | Latent `MoveTo` owns destination rotation. The shipped actor tick runs this state's `PlayerTick` first, then the latent poll, so the latent heading is the final `DesiredRotation` consumed by physics. |
| `Harry.waitForDeath` | `MoveToward(bustedBy)` with `run`, or `MoveTo(Location)` when close; state export 789 | Latent movement owns destination rotation according to `Engine.u`. |
| `Harry.FallingMount` / `Harry.Mounting` | Turn toward the mount, then animation/root movement; state exports 740 and 774 | Latent `TurnTo`; these are scripted traversal but not running. |
| `baseHarry.lookatActor` / `wingspell` | Stationary repeated turns; state exports 2864 and 2760 | Latent `TurnTo`. |
| `baseHarry.CutIdleing` / `stateDead` | `MoveTo(self.Location)` is only a stop/grounding operation; state exports 2848 and 2907 | No travel heading. |

`Engine.u`, `Pawn` `ScriptText` export 4863 at `0xbbe8c`, declares final latent
native 500 `MoveTo` and final latent native 502 `MoveToward`; its adjacent
shipped comment states that they set `Destination`/`MoveTarget` and that the
actor rotates toward the destination. The underlying rotation contract is
defined on `Actor`: `DesiredRotation` export 199,
`RotationRate` export 61, `bRotateToDesired` export 1157, and
`bFixedRotationDir` export 1155. `Actor` `ScriptText` export 5401 at `0xd98a9`
describes `DesiredRotation` as the pawn rotation target.

### Native latent movement owns the receiving pawn's facing

The shipped `Engine.dll` makes the precedence and ownership explicit. Its PE
image base is `0x10300000`, and the retained decorated symbols identify these
functions without relying on a reference engine:

- `APawn::execMoveTo` body `0x103d8580` writes latent code `0x1f5` to the
  receiving pawn's own state frame at `Pawn+0x0c` (`0x103d8674..0x103d8686`),
  then calls `APawn::rotateToward` at `0x103d86ae` before `moveToward`.
  `APawn::execPollMoveTo` body `0x103d8730` calls `rotateToward` again at
  `0x103d873a` on every poll.
- `APawn::execMoveToward` body `0x103d87a0` similarly writes latent code
  `0x1f7` to that pawn's state frame at `0x103d892b..0x103d8935`, then calls
  `rotateToward`. `APawn::execPollMoveToward` body `0x103d89c0` refreshes its
  target location and calls `rotateToward` at `0x103d8ac1` on every poll.
- `APawn::rotateToward` body `0x103d9e90` computes a rotator from
  `target - Location` and stores it directly at actor offsets
  `0x214..0x21c` (`0x103d9f1d..0x103d9f34`). The `Engine.u` property exports
  identify that field as `Actor.DesiredRotation`.

There is no PlayerPawn exception after those writes. In `AActor::Tick` body
`0x103b3840`, the PlayerPawn path dispatches `PlayerInput` at
`0x103b4159..0x103b417c` and `PlayerTick` at
`0x103b417f..0x103b419b`, then calls virtual `ProcessState` at
`0x103b4248..0x103b424d`, where the latent poll runs. Automatic physics follows
at `0x103b4331..0x103b434c`. `APawn::performPhysics` body `0x103e5520`
explicitly recognizes `APlayerPawn::PrivateStaticClass` at `0x105f1ef0` and
routes it to `APawn::physicsRotation` at `0x103e5647..0x103e565d`.
`physicsRotation` body `0x103e5950` reads current `Rotation`,
`DesiredRotation`, and `RotationRate` and turns toward that desired value; it
has no PlayerPawn early return.

This also establishes why OpenHP1's gate must match the latent receiver rather
than merely detect any active movement latent. In the original native, the
`this` pawn both receives `DesiredRotation` and owns the state-frame latent
code. OpenHP1 represents a latent action as `MoveTo(actor)` or
`MoveToward(actor)`, where `actor` is that native receiver, while state frames
are stored separately. Yielding only when the encoded actor equals the
PlayerPawn reproduces the original per-pawn ownership; yielding for an
unrelated pawn's latent would not.

The matching-latent predicate is an OpenHP1 representation of this ownership,
not a branch present in the native executable: the original pawn's state frame
already makes receiver identity intrinsic. Matching the encoded receiver in
commit `cdbdea6` is therefore faithful, and an unrelated pawn's latent must not
enable rotation.

Both original poll bodies call `rotateToward` before `moveToward`; if
`moveToward` reports completion, they then clear the state-frame latent code.
The same `AActor::Tick` nevertheless reaches `APawn::physicsRotation`
afterward, so the last heading written by that completing poll is still
consumed. OpenHP1 now writes `DesiredRotation` before testing movement
completion and records the receiving pawn for the remainder of the tick. Its
later global physics phase uses that marker after the latent has changed to
`Continue`, preserving both same-tick state resumption and the retail final
turn. The regression covers the completing `MoveTo` poll explicitly. This
evidence does not validate the pre-existing decision to suppress ordinary
PlayerPawn rotation physics when no matching latent or `CutMovingTo` state is
active.

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

### Corrected failing route: `CutScene59`

The door and `Mover36` hypothesis is retracted. In a faithful replay, Harry is
near `(1997.6,-6208,923)` before `CutScene59`; once its movement is active he is
near `(1502.6,-6585,842.1)`. `Mover36` remains at its closed rotation
`(0,16384,16384)` and is not on this route. Earlier forced `CutScene60` probes
also overlapped the startup cast or began from unauthored positions, so their
stalls do not identify the supplied failure.

The original `Lev_Tut1.unr` data gives the active path directly. `CutScene59`
is export 2086 at serialized offset `0x75804`. Its first Harry cast commands
include `CAPTURE`, then `MOVETO LocName0`, `MOVETO LocName1`, and
`MOVETO LocName2` at cast indices 5, 7, and 12. The `Locs` records are compact
object references, not name searches:

- `Locs[0]` at `0x7586e` references `CutMark33` export 671 at
  `(1519.8224,-6270.44,933)`;
- `Locs[1]` at `0x7587e` references `CutMark34` export 796 at
  `(1506.0388,-6563.1367,824.5744)`;
- `Locs[2]` at `0x7588e` references `CutMark40` export 827 at
  `(1620.7673,-6577.815,822.35846)`.

The authored horizontal legs are therefore approximately 293 units toward
`(-13.78,-292.70)` and 116 units toward `(114.73,-14.68)`. `CutMark` itself is
only `class CutMark expands NavigationPoint;` (`HPBase.u`, `ScriptText` export
3478 at `0x2cf610`), while `CutMovingTo.PlayerTick` uses direct `MoveSmooth`
probes rather than navigation or path search. Animation is not a second motion
owner: the shipped `LoopAnim('run')` supplies only the sequence argument even
though `Engine.u`'s native `LoopAnim` declaration has optional root-motion
arguments.

The earlier stale-`CutWalkDest` inference is also retracted. It compared a
forced command with an already-running startup movement. In isolated ordering,
both compiled `CutWalkDest.Location` contexts read the current serialized mark.

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
`MoveActor` to world-model collision. In the shipped `Engine.dll`, `MoveActor`
at `0x103aa719` calls the actor virtual `GetPrimitive`; for Harry's
`CollideType=0`, `AActor::GetPrimitive` at `0x1037a880` selects the generic
`UPrimitive`. `MoveActor` then calls that primitive's
`GetCollisionBoundingBox(actor,true)` virtual at `0x103aa731`.
`UPrimitive::GetCollisionBoundingBox` at `0x103fa2f0` reads
`CollisionRadius` twice, `CollisionHeight` vertically, and `CollisionWidth` as
a vertical center offset. Shipped Harry has radius `15`, width `0`, and height
`42`, so the resulting box is centered on `Location` with half extents
`(15,15,42)`. `MoveActor` derives that center and extent at
`0x103aa743..0x103aa7e8` before its level trace.

The original `UModel::LineCheck` body at `0x10429c80` sends a non-zero extent
to `FBoxLineCheck` at `0x1042a480`. That path sweeps an axis-aligned box through
the BSP hull planes and bevel planes; it does not replace the horizontal
extent with a rounded cylinder. The local licensed SurrealEngine independently
uses the same model-level `TraceAABBModel` path, but the original binary is the
authority for this choice.

OpenHP1 already had a BSP AABB sweep and used it for `CT_Box`; using the
shipped aligned-cylinder bounds for world movement closes that shared native
gap. It is not, by itself, evidence that a forced route which starts from an
unauthored location should clear the same BSP. Actor-versus-actor collision and
spawn placement remain separate shape-specific paths.

### Original BSP traversal limits eligible hulls

`MoveActor` body `0x103aa3a0` passes the collision-box center as `Start`, that
center plus the requested delta as `End`, and the box half extent to
`ULevel::MultiLineCheck` at `0x103aa90c`. It passes Harry's actor-collision bit,
the active `LevelInfo` because `bCollideWorld` is set, and a literal
`ExtraNodeFlags=0`. `MultiLineCheck` body `0x103ac620` performs the world
`UModel::LineCheck` first, with `Actor=None`, before considering actor-hash
hits. There is no hidden center offset for this call: Harry's shipped
`CollisionWidth=0` leaves the center at `Location`.

The box recursion is not a global hull scan. In `FBoxLineCheck` body
`0x1042a480`, the support distance for each partition plane is
`1.1 * dot(abs(Normal), Extent)`; the `1.1` constant is serialized at
`0x10478854`. Applicable back and front children are derived from the start
and end plane distances. The start-side child is visited first (front when the
start distance is at least the negative support, otherwise back), then the
other applicable child. Its outside state follows the compiled CSG relation:

```text
is_csg        = NumVertices > 0 && !(NodeFlags & (ExtraNodeFlags | 0x21))
back_outside  = outside && !is_csg
front_outside = outside || is_csg
```

At a missing child, the parent node's `CollisionBound` is tested only when the
resulting state is inside. `FBspNode::IsCsg` body `0x1042cda0` implements the
first expression exactly. This checks the BSP node's flags, not its surface's
polygon flags. Node 2373 has eight vertices and node flags `0x04`; with
`ExtraNodeFlags=0`, it is CSG. Surface 2785's numerically similar
`PolyFlags=0x21` therefore does not exclude its invisible semisolid brush from
world movement collision.

Replaying that recursion over the shipped Model export 3592 establishes the
grounding contact; that model serializes `RootOutside=false`:

- For the `CutIdleing` downward sweep from
  `(1506.0388,-6563.1367,853.03174)` with extent `(15,15,42)`, the walker
  reaches the coplanar ramp leaves 2371 and 2373. For node 2373 the signed
  center distances are `45.2580566` and `-44.1848145`. The recursive traversal
  support is `48.7015572`, so it reaches that leaf. The later hull-plane clip
  at `0x1042a9c4..0x1042aacb` uses the unscaled support `44.2741432`, placing
  the geometric entry near fraction `0.011`. The ramp contact and its normal
  are therefore expected native behavior. Static reconstruction does not
  uniquely establish which of the two coplanar leaf item numbers retail
  retains as the final hit, but both have the same motion-relevant plane.

This distinction explains why filtering by semisolid surface flags or changing
the slope projection would be wrong: the ramp contact is valid. The actual
shared gap was selecting hull candidates globally instead of allowing the BSP
topology and CSG outside state to select collision leaves.

The zero-momentum `CutScene59` harness did show that the recursive candidate
walk advances through X=1603 and onward to `CutMark40` instead of selecting an
ineligible hull near X=1599. The live replay nevertheless proves that result
does not validate the authored route: it omitted the incoming player momentum
that causes the earlier divergence. A later forced-harness contact around nine
seconds is also an overlapping startup/cutscene diagnostic and is not evidence
for the supplied route. Logged `CollisionHit.node` values identify the entering
hull plane, not the owner of the eligible collision leaf; plane IDs alone must
not be used to infer that the recursion selected an unreachable leaf.

### Exact `CutScene59` grounding displacement

On the faithful replay, Harry reaches the second mark with center
`(1506.0388,-6563.1367,853.03174)`. `CutMovingTo` then cues the cutscene and
enters `CutIdleing`. The compiled Begin of state export 2848 selects physics 5
(`PHYS_Rotating`) and calls native 3969 with `(0,0,-100)` before `LoopAnim`.
In the isolated harness, Velocity and Acceleration were both seeded to zero and
no animation root displacement was present; this call alone changed the center to approximately
`(1506.0388,-6603.1367,833.0317)`.

OpenHP1's first world hit is at essentially time zero with normal
`(0,-0.44721356,0.8944272)`. This is not a generated or guessed plane:
`Lev_Tut1` world Model export 3592 serializes it as BSP node 2373, surface 2785.
The surface belongs to `Brush2342` export 1310, group `DADA classroom`, at
`(1520,-6568,800)`, with `CsgOper=1` and `PolyFlags=0x21` (invisible and
semisolid). The flat comparison hit is node 2490 with normal `(0,0,1)`.

The shipped `moveSmooth` math explains the displacement exactly. Its first
projection at `0x103e4cf4..0x103e4d5c` is
`(Delta - Normal*(Delta dot Normal))*(1-Hit.Time)`, followed by an acceptance
test of `Delta dot projected` at `0x103e4d5f..0x103e4d8e`. For
`Delta=(0,0,-100)` and node 2373's normal, the projection is approximately
`(0,-40,-20)` and the acceptance dot product is positive. There is no
walkable-floor or `Normal.Z` rejection in the original function. On flat node
2490 the same projection is zero.

Therefore `smooth_remaining_delta` is not missing a slope condition in this
case. The original executable's own BSP recursion reaches the same ramp plane
from this center and extent, so it performs the same near-40-unit slide. A
map-specific floor guard, ignoring semisolid BSP during cutscenes, or
discarding horizontal components of a downward `MoveSmooth` would all change
shipped semantics.

### State changes and actor tick order

The shipped native `SetPhysics` has one relevant semantic absent from the
original OpenHP1 implementation: `AActor::setPhysics` body `0x103e5140` zeros
the actor vectors at offsets `0x12c` and `0x13c` when selecting physics 0 or 5.
`Engine.u` identifies those fields as `Velocity` and `Acceleration`. The live
path reaches `CutScene59` by running into its touch trigger, so those vectors
are not generally zero. OpenHP1's missing reset allowed automatic walking
physics to add that stale momentum after the scripted `MoveSmooth`; the
zero-seeded diagnostic concealed this first-leg divergence.

`GotoState` is not deferred to another frame. `AActor::ProcessState` body
`0x1040ef10` compares the current StateFrame node with the saved node after an
opcode at `0x1040eff9`; on a state change it loops back into the interpreter at
`0x1040f00e -> 0x1040efba`, allowing up to four immediate state transitions.
Thus `GotoState('CutIdleing')` can execute that state's Begin and downward
`MoveSmooth` in the same actor tick.

The original actor-local order is also explicit in `AActor::Tick` body
`0x103b3840`. On the PlayerPawn path, `PlayerInput` dispatch at
`0x103b4159..0x103b417c` and `PlayerTick` at
`0x103b417f..0x103b419b` precede virtual `ProcessState` at
`0x103b4248..0x103b424d`; automatic physics is later at
`0x103b4331..0x103b434c`. `ULevel::Tick` body `0x103b6db0` then walks its
Actors array in ascending slot order at `0x103b7177..0x103b71a2`, calling each
actor's Tick before advancing. In original `Lev_Tut1` Level export 3616, Harry
export 602 is actor slot 220 and `CutScene59` export 2086 is slot 1697.

OpenHP1 instead globally runs Tick/PlayerTick events, then all state frames,
then all physics. That is a confirmed engine-order gap and delays
`CutScene59.Tick`'s response to Harry's cue by one frame. It does not prevent
the grounding slide: original Harry executes PlayerTick, immediate state code
including `CutCue` and `CutIdleing` Begin, and physics before the later
`CutScene59` actor ticks. The shipped `CutScene.CutCue` only clears the matching
cast entry's `bWaiting` and `strWaitingFor`; it does not run the next cast
command reentrantly.

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

## SurrealEngine comparison and remaining uncertainty

The local SurrealEngine remains useful only as a secondary implementation
comparison. Its `UActor::TryMoveSmooth` stops after the first projected
`TryMove` and even marks the return with `// XXX: does this break anything?`
(`SurrealEngine/UObject/UActor.cpp`), omitting the shipped binary's
`TwoWallAdjust` and third move. Its PlayerPawn tick placement also differs from
the original binary order established above. Neither should override the
original packages and `Engine.dll`.

The source-backed facing change remains narrow: matching latent `MoveTo` and
`MoveToward` join `CutMovingTo` and the turn latents in the existing
PlayerPawn rotation gate. The original native ordering above confirms that
this precedence is not merely inferred from the script declaration. Retaining
the per-tick authored-rotation marker through physics also matches the native
completion poll. The collision work likewise stays at shared native
seams: aligned pawn bounds, the complete `MoveSmooth` wall adjustment, and
recursive CSG leaf eligibility. None of those collision changes explains the
earlier live divergence. Matching the native `SetPhysics` motion reset removes
the extra pre-collision movement. A normal live approach into `CutScene59` on
2026-08-10 confirmed that Harry follows the authored aisle route without the
timeout teleports.

The original recursive BSP `Outside`/`IsCsg` eligibility remains a shared
collision requirement, but it is not the first cause of this route failure.
An authored `ChessMode` replay remains useful as gameplay confirmation, but is
no longer needed to decide rotation precedence: the shipped PlayerPawn tick,
latent poll, and pawn physics ordering establishes that precedence directly.
