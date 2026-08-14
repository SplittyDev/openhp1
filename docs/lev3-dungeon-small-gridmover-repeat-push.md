# Lev3 Dungeon small GridMover repeated-push evidence

## Scope and conclusion

This note traces the small Flipendo cube shown in `Lev3_Dungeon` through the
shipped map, compiled UnrealScript, class defaults, and `Engine.dll`.

The original game has **no two-push limit**. Each relevant Flipendo contact
recomputes `KeyPos[1]` from the cube's current location, offsets it by one
`MoveIncrement`, and starts one collision-aware moving-brush interpolation.
When that interpolation completes or is stopped by a real collision, the
compiled `BumpMove` state resets its scratch key numbers and re-enables
`Bump`. The map actor does not set `bTriggerOnceOnly`, and the compiled
`Mover` default for that property is false. Repeated casts must therefore
remain accepted until ordinary BSP or actor collision blocks movement.

The shipped evidence establishes that rule and disproves a keyframe- or
counter-based ceiling. Investigation found several OpenHP1 divergences in
shared native seams: its
moving-brush tick did not preserve `physMovingBrush`'s mutable remaining-frame
time, its BSP extent trace sampled beyond the requested endpoint, and its
actor-owned brush sweep incorrectly selected a cylinder from `CollideType`
instead of reusing the moving brush's native box extent. Its leaf-hull clipping
also applied the serialized-bound tolerance with the wrong signs. Those are
source-backed shared corrections, but the newest live variable-timestep report
still stops the third push while a fixed 60 Hz replay succeeds. The remaining
OpenHP1 cause is therefore not closed; it is a frame-time-sensitive contact or
support discrepancy, not a retail push counter. No map-specific position or
push count is justified.

## Primary evidence

| File | SHA-256 |
| --- | --- |
| `res/Maps/Lev3_Dungeon.unr` | `3ea3e78b74022546941a63f4ca0c879a092f244f543db24a20043d716467d5c2` |
| `res/System/Engine.u` | `b3661a1d2afb1f730ba5bb3cbd2a6716efaf55fd0d8cefa753b69026a8bc5a85` |
| `res/System/HPBase.u` | `0cec62e098ded3a16024ee15dbc982bf9662b443f630cd19890b7b5d325bf503` |
| `res/System/Engine.dll` | `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc` |
| `res/System/D3DDrv.dll` | `7683b11647dafe3926eff7d0d055abbe3d728648a19f5f8a613fd03efd151599` |

Embedded source text was used only as a readable lead. The active decisions
below were checked against compiled bytecode, metadata, defaults, or native
disassembly.

OpenHP1 gameplay reports and local replays below are diagnostic evidence of
the reimplementation's divergence. They are kept separate from the shipped
files that establish retail behavior.

## Exact map actor

The pictured object is `Lev3_Dungeon.unr` export 2394, `GridMover16`, class
`Engine.GridMover`. It is not either `HProps.WingardiumBlock` actor in the
map.

| Authored field | Value |
| --- | --- |
| Brush | export 3163, `Model860` |
| Location / BasePos | `(944, -5200, -2272)` |
| BaseRot | `(0, -32768, 0)` |
| PrePivot | `(1.998535, -0.002441525, 0)` |
| MoveIncrement | `192` |
| StayOpenTime | `0` |
| CollisionRadius / CollisionWidth / CollisionHeight | `48 / 48 / 48` |
| Local brush bounds | `(-48,-48,-48)..(48,48,48)` |
| bProjTarget | `true` |
| eVulnerableToSpell | `13`, compiled enum value `SPELL_Flipendo` |
| OpeningSound | `HPSounds.Hub1_sfx.flipendo_block_move` |

Its brush uses `BFlipendoShort`, `BFlipendoShort2`, `dungeonf_Dl`, and
`DcenterCol_D`, which also distinguishes the small cube visually from the
nearby tall movers.

The actor's tagged property stream contains no `bTriggerOnceOnly`, authored
push counter, or nonzero `KeyPos[1]` override. Its serialized stack selects
the imported `Engine.GridMover` state/class path.

## Compiled defaults

The compiled `Engine.u` class metadata and UClass default streams establish:

- `Mover.bTriggerOnceOnly` is declared as export 203 but absent from the
  `Mover` default stream, so its exact zero-initialized value is `false`.
- `Mover.BumpType` is declared as export 450 and is absent from the default
  stream, so it is byte zero, `BT_PlayerBump`. This does not reject the spell:
  `bProjTarget=true` makes `Mover.IsRelevant` take its projectile branch first.
- `Mover.MoveTime` export 431 has compiled default `1.0`.
- `Mover.MoverEncroachType` export 824 has compiled byte value 1. Enum export
  2910 maps that to `ME_ReturnWhenEncroach`.
- `Mover.NumKeys=2`, `Physics=PHYS_MovingBrush` while interpolating, and its
  collision/block flags are enabled.
- `GridMover` changes the initial state to `BumpMove` and enables
  `bCollideWorld`; `GridMover16` overrides the class `MoveIncrement` with 192
  and `StayOpenTime` with zero.

The two mover keys are reusable scratch state, not a movement count. The
compiled script overwrites key 1 before every push and resets both key numbers
afterward.

## Compiled spell and GridMover flow

The decisive compiled exports are:

| Package export | Active behavior |
| --- | --- |
| `Engine.u` 3158, `Mover.IsRelevant` | With `bProjTarget`, accepts only a `Projectile` whose virtual `IsRelevantToMover` returns true. |
| `HPBase.u` 2945, `baseSpell.IsRelevantToMover` | Returns true exactly when the projectile dynamically casts to `spellFlip`. |
| `Engine.u` 3167, `GridMover.BumpMove.Bump` | Checks relevance, saves the trigger, sets `KeyPos[1]=Location-BasePos`, applies one dominant-axis `MoveIncrement` away from the spell, then enters `BumpMove.Move`. |
| `Engine.u` 737, `Mover.DoOpen` | Calls `InterpolateTo(1, MoveTime)` and starts the opening sound. |
| `Engine.u` 202, `Mover.InterpolateTo` | Uses current `Location` as `OldPos`, selects key 1, sets `PhysAlpha=0`, `PhysRate=1/MoveTime`, `PHYS_MovingBrush`, and `bInterpolating=true`. |
| `Engine.u` 3317, `GridMover.BumpMove` | Disables `Bump`, calls `DoOpen`, waits in native `FinishInterpolation`, then runs `DoneMoving`. |
| `Engine.u` 3164, `GridMover.Tick` | If movement was interrupted while `bDoingInterpolation` remains true, jumps to `BumpMove.DoneMoving`. |

`BumpMove.DoneMoving` is the repeated-push rule. Its compiled bytecode:

1. clears `bDoingInterpolation`;
2. calls `FinishedOpening`;
3. writes `KeyNum=0` and `PrevKeyNum=0`;
4. sleeps for `StayOpenTime` (zero on this actor);
5. leaves the state only if `bTriggerOnceOnly` is true;
6. otherwise enables `Bump` again.

Thus a successful cast in this decoded direction requests targets at X 752,
560, 368, and so on, with each target derived from the location reached by
the preceding interpolation. Neither `NumKeys=2` nor resetting the two key
numbers limits the number of casts.

## Shipped native movement and collision

The relevant `Engine.dll` exports are jump thunks:

| Function | Export RVA / VA | Implementation VA |
| --- | --- | --- |
| `AActor::physMovingBrush` | `0x328d` / `0x1030328d` | `0x104061f0` |
| `AActor::performPhysics` | `0x3297` / `0x10303297` | `0x103e52c0` |
| `AActor::FindBase` | `0x165e` / `0x1030165e` | `0x103e4fd0` |
| `ULevel::MoveActor` | `0x404d` / `0x1030404d` | `0x103aa3a0` |
| `ULevel::CheckEncroachment` | `0x26fd` / `0x103026fd` | `0x103ab5f0` |
| `FCollisionHash::ActorLineCheck` | `0x4d2c` / `0x10304d2c` | `0x10365cc0` |
| `UModel::LineCheck` | `0x42d2` / `0x103042d2` | `0x10429c80` |
| `AActor::execFinishInterpolation` | `0x1cb2` / `0x10301cb2` | `0x104083b0` |
| `AActor::execPollFinishInterpolation` | `0x46dd` / `0x103046dd` | `0x104084c0` |

The Ghidra C output and instruction-level cross-check give this call chain for
one moving-brush physics tick:

1. `AActor::performPhysics` snapshots `Location`, calls
   `AActor::physMovingBrush(DeltaTime)`, and derives `Velocity` from the whole
   realized displacement.
2. `physMovingBrush` optionally submits its gravity displacement and then its
   keyframe displacement through the level's virtual `MoveActor` path. Both
   use the same mutable stack `DeltaTime` and `FCheckResult` convention.
3. `ULevel::MoveActor` obtains the actor primitive's collision bounding box,
   traces the world `UModel`, asks `FCollisionHash::ActorLineCheck` for actor
   primitives, filters blockers, and then performs its encroachment response.
4. The world and actor-owned brush paths both reach virtual
   `UModel::LineCheck` with the moving brush's box extent. A later overlap-only
   encroachment query is a separate operation.
5. An immediate gravity hit makes `physMovingBrush` call `AActor::FindBase`;
   interpolation completion is exposed to script through
   `FinishInterpolation` only after `bInterpolating` clears.

The exact native constants used by that chain are:

| Constant | Storage / use | Meaning |
| --- | --- | --- |
| `0.51f` | `0x104770a0`; subtracted at `0x103aa80c..0x103aa830` | Reduction applied independently to all three moving-brush box extents before collision tracing. |
| `0.5f` | `0x1046e9c0`; used at `0x10429e3e..0x10429e76` and `0x1042a141..0x1042a17c` | `UModel::LineCheck` hit-distance pullback for point and nonzero-extent traces. |
| `0.1f` | `0x104737f4`; leaf-hull path beginning `0x1042ab7d` | Serialized hull-bound tolerance, with the axis-dependent signs listed below. |
| `8.0f` | immediate in `AActor::FindBase` | Downward `FindBase` probe distance. |

Confirmed `physMovingBrush` control flow:

- `0x10406261` requires the actor's `bInterpolating` flag.
- `0x10406282..0x104063dc` projects the current velocity onto zone gravity,
  computes `Velocity*DeltaTime + Gravity*0.5*DeltaTime^2`, then adds
  `Gravity*DeltaTime` to the actor's velocity before the gravity move.
- `0x104063e2..0x104065b5` performs that move, translates the active key and
  `OldPos` by any realized displacement, or calls `FindBase` on an immediate
  hit.
- `0x104065ba..0x10406626` advances and clamps
  `PhysAlpha + DeltaTime * PhysRate`.
- `0x104065ec..0x1040661f` mutates the same stack `DeltaTime`: crossing alpha
  1 leaves only interpolation overshoot, while an unfinished interpolation or
  a tick beginning at alpha 1 leaves zero time.
- `0x10406684..0x10406860` derives the requested location and rotation from
  the selected mover key.
- `0x10406866..0x104068ac` initializes an `FCheckResult` with `Time=1.0` and
  calls the level's virtual `MoveActor` path.
- `0x104068b8..0x104068cd` replaces the requested alpha advance with the
  fraction actually reached according to `FCheckResult.Time`.
- `0x104068d3..0x104068d8` tests the local gravity-moved flag and jumps back to
  `0x10406261` before collision or interpolation completion when it is set.
- `0x104068de..0x104068ff` clears `bInterpolating` when collision leaves
  `Time < 1.0`.
- `0x10406904..0x10406947` clears `bInterpolating` at the target and dispatches
  `InterpolateEnd` on normal completion.

`physMovingBrush` is nested inside `AActor::performPhysics` at `0x103e52c0`.
The moving-brush branch snapshots `Location` into `OldLocation` at
`0x103e5326`, calls `physMovingBrush` at `0x103e534c`, then unconditionally
writes `(Location - OldLocation) / DeltaTime` to `Velocity` at
`0x103e5351..0x103e539e`. This outer write is part of retail behavior even
though it is absent from the inner function.

`FindBase` is not a generic snap-to-floor operation. Its exported Ghidra C
body constructs an endpoint exactly 8 units below the
current location, obtains the actor's cylinder extent, and invokes the level's
single-line collision query with trace flags 7. At
`0x103e5097..0x103e50a9` it compares the returned hit actor with the existing
base and calls `SetBase` only when they differ. In `physMovingBrush`, the call
site is `0x104065b3`, reached after the gravity `MoveActor` returns an immediate
hit rather than after every partial or successful gravity displacement.

Confirmed `MoveActor` and model-trace geometry:

- `0x103aa719..0x103aa741` obtains the moving actor's primitive collision box;
  `UPrimitive::GetCollisionBoundingBox` at `0x103fa2f0..0x103fa3d0` derives it
  from actor location and collision dimensions.
- `0x103aa80c..0x103aa830` reduces each moving-brush extent by the exact float
  constant 0.51 before the world trace.
- `0x103aa666..0x103aa696` gives moving brushes zero endpoint padding.
- `UModel::LineCheck` at `0x10429c80` traces only the supplied segment. Its
  nonzero-extent path at `0x1042a141..0x1042a17c` subtracts exactly 0.5 units
  from the realized hit distance and clamps it; the point path has the same
  pullback at `0x10429e3e..0x10429e76`.
- The nonzero-extent leaf-hull clip at `0x1042ab7d..0x1042b341` applies the
  serialized-bound tolerance asymmetrically: minimum X/Y/Z are reduced by
  0.1, maximum X/Y are reduced by 0.1, and maximum Z is increased by 0.1.

`execFinishInterpolation` sets latent action `0x12e` at
`0x104083b0..0x104083c5`. Its poll implementation at
`0x104084c0..0x104084d3` releases the state frame only after
`bInterpolating` clears. A real blocking hit therefore ends this attempted
increment; `GridMover.Tick`/`DoneMoving` still perform their normal cleanup
and re-enable the spell contact unless `bTriggerOnceOnly` says otherwise.

The related native brush encroachment trace is recorded in
[`lev3-dungeon-pillar-gate.md`](lev3-dungeon-pillar-gate.md): retail
`MoveActor` asks the collision hash for actor hits as well as tracing the world.
`FCollisionHash::ActorLineCheck` gets each candidate actor's primitive and
calls its virtual `LineCheck`; an actor-owned `UModel` therefore participates
in the moving sweep. The extent passed to that call is the same box extent
which `MoveActor` obtained at `0x103aa719..0x103aa830`; `CollideType` does not
select a cylinder for this path. `MoveActor` then applies the ordinary
`IsBlockedBy` filter and excludes the based-actor relationships checked at
`0x103aa995..0x103aa9b6`.

The later overlap pass is deliberately different. In
`FCollisionHash::ActorEncroachmentCheck`, `0x10365a54..0x10365a82` tests an
actor-owned primitive's owner and skips it when it is an `ABrush` whose
`bStatic` bit is clear. Thus a dynamic mover can stop another mover during the
continuous sweep, but it must not make `CheckEncroachment` restore the move a
second time merely because the two transformed brush bounds touch. Static
brush actors remain eligible. `ME_ReturnWhenEncroach` is still the authored
response to a genuine remaining overlap; it is not a push-count rule.

## OpenHP1 divergence history and remaining live mismatch

OpenHP1's `tick_moving_brush` performed its `bCollideWorld` gravity movement
before the interpolation loop checked `bInterpolating`. Because GridMover
leaves `Physics=PHYS_MovingBrush`, gravity and world collision continued on
every idle tick after `FinishInterpolation` released `BumpMove`. That ordering
does not exist in the shipped native function: the branch at `0x10406261`
leaves `physMovingBrush` immediately when `bInterpolating` is clear, before the
gravity branch at `0x10406282`.

Adding only that gate exposed the paired divergence. Report
`report-1786674310-234658000.md` captured `GridMover15` at
`(560,-5200,-2224.4)`: four 96-unit horizontal increments had completed,
but the tall brush fell only 0.4 units and remained on the ledge. Retail's
local gravity-moved branch at `0x104068d3..0x104068d8` restarts the moving-brush
path before it can clear `bInterpolating`; OpenHP1 instead completed the
interpolation after one gravity attempt, so the new idle gate prevented the
remaining fall.

A first attempt then repeated the gravity path with the original full frame
duration each time. That contradicted `0x104065ec..0x1040661f` and made the
brush fall to support in one rendered frame. The implementation now carries
one mutable `time_left` through the entire native path. An unfinished
interpolation consumes it, retaining `bInterpolating` for the following game
tick and producing the retail smooth fall.

Report `report-1786675375-221108000.md` also captured `GridMover15` rejecting
the initial negative-X push from player position `(487.5,-5377,-2259)`. The
old BSP sweep extended that 96-unit request by one unit and combined a
0.1-unit inflated transformed brush bound with the native 0.51 shrink. It
therefore manufactured a hit just beyond the requested endpoint. The corrected
native collision-box extent `(47.49,47.49,95.49)` and exact segment leave the
first `(368,-5392,-2224)..(272,-5392,-2224)` request clear; a later request
from X 272 genuinely meets world node 2269 and remains blocked.

Report `report-1786677933-000711000.md` and the following live playthrough
exposed the actor half of the same path. OpenHP1 had excluded every moving
brush from actor sweeps, then tried to compensate in the post-move
encroachment pass. That made the tall movers either stop short, penetrate one
another, or become an unconditional wall for `GridMover16`. Retail instead
performs the actor primitive sweep first and skips only non-static
actor-owned brushes in the later encroachment overlap pass, as the two native
traces above show. OpenHP1 now follows that split and applies the same native
0.51-unit extent reduction to a moving brush in both its world and actor-owned
model sweeps.

Report `report-1786683001-255939000.md` isolated the final remaining mismatch.
`GridMover16` accepted its third cast and began interpolating from
`(559.531,-5200,-2272.8052)`, but its next 0.86-unit negative-X step returned
`Time=0` from fixed world node 2276, plane
`[1,0,0,511.9995]`. That node's collision hull ends at Z -2320, so this was the
east face of the fixed bridge lip, not either tall mover. The cube had settled
about 0.2 units too low on `GridMover14` and consequently clipped that face.

One real divergence was in the preceding mover-on-mover support sweep. OpenHP1 used
`CollideType=0` to send the moving brush through its cylinder helper when the
other primitive was a brush. That axis-aligned fast path omitted the exact
0.5-unit `UModel::LineCheck` pullback shown at
`0x1042a141..0x1042a17c`. Retail has already constructed the moving brush's
box extent before querying both world and actor primitives, so no such
`CollideType` branch exists. OpenHP1 now always uses its already computed
moving-brush box extent for an actor-owned brush sweep. A synthetic regression
places a default-`CollideType` moving brush onto another brush and requires the
native 19.89-unit resting center.

The latest trace, `report-1786684425-868040000.md`, proved that correction was
not sufficient. The third spell was accepted and interpolation began, but a
small gravity step moved `GridMover16` from Z -2272.77 to -2272.8015. Its next
negative-X substep then hit fixed BSP node 2276 at `Time=0`. The prior gravity
probe had missed the supporting mover only because its distance was shorter
than the small contact gap; the following, longer probe found `GridMover14`.

Removing the position-derived write was disproved by the outer
`performPhysics` trace. It allowed the cube to cross offline, but left a large
negative gravity velocity on a completed mover. Shipped `PlayerPawn.DoJump`
adds `Base.Velocity.Z` when jumping from a mover, explaining the observed
instant death while jumping or climbing from these blocks. OpenHP1 now mirrors
the outer write and produces zero velocity on the next idle moving-brush tick.

One confirmed cube-stop cause was the leaf-hull bound tolerance. OpenHP1 had
shrunk both ends of every serialized hull bound by 0.1. Retail instead uses the
exact axis bias listed above, expanding the vertical clip by 0.1 at each end.
The 0.2-unit vertical discrepancy exceeded the cube's measured 0.1294-unit
overlap with the fixed bridge lip. An exact unit check records all six native
adjusted bounds.

The runtime regression starts a gravity-affected interpolation above its
floor, requires a sub-unit first-frame drop and continued interpolation, lets
it reach support over later ticks, then proves the completed brush stays idle
with zero velocity. A full-map 60 Hz replay seated the first tall mover at
`(560,-5200,-2416.01)`, seated the second beside it at
`(656,-5200,-2416.01)`, and moved `GridMover16` through X 752, 560, and 368 at
constant bridge height Z -2272.4973. That replay was a transient local
fixed-step diagnostic rather than a captured gameplay report, so the numbers
are recorded here but must not be treated as live acceptance evidence.
A separate packaged-state probe dispatched four consecutive relevant `Bump`
events to `GridMover16`; all four were accepted after `DoneMoving`, confirming
there is no runtime two-use event ceiling. Physical motion still ends normally
when the ordinary collision sweep reports a blocking hit.

### Newest live report: frame-time-dependent support loss

The newest capture is
`/Users/splitty/Library/Application Support/OpenHP1/Reports/report-1786689714-906406000.md`.
It records a 24.39 ms frame-time sample at line 33 and disproves the claim that
the shared collision fixes above fully resolved live play:

| Actor | Exact trace evidence | Captured final state |
| --- | --- | --- |
| #94 `GridMover14`, map export 2743 | Lines 1938-1948: a 4.0764117-unit fall hits BSP node 2250 at `Time=0.092590` and reaches Z -2416.01; the immediately following 0.244624-unit fall misses, reaching Z -2416.2546; the next 0.44121015-unit fall returns `Time=0`. | Lines 11303-11308 round the location to `(560,-5200,-2416.3)`; the trace preserves exact Z -2416.2546. |
| #95 `GridMover15`, map export 2741 | Lines 4258-4263: it reaches Z -2416.01 on node 2250 and the next 2.171346-unit fall returns `Time=0`. | Lines 11240-11246 record `(656,-5200,-2416.0)`. |
| #98 `GridMover16`, map export 2394 | Lines 6048-6086: support is first reported at Z -2272.5059; a shorter downward sweep then misses, the cube falls to Z -2272.7256, and the third push starts from Z -2272.749 before fixed node 2276 rejects a 0.27270508-unit negative-X substep at `Time=0`. | Lines 11247-11253 round the stopped location to `(559.8,-5200,-2272.7)`. |

The cube trace is especially diagnostic. At X 560.7697, a 0.03427485-unit
downward sweep reports immediate support from actor #95. After a clear
0.4031372-unit horizontal substep to X 560.3666, a shorter 0.024827043-unit
downward sweep reports no support. It then falls through several short probes
until a 0.20373785-unit probe at Z -2272.7256 reports actor #94 at `Time=0`.
The following cast moves from X 560 to 559.9474 and 559.7574 before BSP node
2276 rejects the next horizontal substep.

A read-only dump of node 2276 gives plane
`[1, 8.742278e-8, -0, 511.9995]` and collision-hull bounds
`[415.99924,-5248,-2448]..[511.99997,-5152,-2320]`. With the native-reduced
cube extent 47.49, the cube's bottom at the failed third push is approximately
-2320.239. It is therefore about 0.339 units below the hull's retail-adjusted
top at -2319.9. Node 2276 is a real fixed bridge-lip contact once the cube is
that low; it is evidence of the preceding support-height error, not evidence
of a retail two-push rule.

### Fixed 60 Hz versus live variable timestep

The fixed-step replay and newest live report take different collision paths:

| Run | Tall-mover support height | Cube result |
| --- | --- | --- |
| Local replay, `DeltaTime=1/60` | #94 and #95 both settle near Z -2416.01. | Holds near Z -2272.4973 and completes pushes to X 752, 560, and 368. |
| Live render-driven timestep | #94 falls an extra 0.2446 units to Z -2416.2546 while #95 remains at Z -2416.01. | Falls to about Z -2272.749 and hits node 2276 during the third push. |

This difference is explained by the current host timing path: OpenHP1 selects
`wgpu::PresentMode::AutoNoVsync` in `crates/openhp1-game/src/app.rs`, and passes
direct wall-clock frame duration (capped only at 0.1 seconds) into the game
update. The shipped
localized defaults contain `RefreshRate=60Hz` at
`res/System/0/Default.ini:233` and `res/System/1/Default.ini:233`, but only in
the `[GlideDrv.GlideRenderDevice]` section. That is not evidence that the
normally used Direct3D path or Engine physics was globally fixed at 60 Hz.

The exported Ghidra C narrows the native timing rule further. `ULevel::Tick`
(export `0x103010a5`, implementation `FUN_103b6a90`) clamps its incoming
`DeltaSeconds` to the inclusive range 0.005..0.1 before applying the level time
dilation and passing the result to actor ticks. OpenHP1 currently implements
the 0.1-second ceiling in the game host but not the native 0.005-second floor.
That floor is an exact parity gap worth restoring, but it does not close this
bug: a 5 ms post-hit gravity displacement can still be shorter than the
0.5-unit line-check pullback interval.

`UGameEngine::GetMaxTickRate` (export `0x103017ad`) returns zero for an ordinary
standalone client and only returns a bounded rate for network driver/client
states. `UEngine::GetMaxTickRate` likewise returns zero outside the editor.
Consequently `Engine.dll` itself provides no evidence for a 60 Hz standalone
cap.

The shipped Direct3D 7 driver does establish presentation synchronization in
the ordinary fullscreen path. Ghidra identifies
`UD3DRenderDevice::Unlock` at export VA `0x100010cd`. When presenting a
fullscreen back buffer it calls DirectDraw surface `Flip` with `DDFLIP_WAIT`
(`0x1`) and conditionally adds `DDFLIP_NOVSYNC` (`0x8`) from the boolean field
at offset `0x9cc`. The property-registration function at `0x100019b0` maps
that field to the config name `UseVSync`; it does not assign a nonzero class
default. No shipped INI under `res/System` sets `UseVSync`, so the zero-valued
shipped configuration takes the synchronized `DDFLIP_WAIT` path without
`DDFLIP_NOVSYNC`. (The historical property name is counterintuitive; the
actual flip flags are decisive.)

This does not prove a fixed 60 Hz physics step: fullscreen retail ticks follow
the display's synchronized presentation cadence. It does prove that
OpenHP1's uncapped `AutoNoVsync` redraw loop is not equivalent to the shipped
fullscreen D3D path.

The `AutoVsync` diagnostic changed only that presentation request. The user
then completed the two tall-block bridge, moved the small block through the
previous third-push failure, and confirmed mover movement, falling, climbing,
and jumping behaved correctly. This is live acceptance that presentation
cadence, rather than another mover or collision rule, caused the remaining
failure. Metal's FIFO path ran this build at approximately 30 FPS, however,
making it too laggy as the permanent solution.

The retained host fix therefore restores `AutoNoVsync` and schedules redraws
with Winit `ControlFlow::WaitUntil` so consecutive game frames begin no sooner
than 1/60 second apart. It does not sleep inside rendering, change mover
physics, or replace native variable-delta simulation with a fixed step. A
frame that overruns its deadline continues immediately instead of adding
another delay. This is a 60 FPS compatibility policy supported by the shipped
60 Hz renderer setting, the successful fixed-60 replay, and the accepted
presentation-synchronized live run; it is not claimed as an
`Engine.dll::GetMaxTickRate` rule. Live acceptance of this final no-vsync
60 Hz host configuration remains pending.

The mover trace itself contains interpolation-alpha advances of approximately
0.0096 to 0.0116 with `PhysRate=1`, corresponding to variable updates near
86-104 Hz. The report's 24.39 ms header is only the frame sampled when the
report was written; it does not describe every earlier trace entry.

There is also one concrete `FindBase` divergence, but its native ordering rules
it out as the cause of this support loss. After an immediate gravity hit,
current OpenHP1 directly chooses the
hit actor (or `LevelInfo`) as `Base` in
`crates/openhp1-runtime/src/world/physics/dynamics.rs:1158-1166`. Retail instead
runs the independent 8-unit downward cylinder query described above. The
existing generic OpenHP1 query helper at
`crates/openhp1-runtime/src/world/movement.rs:964-983` reconstructs the actor's
ordinary collision shape; it does not itself reproduce native `FindBase`'s
forced cylinder extent and flags. However, both implementations reach this
path only after `MoveActor` has already returned `Time=0`; it cannot make the
earlier short gravity sweep hit or prevent the extra settling displacement.

The causal chain is now established: the 0.5-unit native line-check pullback
leaves a clearance interval after a partial hit; an uncapped short following
gravity displacement can miss, move the brush closer, and let a later longer
displacement stop at `Time=0`. At the accepted slower cadence the following
displacement is long enough to retain support. The outstanding check is only
whether the explicit 60 Hz no-vsync host pacing reproduces the accepted
`AutoVsync` behavior without its Metal 30 FPS stall.

## Serialized-elevation collision cross-check

The cube's first two 192-unit pushes in the decoded negative-X direction reach
X 752 and X 560. Its third target is `(368,-5200,-2272)`.

The two tall movers at that X coordinate have these decoded world bounds:

| Actor | Location | World Y bounds |
| --- | --- | --- |
| `GridMover15` | `(368,-5392,-2224)` | `-5440..-5344` |
| `GridMover14` | `(368,-5008,-2224)` | `-5056..-4960` |

At the third target the small cube occupies approximately Y
`-5248..-5152`. It remains 96 units clear of each tall mover. Their authored
brush bounds therefore do not justify rejecting the third increment.

A read-only sweep of the actual transformed `Model860` hull against the
decoded world BSP also keeps the X `560..368` segment clear. The following
`368..176` request first meets world node 2263 at approximately half the
segment: plane `[1,0,0,223.99954]`, on a solid `palestone` surface. That is the
authored west-wall stop, leaving the actor location near X 269 rather than
requiring it to reach the scratch target at X 176.

This is intentionally only a serialized-elevation cross-check. Native active
interpolation can apply gravity and change the cube's elevation, so these
horizontal sweeps do not define how many increments every direction must
complete. They only rule out the adjacent authored pillars as a fixed
two-push counter or unconditional third-cast blocker.

## Rejected hypotheses and implementation boundary

- **Two mover keys allow only two pushes:** rejected. Key 1 is recomputed from
  current location before every cast; both key numbers are reset afterward.
- **The map enables one-shot behavior:** rejected. `GridMover16` has no
  `bTriggerOnceOnly` override and the compiled inherited value is false.
- **The map authors a hidden push count:** rejected. No counter or equivalent
  property exists on the actor or in the compiled GridMover state.
- **BumpType stops later spell contacts:** rejected. `bProjTarget` routes
  Flipendo through the projectile relevance override before BumpType.
- **The third Flipendo contact is rejected before movement starts:** rejected.
  The newest live trace shows #98 enter a fresh interpolation at X 560 and
  complete two negative-X substeps before BSP node 2276 ends that increment.
- **The two adjacent pillars block the third target:** rejected by their
  decoded authored brush bounds. Once deliberately moved into the bridge,
  their top faces support the small cube; the native 0.51-unit moving extent
  reduction leaves its tangential horizontal sweep clear.
- **The shipped `RefreshRate=60Hz` setting proves fixed-step physics:**
  rejected. It belongs specifically to the Glide renderer configuration, and
  no shipped timing trace here proves how the Direct3D host converts render
  cadence to physics `DeltaTime`.
- **The corrected leaf-hull tolerance alone resolves live play:** rejected by
  `report-1786689714-906406000.md`; the variable-timestep run still lets the
  support height drift before node 2276 blocks the third push.
- **Changing this actor's location, MoveIncrement, collision dimensions, or
  one-shot flag:** forbidden by the evidence. The original behavior comes
  from shared GridMover state and native moving-brush collision semantics.

The confirmed implementation seams are shared native behavior: the
interpolation gate surrounds gravity and collision, one mutable frame-time
value controls retries, and moving-brush BSP queries use the actor collision
box over the exact requested segment. Once an active interpolation really hits
BSP or a blocking actor, that increment ends exactly as retail does. Matching
those individual rules is necessary but, as the latest live report shows, not
yet sufficient to claim parity for render-dependent support transitions.

## Reproduction commands

```sh
rtk shasum -a 256 \
  res/Maps/Lev3_Dungeon.unr \
  res/System/Engine.u \
  res/System/HPBase.u \
  res/System/Engine.dll \
  res/System/D3DDrv.dll

rtk target/debug/examples/package_inspect res/Maps/Lev3_Dungeon.unr | \
  rtk rg 'GridMover16|Model860'

rtk target/debug/examples/actor_bounds res/Maps/Lev3_Dungeon.unr GridMover16
rtk target/debug/examples/actor_bounds res/Maps/Lev3_Dungeon.unr GridMover15
rtk target/debug/examples/actor_bounds res/Maps/Lev3_Dungeon.unr GridMover14

rtk target/debug/examples/package_inspect res/System/Engine.u | \
  rtk rg 'GridMover|BumpMove|IsRelevant|InterpolateTo|FinishInterpolation'
rtk target/debug/examples/script_inspect res/System/Engine.u 3158
rtk target/debug/examples/script_inspect res/System/Engine.u 3167
rtk target/debug/examples/script_inspect res/System/Engine.u 3317
rtk target/debug/examples/script_inspect res/System/Engine.u 737
rtk target/debug/examples/script_inspect res/System/Engine.u 202
rtk target/debug/examples/script_inspect res/System/HPBase.u 2945

rtk proxy rg -a -n -A 100 'class GridMover extends Mover;' res/System/Engine.u
rtk proxy rg -a -n -A 40 'function bool IsRelevant\( actor Other \)' res/System/Engine.u
rtk proxy rg -a -n -A 10 'function bool IsRelevantToMover' res/System/HPBase.u

rtk proxy objdump -p res/System/Engine.dll | \
  rtk proxy rg 'physMovingBrush|performPhysics|FindBase|MoveActor|CheckEncroachment|FinishInterpolation'
rtk proxy objdump -d --start-address=0x104061f0 --stop-address=0x10406998 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x103e52c0 --stop-address=0x103e53b0 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x103e4fd0 --stop-address=0x103e50b0 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x103aa3a0 --stop-address=0x103aabc0 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x103fa2f0 --stop-address=0x103fa3e0 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x10429c80 --stop-address=0x1042a190 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x1042ab60 --stop-address=0x1042b350 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x104083b0 --stop-address=0x104084d6 res/System/Engine.dll
rtk proxy objdump -s -j .rdata res/System/Engine.dll | \
  rtk proxy rg '^ 1046e9c0|^ 104737f0|^ 104770a0'

# Ghidra: D3DDrv UD3DRenderDevice::Unlock (0x100010cd) and the
# UseVSync property-registration function (0x100019b0).
rtk rg -a -n 'UseVSync' res/System --glob '*.ini' --glob '*.int'

rtk sed -n '1928,1950p;4238,4265p;6038,6088p;11240,11310p' \
  '/Users/splitty/Library/Application Support/OpenHP1/Reports/report-1786689714-906406000.md'
rtk nl -ba crates/openhp1-game/src/app.rs | rtk sed -n '700,710p;827,837p'
rtk nl -ba res/System/0/Default.ini | rtk sed -n '229,235p'
rtk nl -ba res/System/1/Default.ini | rtk sed -n '229,235p'
```

The read-only actor/default decoder used to cross-check the tagged property
streams was run outside the repository from `/tmp/openhp1-actor-inspect`; its
outputs are summarized above, and no extracted original-game data was added
to the repository.
