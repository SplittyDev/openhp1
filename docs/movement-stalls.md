# Scripted pawn movement and stalls

This note records the original game contract for cutscene walking, collision,
and recovery. It is deliberately generic: a stalled cutscene should be fixed
at these runtime seams rather than with a map- or actor-specific teleport.
The Fred and George scene that exposed the issue is documented separately in
[`fred-george-cutscene.md`](fred-george-cutscene.md).

The primary evidence is the compiled bytecode, class defaults, and serialized
properties in the legally owned local packages:

- `res/System/HPBase.u`;
- `res/System/Engine.u`;
- `res/System/HarryPotter.u`;
- `res/Maps/Lev_Tut1.unr`.

Embedded `ScriptText` was used only to locate relevant objects. Every behavior
below was checked against compiled bytecode or serialized properties and is
paraphrased rather than reproduced.

## The compiled `CutMovingTo` contract

The NPC and player implementations share the same movement algorithm but run
it from different script events:

| Actor | Setup/call entry points | State movement event | Completion state |
| --- | --- | --- | --- |
| `HPBase.baseChar` | `SetupMoveTo` export 2580; `CutMoveTo` export 2582 | `CutMovingTo.Tick` export 2577 in state export 2578 | `CutIdle` |
| `HPBase.baseHarry` | `SetupMoveTo` export 2814; `CutMoveTo` export 2815 | `CutMovingTo.PlayerTick` export 2806 in state export 2807 | `CutIdleing` |

`SetupMoveTo` stores the target, flattens its Z to the pawn's current Z for the
distance calculation, clears the arrival flag, and computes:

```text
timeout = planar distance / GroundSpeed + 1 second
```

`CutMoveTo` stores the callback and cue, then enters `CutMovingTo`. A null
destination returns the cue immediately instead of entering the state.

The active state selects walking physics and its authored movement animation.
Its movement event then does the following each game tick:

1. derive a planar heading and assign `DesiredRotation`;
2. choose either `GroundSpeed * DeltaTime` or the remaining planar distance;
3. call `MoveSmooth` upward by 15 units, across by the chosen delta, and
   downward by 15 units;
4. mark arrival if the destination was within that tick's requested distance;
5. decrement the timeout regardless of how far collision allowed the pawn to
   move;
6. after timeout, call `SetLocation` at the destination, call `MoveSmooth`
   downward by 100 units, and mark arrival.

The script ignores every `MoveSmooth` result and the `SetLocation` result.
`Actor.MoveSmooth` is native 3969 (`Engine.u`, export 5764), while
`Actor.SetLocation` is native 267 (`Engine.u`, export 5773). Consequently, even
a rejected timeout relocation must still complete the scripted move and
return its callback cue. A visibly misplaced pawn is not, by itself, a reason
for a cutscene to remain captured.

The retained state body sleeps in 0.1-second intervals until the arrival flag
is set. It then emits the stored cue and changes to the idle state. There is no
path search, mover query, door-open wait, `HitWall` override, or retry route in
the active algorithm. The timeout relocation is the original wall/door
recovery.

## Collision and engine call order

The local licensed SurrealEngine reference maps native 3969 to
`Actor.MoveSmooth`
([`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L56-L59))
and forwards it to the actor implementation
([`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L485-L488)).
Its movement path:

- traces against BSP and blocking actors, moves to the blocking fraction, and
  applies actor/player blocking flags
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1588-L1647));
- invokes both actors' `Bump` events synchronously when an actor blocks the
  move
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1707-L1715));
- projects the remaining delta onto the collision plane and tries that slide
  once
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1755-L1769)).

This is distinct from ordinary walking physics. The licensed reference's
`TickWalking` uses `MaxStepHeight`, wall events, repeated movement attempts,
and ground/falling checks
([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L537-L630)).
The original cutscene script explicitly supplies its own fixed `+15 / -15`
probes around `MoveSmooth`; those probes must not be replaced by a latent
`MoveTo` or by pathfinding.

Call order also differs between the two original script classes:

- `baseChar.CutMovingTo.Tick` runs through the actor's script `Tick` event
  before latent-state resumption and physics in the licensed reference
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L386-L418)).
- `baseHarry.CutMovingTo.PlayerTick` runs after `UPawn::Tick`, which already
  includes `UActor::Tick` and physics
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L3817-L3895),
  [`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L4141-L4151)).

SurrealEngine is a comparison implementation rather than the proprietary
engine source, but this order explains why the compiled game deliberately
uses `Tick` for an NPC and `PlayerTick` for Harry. OpenHP1 should preserve
that event distinction.

## State interruption pauses recovery

The timeout is active `CutMovingTo` simulation time, not wall-clock time. It is
decremented only by the state-local movement event. Changing state suspends
both movement and the countdown until that state is restored.

This matters for `HPBase.baseChar.Bump` (export 1255). A collision with Harry
may invoke `DoEmote` (export 2603), which saves the current state and enters
`Emoting` (state export 2601; `BeginState` export 2600). `Emoting` disables
`Tick`, plays the ambient response, then restores the saved state. The bump
handler is supposed to return before this transition when
`baseHud(playerHarry.myHUD).bCutSceneMode` is true.

Therefore, a missing or incorrect HUD cutscene flag can turn ordinary
`MoveSmooth` collision callbacks into repeated `CutMovingTo → Emoting →
CutMovingTo` interruptions. Each interruption pauses the authored timeout.
This can make a door blockage appear to defeat the fallback even though the
fallback itself is present and correct. The state transition is synchronous:
the licensed reference dispatches `Bump` inside movement, and `GotoState`
replaces the active state frame before invoking the new state's `BeginState`
([`UObject.cpp`](../../SurrealEngine/SurrealEngine/UObject/UObject.cpp#L488-L545)).

## Rotating-door interaction

The Fred/George doorway in `Lev_Tut1` is `Mover33`, map export 1824. It uses
`TriggerToggle`, blocks actors and players through the `Engine.Mover` class
defaults, takes two seconds to rotate, and serializes mover encroachment mode
3. Two one-shot triggers, exports 1738 and 1702, address its `FGsec1` tag.

Compiled `Engine.Mover.EncroachingOn` (export 3305) returns false for mode 3,
so a pawn encroachment does not make the mover reject its own interpolation.
That does not make the brush non-blocking to a pawn moving into it.
`CutMovingTo` never asks whether this mover is open; the pawn simply collides,
slides if possible, and continues counting down.

The licensed reference advances a moving brush toward its selected
position/rotation key and only advances interpolation after a successful
brush move
([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1240-L1300)).
For a blocking encroachment mode, its collision path calls
`EncroachingOn` and restores the brush transform when that event returns true
([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1658-L1687)).
Those mover rules and the pawn's `MoveSmooth` rules are separate and both must
be honored.

Moving-brush rotation must use the same runtime mutation path as native
`SetRotation`. Updating only the serialized `Rotation` value and render action
leaves the cached collision transform at the previous angle, so an authored
rotating door can look open while still blocking a scripted pawn as if closed.

The shipped `Engine.Actor` script defines collision type 0 as an aligned
cylinder. Ambient students in `Lev_Tut1`, including the patrol through
`GrandHallDoors`, serialize that type. Their yaw therefore cannot change the
footprint that meets the door. Brush sweeps must preserve the cylinder against
the mover's transform; approximating it with an oriented box creates square
corners that catch on the opened door leaf instead of producing the radial
normal needed to slide past it.

## Harry jump and ledge states are separate

Harry's ordinary jump and ledge-climb flow is not a fallback used by
`CutMovingTo`:

- `HarryPotter.Harry.DoJump` (export 436) starts a jump only from walking
  physics by assigning vertical velocity and switching to falling physics.
- `Harry.Mount` (export 40) selects `Mounting` or `FallingMount` according to
  current physics. `Mounting` is state export 774, `FallingMount` is state
  export 740, and `MountFinish` is state export 633.
- Neither NPC nor Harry `CutMovingTo` calls these functions or enters these
  states. Both force walking physics and use the direct `MoveSmooth` sequence.

A doorway stall should therefore not be repaired by invoking Harry's
jump/mount logic or by adding ledge detection to cutscene walking.

## Runtime invariants for diagnosing a stall

A faithful runtime should be checked at these shared boundaries:

1. dispatch the state-specific `Tick` or `PlayerTick` every applicable frame;
2. preserve the retained state frame while its body sleeps;
3. implement `MoveSmooth` as collision-limited movement followed by a
   collision-plane slide, with synchronous `Bump`;
4. decrement `CutMoveToTimeout` even when the pawn did not move;
5. allow timeout to set the arrival flag and return the cue regardless of the
   relocation result;
6. preserve mover collision and `EncroachingOn` semantics independently;
7. prevent ambient bump emotes while the original HUD cutscene flag is set.
8. keep a moving brush's rendered and cached collision rotations synchronized.
9. preserve an aligned-cylinder pawn shape when sweeping it against a rotated
   moving brush.

If all nine hold, a blocked scripted pawn can end at an imperfect visible
location, but it cannot retain player capture forever. A forced scene release,
actor-specific collision exception, or map-specific teleport would bypass
the original contract rather than restore it.

## Package evidence index

| Package | Compiled/serialized object | Evidence |
| --- | --- | --- |
| `HPBase.u` | `baseChar` exports 2577, 2578, 2580, 2582 | NPC movement event, state body, setup, and call entry |
| `HPBase.u` | `baseHarry` exports 2806, 2807, 2814, 2815 | Player movement event, state body, setup, and call entry |
| `HPBase.u` | `baseChar` exports 1255, 2600, 2601, 2603 | Bump guard and emote state interruption |
| `Engine.u` | `Actor` exports 5764 and 5773 | Native `MoveSmooth` and `SetLocation` declarations |
| `Engine.u` | `Mover` export 3305 and `TriggerToggle` state export 3250 | Encroachment result and door interpolation state |
| `HarryPotter.u` | `Harry` function exports 40 and 436; state exports 633, 740, 774 | Separate jump and mount/ledge flow |
| `Lev_Tut1.unr` | exports 1824, 1738, 1702; `Mover0` export 1693 | Rotating doorways and their triggers |
