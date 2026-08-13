# Fred and George hallway cutscene

This note records the authored flow for the first Fred and George encounter in
`Lev_Tut1`. It is intended to make runtime diagnosis reproducible without
adding map-specific recovery code.

The primary evidence is the legally owned local packages:

- `res/Maps/Lev_Tut1.unr`
- `res/System/Engine.u`
- `res/System/HPBase.u`
- `res/System/HarryPotter.u`
- `res/System/HPMenu.u`
- `res/System/Tut1.u`

Embedded `ScriptText` was used only to orient the investigation. Every
behavioral claim below was checked against compiled bytecode, class defaults,
or serialized map properties through the repository's package and script
decoders. The note paraphrases the scripts and does not contain an extracted
source dump.

## Scene and cast

The encounter is `Lev_Tut1.CutScene52`, map export 2257. Its serialized
location is `(225.483, -2887.088, 90)`. It inherits the compiled `CutScene`
defaults `bPlayOnce`, `bTouchStarts`, `bTriggerStarts`, and `bCanPlay` as true.

Its relevant cast bindings are:

| Cast slot | Alias | Map actor |
| --- | --- | --- |
| 0 | Harry | export 602, `harry0` |
| 1 | Fred | export 1328, `Tut1Fred2` |
| 2 | George | export 1325, `Tut1George3` |
| 3 | camera | export 621, `PotCam0` |
| 4 | Ron | export 1347, `Tut1Ron0` |
| 5 | background student | export 1389, `gen_male_0` |

Fred and George both have a map-authored `GroundSpeed` of 200. Fred begins at
`(169.040, -3037.473, 90.017)` and George at
`(282.971, -3033.848, 90.017)`.

## Harry's HUD and `CAPTURE`

`Lev_Tut1` does not serialize a HUD actor, and export 602, `harry0`, has no
`myHUD` property override. The HUD is a runtime-spawned actor rather than a
map actor with a stable export index.

The compiled creation path is:

1. `Harry.PostBeginPlay` in `HarryPotter.u` (export 175) calls the inherited
   implementation and then assigns its inherited `PlayerPawn.HUDType` to
   `HPMenu.HPHud`.
2. `PlayerPawn.PreRender` and `PlayerPawn.PostRender` in `Engine.u` (exports
   3294 and 3292) both contain the same lazy bootstrap. If `myHUD` already
   exists, the hook renders it. Otherwise, when `Player` is a viewport and
   `HUDType` is non-null, the hook assigns
   `myHUD = Spawn(HUDType, self)`.
3. Native `Spawn` therefore creates an `HPMenu.HPHud` actor, whose superclass
   chain is `HPBase.baseHud -> Engine.HUD -> Engine.Actor`, with `harry0` as
   its owner. The call supplies neither a tag nor a transform, so those use
   the ordinary actor-spawn defaults.

The HUD's durable identity is the object reference stored in
`harry0.myHUD`: it has no map export or fixed actor number, and runtime code
must not find it by a hard-coded generated name. The licensed reference
engine's actor spawn path shows the class-based transient allocation, owner
assignment, and event order
([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L19-L108)).
That order is `Spawned`, `PreBeginPlay`, `BeginPlay`, `PostBeginPlay`, then
`SetInitialState`. Compiled `HPHud.PostBeginPlay` (export 1750) only calls the
inherited `Actor.PostBeginPlay` and returns; there is no additional
Harry-Potter-specific HUD setup event required before the cutscene properties
can be written.

For Harry, compiled `CutScene.handleCast` implements `CAPTURE` in this order:

1. cast `PlayerHarry.myHUD` to `baseHud` and set `bCutSceneMode = true`;
2. set that HUD's `curCutScene` to the current cutscene;
3. call `PlayerHarry.CutDoIdle`.

`RELEASE` performs the inverse HUD writes and then calls
`PlayerHarry.CutRelease`. The two HUD writes use UnrealScript context
expressions. If `myHUD` is unexpectedly null, the context reports
`AccessedNone` and does not evaluate the member expression
([`ExpressionEvaluator.cpp`](../../SurrealEngine/SurrealEngine/VM/ExpressionEvaluator.cpp#L245-L258));
it does not turn the following `CutDoIdle` call into a deferred operation.
Consequently, reproducing the original lazy HUD spawn is required for the
visible cutscene HUD state, but a missing HUD must never block or postpone the
cutscene track itself.

## Authored track dependency

`CutScene52` runs one command track per cast slot. Blank serialized slots
parse as `CUT_NONE` and still advance the track, so the authored array indices
below are significant.

### Harry, cast 0

| Index | Authored operation | Dependency/effect |
| ---: | --- | --- |
| 1 | `CAPTURE` | Enters Harry's cutscene idle state and enables HUD cutscene mode. |
| 2 | `MOVETO HPLoc1` | Waits for automatic cue `2Lev_Tut1.harry0`. |
| 3 | `CUE FredTalk` | Releases Fred's initial wait. |
| 4 | `WAITFOR Leave` | Waits for Fred's first dialogue exchange. |
| 5 | `CUE RonCam` | Releases the camera and Ron tracks that wait for this phase. |
| 6 | `FACE Ron` | Direct rotation; no callback cue. |
| 7 | `WAITFOR RonGone` | Waits for Ron's explicit departure cue. |
| 8 | `SLEEP 2.5` | Track-local delay. |
| 9 | `MOVETO FredPath1` | Waits for `9Lev_Tut1.harry0`. |
| 10 | `CUE CameraFollow` | Releases the camera's first follow phase. |
| 11 | `MOVETO AllPath` | Waits for `11Lev_Tut1.harry0`. |
| 12 | `CUE CameraChange` | Allows the camera to recapture for the final room. |
| 13 | `SLEEP 0.5` | Track-local delay. |
| 14 | `MOVETO HPLoc2` | Waits for `14Lev_Tut1.harry0`. |
| 15 | `FACE Target` | Faces export 943, `CutMark16`; no callback cue. |
| 17 | `CUE FredTalkAgain` | Releases Fred's final dialogue phase. |
| 19 | `WAITFOR CutEnd` | Waits only for Fred. |
| 21 | `RELEASE` | Clears HUD cutscene mode and restores Harry's saved state. |

### Fred, cast 1

| Index | Authored operation | Dependency/effect |
| ---: | --- | --- |
| 1 | `WAITFOR FredTalk` | Waits for Harry's index-3 cue. |
| 2 | `FACE HPLoc1` | Direct rotation. |
| 3–4 | two dialogue operations | Delivers `FRED_GEORGE_001` and `_002`. |
| 5 | `CUE Leave` | Releases Harry's index-4 wait. |
| 6 | `WAITFOR RonGone` | Waits for Ron's departure. |
| 8 | `MOVETO FredPath1` | Waits for `8Lev_Tut1.Tut1Fred2`. |
| 9 | `MOVETO FredPath2` | Waits for `9Lev_Tut1.Tut1Fred2`. |
| 10 | `SLEEP 1.0` | Track-local delay while the door is opening. |
| 11 | `MOVETO FredPath2` | Waits for `11Lev_Tut1.Tut1Fred2`. |
| 12 | `MOVETO FredPath3` | Waits for `12Lev_Tut1.Tut1Fred2`. |
| 13 | `MOVETO AllPath` | Crosses the rotating door and waits for `13Lev_Tut1.Tut1Fred2`. |
| 14 | `MOVETO FredLoc` | Reaches the bookshelf and waits for `14Lev_Tut1.Tut1Fred2`. |
| 15 | `FACE HPLoc2` | Faces Harry's final position. |
| 16 | `WAITFOR FredTalkAgain` | Waits for Harry's index-17 cue. |
| 19, 21, 23 | three dialogue operations | Delivers `FRED_GEORGE_003`, `_004`, and `_005`. |
| 27 | `CUE CutEnd` | Releases Harry and the recaptured camera. |

Because the movement handler increments `nCurAction` immediately after
starting an asynchronous move, Fred blocked during index 13 is observable as
`nCurAction == 14`, `bWaiting == true`, and
`strWaitingFor == "13Lev_Tut1.Tut1Fred2"`. His `CutWalkDest` must be
`AllPath`. Once that callback is returned, index 14 starts the final movement
and the analogous waiting state uses the `14...` cue.

### George, cast 2

| Index | Authored operation | Dependency/effect |
| ---: | --- | --- |
| 2 | `WAITFOR RonGone` | Shares the Ron departure dependency. |
| 3 | `SLEEP 1.0` | Track-local delay. |
| 4 | `MOVETO GeorgePath1` | Waits for `4Lev_Tut1.Tut1George3`. |
| 5 | `MOVETO AllPath` | Waits for `5Lev_Tut1.Tut1George3`. |
| 6 | `MOVETO GeorgeLoc` | Waits for `6Lev_Tut1.Tut1George3`. |
| 7 | `FACE HPLoc2` | Final direct rotation. |

George emits no explicit cue. Neither Harry nor Fred waits for any of
George's automatic movement cues. A George-only movement stall therefore
cannot prevent Harry from emitting `FredTalkAgain`, cannot prevent Fred from
emitting `CutEnd`, and cannot by itself keep player input captured.

It can keep `CutScene52.bPlaying` true because `runActors` calls
`FinishPlaying` only after every cast track finishes its 40 slots. That is
separate from input release: Harry's explicit index-21 `RELEASE` clears
cutscene mode as soon as Fred emits `CutEnd`, even if George's track remains
waiting.

### Camera, cast 3

| Index | Authored operation | Dependency/effect |
| ---: | --- | --- |
| 0 | `CAPTURE` | Enters the camera cut state. |
| 2–6 | preface Fred, teleport to `CamStart`, delay, preface `Target2`, teleport to `2ndCam` | Establishes the first view. Teleports have no callback. |
| 8 | `WAITFOR RonCam` | Waits for Harry's index-5 cue. |
| 9–11 | preface Ron, set camera speeds, `MOVETO CamStart` | The move waits for `11Lev_Tut1.PotCam0`. |
| 13–14 | `WAITFOR RonGone`, then `TURNTO Fred` | Camera turn changes its direction actor directly; it does not wait for a turn callback. |
| 16–18 | `WAITFOR CameraFollow`, restore normal camera speeds, `RELEASE` | Harry's index-10 cue releases the first camera phase. |
| 20–24 | `WAITFOR CameraChange`, `CAPTURE`, set final speeds, preface `Target`, `MOVETO NextRoom` | The move waits for `24Lev_Tut1.PotCam0`. |
| 25–26 | delay one second, then `MOVETO 2ndCutScene` | The move waits for `26Lev_Tut1.PotCam0`. |
| 28–29 | `WAITFOR CutEnd`, then `RELEASE` | Fred's index-27 cue releases the final camera phase. |

The explicit cue graph around the failure is therefore:

```text
Ron --RonGone--> Harry, Fred, George, camera
Harry --CameraFollow/CameraChange--> camera
Harry --FredTalkAgain--> Fred
Fred --CutEnd--> Harry, camera
```

George has no outgoing edge in this graph.

### Automatic movement cue spelling

Compiled `CutScene.handleCast` constructs a movement callback by concatenating
the decimal action index with UnrealScript's object-to-string result. UE1
object conversion is `Package.Object`, as confirmed by the licensed reference
evaluator
([`ExpressionEvaluator.cpp`](../../SurrealEngine/SurrealEngine/VM/ExpressionEvaluator.cpp#L581-L585)).
There is no separator.

`CutScene.CutCue` uppercases a returned cue before recording it. The
waiting-track preamble compares recorded old cues case-insensitively, so a
returned `13LEV_TUT1.TUT1FRED2` still releases the track waiting for the
mixed-case generated form on its next scene tick.

The compiled-data dependency chain is:

1. Harry is captured, walks to the first meeting position, and cues Fred to
   speak.
2. Fred delivers the first two lines and cues the rest of the cast to leave.
3. Ron emits `RonGone`; the four relevant tracks continue independently.
4. Harry reaches `HPLoc2`, faces the final target, emits `FredTalkAgain`, and
   waits for `CutEnd`.
5. Fred must finish his own route, observe `FredTalkAgain`, deliver three
   lines, and emit `CutEnd`.
6. `CutEnd` lets Harry and the camera release. George is not consulted.

Harry's `HPLoc2` is export 817, `CutMark30`, at
`(-33.773, -4166.632, 61)`. This exactly matches the player location in the
reported stalled scene. At that point, no player input is expected:
`CAPTURE` has placed Harry in his cutscene idle state and set the HUD's
cutscene mode, while his track is deliberately waiting for Fred's `CutEnd`.

There is no independent timeout on the scene or on `WAITFOR CutEnd`. If Fred's
track never emits that cue, Harry and the camera remain captured indefinitely
while ordinary music, animation, and actor ticks may continue.

## Fred's route to the bookshelf

Fred's serialized command track uses these destinations:

| Step | Alias | Map object | Location | Nominal timeout |
| --- | --- | --- | --- | ---: |
| start | — | `Tut1Fred2` | `(169.040, -3037.473, 90.017)` | — |
| 1 | `FredPath1` | export 771, `CutMark3` | `(154.225, -3176.143, 64)` | 1.697 s |
| 2 | `FredPath2` | export 991, `CutMark4` | `(224.313, -3174.436, 64)` | 1.351 s |
| 3 | `FredPath3` | export 1391, `CutMark29` | `(163.072, -3289.440, 101)` | 1.651 s |
| 4 | `AllPath` | export 837, `CutMark7` | `(221.923, -3955.423, 64)` | 4.343 s |
| 5 | `FredLoc` | export 836, `CutMark8` | `(-109.072, -4254.493, 82.026)` | 3.230 s |

After the first `FredPath2` movement, the track sleeps for one second, repeats
that destination, and then continues through the remaining points. The
timeout values above use the exact compiled formula and planar distances;
vertical differences do not contribute.

George waits for the same Ron departure cue, sleeps for one second, then
moves through `GeorgePath1` (export 976), `AllPath`, and `GeorgeLoc`
(export 996). George's completion does not emit `CutEnd`; Fred is the cast
member that releases the stalled dependency.

## The rotating door

The obstacle on Fred's route is map export 1824, `Mover33`:

- base location `(224, -3328, 112)`;
- base yaw `16384`, with key-one yaw offset `-15360`;
- `MoveTime = 2.0`;
- initial state `TriggerToggle`;
- tag `FGsec1`;
- mover encroachment mode serialized as value 3;
- brush collision radius 96 and height 64.

Two one-shot `Trigger` actors address the same `FGsec1` event:

- export 1738, `Trigger7`, at `(224.220, -3181.844, 88)`;
- export 1702, `Trigger0`, at `(225.557, -3560.214, 88)`.

Fred's `FredPath3` is immediately north of the mover and `AllPath` is south of
it, so the long fourth movement crosses the rotating brush. The door can
legitimately block or encroach on Fred; the cutscene movement code was
authored with a timeout fallback specifically so a blocked pawn does not hold
the scene forever.

For reference, the licensed SurrealEngine implementation advances movers by
their interpolation rate, base transform, and selected position/rotation key,
then sends `InterpolateEnd` on completion
([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1240-L1300)).

## Why the blocked movement could outlive its timeout

Compiled `baseChar.Bump` permits ambient bump dialogue only for a collision
with Harry. Before starting that dialogue, it checks the cutscene flag through
`baseHud(playerHarry.myHUD)` and returns when `bCutSceneMode` is true. The
dialogue helper enters the character's `Emoting` state and later restores the
state that was active before the emote.

In the failing replay, `harry0.myHUD` was null. The context expression
therefore produced the default false value for the cutscene guard. While Fred
and Harry crossed the narrow rotating doorway together, their repeated
collisions incorrectly moved Fred from `CutMovingTo` into `Emoting` and back.
The state-local movement tick described below runs only in `CutMovingTo`, so
each ambient emote suspended the authored recovery timer. Repeated collisions
could stretch a roughly four-second fallback into a long pause or continually
starve it.

The shared correction is for the local game host to restore the missing first
render bootstrap immediately after possessing Harry, using the existing
`HUDType` and actor-spawn path. It is not a special collision exemption, Fred
teleport, or forced cutscene release. With the real `HPHud` instance present,
`CAPTURE` sets the original flag and the existing `baseChar.Bump` guard
suppresses those ambient emotes.

## Compiled `CutMovingTo` behavior

Fred is a `baseChar`, so his movement uses these compiled `HPBase.u` objects:

- `baseChar.SetupMoveTo`, export 2580;
- `baseChar.CutMoveTo`, export 2582;
- `baseChar.CutMovingTo.Tick`, export 2577;
- `baseChar.CutMovingTo`, state export 2578.

`CutMoveTo` stores the destination, callback script, and callback cue, calls
`SetupMoveTo`, then enters `CutMovingTo`. A missing destination is handled by
immediately returning the cue rather than leaving the caller waiting.

`SetupMoveTo` flattens the destination to the pawn's current Z for its
distance calculation, sets:

```text
timeout = planar_distance / GroundSpeed + 1 second
```

and clears `bCutArrived`.

The state enables its `Tick` event, selects walking physics and the authored
walk loop, then retains its state frame in a loop that sleeps for 0.1 seconds
while `bCutArrived` is false. The state-local `Tick`:

1. calls the global character tick;
2. derives a planar heading toward the current destination;
3. updates `DesiredRotation`;
4. attempts the frame's movement with `MoveSmooth`, including the authored
   small step-up and step-down probes;
5. marks arrival when the remaining step fits in the current frame;
6. decrements `CutMoveToTimeout` on every tick;
7. on timeout, calls `SetLocation` with the destination, steps down 100 units,
   and marks arrival.

Once `bCutArrived` becomes true, the retained state frame calls
`CutScene.CutCue`, clears its callback fields, and enters `CutIdle`.

The compiled code does not condition the timeout on successful movement. A
completely stationary or door-blocked Fred must still expire the timer and
return the cue. It also does not branch on `SetLocation`'s Boolean result:
even if collision rejects the relocation, it still marks arrival and returns
the cue. SurrealEngine's native reference confirms that `SetLocation` can
return false when its placement check fails
([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1366-L1408)).

Therefore, a visible Fred left in the doorway is not itself sufficient to
stall the authored scene. Failure to advance after the nominal timeout means
the state-local `Tick`, retained latent state frame, `bCutArrived` update, or
callback cue path did not execute correctly.

## `CutScene` movement handshake

Compiled `CutScene.handleCast` (export 3563) implements each `MOVETO` as a
callback handshake:

1. resolve the target alias;
2. build an automatic cue from the command index and cast actor identity;
3. call the actor's `CutMoveTo(target, scene, cue)`;
4. set that cast track to waiting for the same cue.

Compiled `CutScene.CutCue` (export 3579) records the cue and clears matching
waiting tracks. `runActors` (export 3582) does not advance a waiting track
until that happens. It finishes the scene only after all active cast tracks
finish their command arrays.

For this failure, the smallest faithful diagnostic boundary is consequently:

- Fred must be in state `CutMovingTo`;
- its state-local `Tick` must run while the retained state body sleeps;
- `CutMoveToTimeout` must decrease;
- timeout or physical arrival must set `bCutArrived`;
- state resumption must call `CutScene52.CutCue` with the exact automatic cue;
- Fred's track must eventually emit the explicit `CutEnd`.

Adding a cutscene-specific teleport, force-release, or collision exception
would duplicate behavior already present in the compiled game logic and hide
the shared runtime defect.

## `Lev3_Intro` fireplace closure

The fireplace dispatcher is map export 2481 (`Dispatcher0`, tag
`FireplaceFire`). Its second trigger closes the fireplace immediately, the
side covers after 0.5 seconds, the grate after another second, and the rear
wall after another second. Rear-wall `Mover6`, export 2443 with tag `FireBack`,
then takes 1.5 seconds to interpolate from its open key at
`(-1296,-6864,456)` to its closed base at `(-1296,-6688,456)`.

The 2026-08-14 report captured `Mover6` exactly at the open key while the
fireplace, grate, and both covers were already at their closed bases. That is
the authored dispatcher between its grate and `FireBack` steps. Offline
`CutScene3` replays at 10 through 60 ticks per second delivered the final event
and returned the wall and its vertices exactly to the closed base. This trace
does not justify a map timing override; a persistent failure needs a capture
after that final delayed dispatch rather than during its pending interval.

## Package evidence index

| Package | Compiled/serialized object | Evidence |
| --- | --- | --- |
| `Lev3_Intro.unr` | exports 2481, 2443, 3187, 3204 | Fireplace dispatcher, rear wall, and side-cover movers |
| `Lev_Tut1.unr` | export 2257, `CutScene52` | Cast bindings, location aliases, and every command track |
| `Lev_Tut1.unr` | export 602, `harry0` | Player actor identity and absence of a serialized `myHUD` override |
| `Lev_Tut1.unr` | exports 1328 and 1325 | Fred/George instances, locations, animations, and `GroundSpeed` |
| `Lev_Tut1.unr` | exports 771, 991, 1391, 837, 836, 976, 996 | Authored Fred and George route points |
| `Lev_Tut1.unr` | exports 1824, 1738, 1702 | Rotating mover and its two `FGsec1` triggers |
| `Engine.u` | exports 3292 and 3294 | `PlayerPawn` lazy `myHUD` spawn in the two render hooks |
| `HarryPotter.u` | export 175, `Harry.PostBeginPlay` | Concrete `HUDType = HPMenu.HPHud` assignment |
| `HPMenu.u` | class export 541 and function export 1750, `HPHud` | Concrete HUD class, superclass, and initialization hook |
| `HPBase.u` | class export 265, `CutScene` defaults | Start/play-once behavior |
| `HPBase.u` | exports 3563, 3579, 3582, 3586, 3596, 3600 | Command dispatch, HUD capture/release, cue release, scene tick, start, and finish |
| `HPBase.u` | exports 2580, 2582, 2577, 2578 | `baseChar` movement setup, tick, timeout fallback, and latent state body |
| `Tut1.u` | class exports 205 and 207 | George and Fred class defaults |
