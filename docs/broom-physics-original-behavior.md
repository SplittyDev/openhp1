# Original broom input and physics behavior

This note records the original PC game's interactive `BroomHarry` path from
shipped package data and the retail `Engine.dll`. It is intended to be the
source of truth for broom parity work in OpenHP1. It does not cover scripted
interpolation paths except where they replace interactive flight.

## Evidence standard

The conclusions below come first from the locally installed retail game:

| Artifact | SHA-256 | Relevant evidence |
| --- | --- | --- |
| `res/System/HarryPotter.u` | `5f18066ac7d6a64ba315a19753308613c0819b3944da551a17bd0f710560cf60` | `BroomHarry` source, bytecode, properties, defaults |
| `res/System/HPBase.u` | `0cec62e098ded3a16024ee15dbc982bf9662b443f630cd19890b7b5d325bf503` | `baseHarry` input declarations and `BaseCam` states |
| `res/System/Engine.u` | `b3661a1d2afb1f730ba5bb3cbd2a6716efaf55fd0d8cefa753b69026a8bc5a85` | `PlayerPawn.PlayerInput` script |
| `res/System/Tut2.u` | `ee53aed1c1cd1a65ac0edf399e14b4307b978900dffca04d377fcbdc1880d88b` | broom-practice launch setup |
| `res/System/Hub2.u` | `b44c845961a45d6b34577a59309c569c4c8236ec9ff7f7bb82526e7f499e39d1` | Remembrall referee, camera transitions, and loss flow |
| `res/System/HPMenu.u` | `42da2a2f43ac6a15ea87eace4ebd59a69bab7685cda854e9e7a86e7e6d9c6dbd` | configurable input-page labels and command aliases |
| `res/Maps/Lev2_RemChase.unr` | `5f1ce9f606b68b22acafb23a664e57feff85e21b2b18101f5aa1630aa112f3e6` | authored chase actors and initial camera type |
| `res/System/Engine.dll` | `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc` | native input, flying physics, collisions, and banking |

`Engine.dll` is the retail 32-bit x86 PE. Virtual addresses in this note assume
its image base of `0x10300000`.

The relevant compiled `HarryPotter.u` exports are:

| Export | Object | Raw bytecode size |
| ---: | --- | ---: |
| 1577 | `BroomHarry` class | n/a |
| 1569 | `BroomHarry.ScriptText` | n/a |
| 937 | `BroomHarry.PlayerInput` | 97 bytes |
| 804 | `BroomHarry.PlayerWalking` | n/a |
| 812 | `BroomHarry.PlayerWalking.PlayerMove` | 882 bytes |
| 820 | `BroomHarry.PlayerWalking.UpdateRotation` | 817 bytes |
| 831 | `BroomHarry.PlayerWalking.HitWall` | 462 bytes |

The decoded functions contain real control flow and native calls. Therefore the
matching embedded source is active behavior, not merely a stale source comment.
The source summaries below were checked against those compiled exports. Exact
reproduction commands are at the end of this note.

## End-to-end interactive tick

The confirmed path is:

1. Native `UInput::ReadInput` accumulates bound input properties and normalizes
   every float input property for the frame.
2. `BroomHarry.PlayerInput` calls `PlayerPawn.PlayerInput`, then handles broom
   action edges and tutorial-use statistics.
3. `BroomHarry.PlayerWalking.PlayerTick` calls only its `PlayerMove` override.
4. `PlayerMove` chooses keyboard or mouse steering, updates pitch/yaw, chooses
   `AirSpeed`, and sets a very large forward `Acceleration`.
5. Native `APawn::performPhysics(DeltaTime)` snapshots the old velocity, calls
   `physFlying` once with that same `DeltaTime`, then calls `physicsRotation`
   once with the old-velocity snapshot.
6. `physicsRotation` leaves the script-authored pitch/yaw alone but derives
   broom roll from the lateral velocity change produced by `physFlying`.
7. `BaseCam` follows the resulting pawn `Location`, `Rotation`, and
   `ViewRotation`; camera rotation does not determine thrust direction.

This ordering is conclusive. `APawn::performPhysics` is exported at RVA
`0x28f1` and enters at VA `0x103e5520`. It snapshots `Velocity` at
`0x103e554a..0x103e5561`, dispatches `physFlying` at `0x103e5597`, and calls
`physicsRotation` at `0x103e565d`.

## Input channels and stock bindings

`baseHarry` declares dedicated broom float axes `aBroomYaw` and `aBroomPitch`,
and byte buttons for yaw left/right, pitch up/down, boost, brake, and action.
It also declares global-config booleans `bInvertBroomPitch` and
`bAllowBroomMouse`. See `HPBase.u` embedded source lines 27156-27177 in the
`strings -a | nl -ba` view.

The shipped `DefUser.ini` bindings are:

| Input | Broom command |
| --- | --- |
| Arrow left/right | `bBroomYawLeft` / `bBroomYawRight` |
| Arrow up/down | `bBroomPitchUp` / `bBroomPitchDown` |
| Right mouse or `A` | `bBroomBoost` |
| Left mouse or `Z` | `bBroomBrake` |
| Jump alias (`Ctrl` by default) | `bBroomAction` |
| Mouse X | `Axis aBroomYaw Speed=6.0` |
| Mouse Y | `Axis aBroomPitch Speed=-6.0` |

These are at `res/System/DefUser.ini:15`, `:48-65`, `:87`, and `:108-121`.
`bInvertBroomPitch=False` is at line 239. There is no shipped config entry,
serialized true class default, script assignment, or map property occurrence
for `bAllowBroomMouse`; its stock value is therefore false. The mouse axes are
bound, but direct mouse broom steering is dormant unless a user or external
configuration enables it.

Those are installation defaults, not fixed retail controls. Compiled
`HPMenu.FEOptionsPage` defaults name rows 6 and 7 `Speed up` and `Slow down`,
and associate them with `button bBroomBoost` and `button bBroomBrake`.
`LoadExistingKeys()` enumerates `KEYNAME`/`KEYBINDING` for every input and
matches the command aliases; `SetKey()` rewrites the selected binding with
`SET Input`. Consequently an options page showing `Z`/`X` is conclusive
evidence that the active user configuration has rebound speed-up/slow-down to
those keys. It is not evidence that `DefUser.ini` shipped that way: the shipped
file binds speed-up to `A` and right mouse, slow-down to `Z` and left mouse,
and leaves `X` empty. The engine acts on the bound `bBroomBoost` and
`bBroomBrake` properties regardless of which keys produced them.

### Native axis scaling

`UInput::ReadInput` is export ordinal 1734/RVA `0x22c0` and enters at VA
`0x103a5ba0`. At `0x103a5de7` it loads `20.0` and divides it by frame
`DeltaTime`. At `0x103a5df8..0x103a5e45` it walks the collected input
properties, selects `UFloatProperty` values, and multiplies each by that factor.
This applies to all float input properties, including `aBroomYaw` and
`aBroomPitch`; it is not limited to the ordinary mouse axes.

`PlayerPawn.PlayerInput` subsequently applies `MouseSensitivity * FOVScale`
and smoothing only to `aMouseX/aMouseY` (`Engine.u` embedded source lines
8936-8982). `BroomHarry` reads the dedicated `aBroom*` axes, so broom mouse
steering does not inherit `PlayerPawn`'s mouse sensitivity or smoothing. It
does inherit native all-float input normalization, the binding's `Speed=6`,
and its own `fBroomSensitivity=1/20000`.

The exact conversion between modern window-event pixels and the retail input
device's raw units is not established by the binary. A host calibration factor
cannot be proven from package constants alone.

### `BroomHarry.PlayerInput`

The override first calls `Super.PlayerInput(DeltaTime)`. It then:

- turns `bBroomAction` into a rising-edge `bActioned` event and calls
  `Referee.OnActionKeyPressed()` once per press;
- records whether boost and brake have ever been used for tutorial hints.

It does not itself turn or accelerate the pawn. The shipped source is at
`HarryPotter.u` embedded source lines 43250-43265 and is backed by compiled
export 937.

## Scripted steering law

`PostBeginPlay` always selects `PHYS_Flying` and overwrites editor values with:

| Property | Runtime value |
| --- | ---: |
| `fRotationRateYaw` | 20,000 rotator units/s |
| `RotationRate.Pitch` | 24,000 |
| `RotationRate.Yaw` | 50,000 |
| `RotationRate.Roll` | 6,000 |
| `fBroomSensitivity` | 1/20,000 |

It also clears the mouse accumulators, deceleration, impact state, and wall
avoidance state. Evidence: `HarryPotter.u` embedded source lines 43188-43227
and the compiled `BroomHarry` defaults.

### Keyboard versus mouse

Keyboard steering is rate control. Pitch control is
`PitchUp - PitchDown`; yaw control is `YawRight - YawLeft`. Pitch is negated
when `bInvertBroomPitch` is true. Any active keyboard button takes authority
from the corresponding mouse accumulator and resets that accumulator.

When enabled, mouse steering is positional:

- pitch accumulates `aBroomPitch / 20000` (with the authored inversion rule),
  clamps to `[-1.5, 1.5]`, removes a `0.15` deadband, and then clamps the
  effective command to `[-1, 1]`;
- yaw accumulates `aBroomYaw / 20000`, clamps to `[-1.5, 1.5]`, and uses a
  wider `0.5` deadband with `0.3` subtracted after crossing it.

Evidence: compiled `PlayerMove` export 812 and embedded source lines
43604-43667; compiled `UpdateRotation` export 820 and lines 43709-43811.

### Pitch

The default pitch limits are 60 degrees up and down, or approximately
`+10922.67` and `-10922.67` in 16-bit rotator space. Keyboard pitch advances
at up to 24,000 units/s. With no keyboard pitch command it self-centers toward
zero at the same maximum rate. Mouse pitch directly selects an angle within
the limits instead of selecting an angular rate.

Positive pitch points the broom toward positive Unreal Z. `PlayerMove` calls
`GetAxes(Rotation,X,Y,Z)` after rotation and uses `X` as its forward direction;
the `Pull_Up` animation is selected for positive pitch.

### Yaw and wall override

Player yaw is script-authored at 20,000 units/s, not the 50,000 value in
`RotationRate.Yaw`. `UpdateRotation` updates `ViewRotation.Yaw`, copies it into
the actor rotation with `SetRotation`, and then assigns
`DesiredRotation = Rotation`.

If the player supplies no yaw while `bHittingWall` is set, the wall-avoidance
delta is converted into a yaw control value. The later multiplication by
`DeltaTime` cancels the division used to construct that value; the requested
avoidance step is therefore applied directly, subject to the `[-1,1]` command
clamp.

There is one ordering quirk: `UpdateRotation` runs before `PlayerMove` writes
this tick's forward acceleration. If `Acceleration` is still exactly zero,
script yaw is multiplied by `4/3` for that call.

## Speed and forward acceleration

Compiled class defaults establish:

| Property | Value |
| --- | ---: |
| `AirSpeedNormal` | 400 |
| `AirSpeedBoost` | 800 |
| inherited `AirSpeed` | 400 |
| inherited `AccelRate` | 1024 |
| inherited `AirControl` | 0.25 |
| `WallDamage` | 1 |

Every interactive `PlayerMove` runs rotation first, obtains the new forward
axis, and sets `Acceleration = 200000 * X`. Native `calcVelocity` limits that
enormous authored vector to the pawn's `AccelRate`; it is a direction command,
not a literal 200,000 units/s² acceleration. Thrust always follows the pawn's
new rotation. Camera direction does not feed this calculation.

The script speed policy is:

```text
if (boost or auxiliary boost) and not brake:
    AirSpeed = 800
else:
    if abs(yaw control) > 0.2 or brake:
        raise Deceleration toward 200 at nominal 80 units/s
    else if Deceleration > 160:
        reduce Deceleration at nominal 1000 units/s
    else:
        reduce Deceleration at nominal 80 units/s
    AirSpeed = 400 - Deceleration
```

Pitch alone does not slow the broom. Sustained turning or braking therefore
settles near speed 200. Boost does not erase stored deceleration, and brake wins
when boost and brake are held together. `Deceleration` is an `IntProperty`
(`HarryPotter.u` export 34), so the nominal float rates are quantized when each
compound assignment is stored; exact small-step results can depend on tick
size and the VM's numeric conversion.

The formula is in compiled export 812 and embedded source lines 43668-43691.

### Authored launch ramp

`Tut2.BroomPracticeReferee.GameTrial` explicitly starts the player at
`AirSpeed=10` and `Deceleration=AirSpeedNormal-AirSpeed`, hence 390. The same
10/390 setup appears in the Remembrall chase and Quidditch intro paths in
`Hub2.u`. On straight release, the high-decay branch rapidly sheds
deceleration until 160, after which recovery is much slower. Holding yaw can
retain the intentionally low launch speed because deceleration already exceeds
the ordinary turning target.

The tutorial assignment is in `Tut2.u` embedded source lines 603-604.

## Native `PHYS_Flying`

`APawn::physFlying` is export RVA `0x1ffa` and enters at VA `0x103f13a0`.
It calls `calcVelocity` at `0x103f14ae`, using normalized acceleration, frame
delta, pawn `AirSpeed`, and zone fluid friction. `calcVelocity` is export RVA
`0x2365` and enters at VA `0x103eb3e0`.

For the broom path, the confirmed native behavior is:

- clamp/normalize the acceleration command to `AccelRate`;
- steer existing velocity toward the commanded forward direction using zone
  fluid friction;
- integrate acceleration and cap a `PlayerPawn` to `AirSpeed`;
- move with swept actor collision;
- recompute all three velocity components from
  `(Location - OldLocation) / DeltaTime` when not teleported.

The last point is visible at `0x103f19b3..0x103f1a16`; retail flying does not
unconditionally zero vertical velocity.

### Collision slide

The retail collision sequence is also explicit in `physFlying`:

1. the first blocked move invokes `processHitWall` at `0x103f17fa` and projects
   the remaining displacement onto the first hit plane;
2. a second blocked move invokes `processHitWall` again at `0x103f1933`;
3. it calls `AActor::TwoWallAdjust` at `0x103f194e` to resolve the two-plane
   corner;
4. it attempts the adjusted third move at `0x103f19a0`.

`TwoWallAdjust` is therefore part of normal broom collision response, not only
falling physics.

## Native broom banking

`APawn::physicsRotation` is export RVA `0x27a7` and enters at VA
`0x103e5950`. The `DesiredRotation=Rotation` assignment prevents native pitch
and yaw from undoing script steering, but it does **not** disable the function's
separate roll block.

For a non-walking pawn with positive `RotationRate.Roll`, the binary does the
following:

1. Compute actual frame acceleration as
   `(post-physics Velocity - pre-physics Velocity) / DeltaTime`.
2. If acceleration magnitude squared is greater than 10,000, transform it into
   pawn-local axes and use the local Y/lateral component.
3. Form a bank target of approximately
   `local_lateral_acceleration * 28000 / AccelRate` and clamp it to
   `+/-RotationRate.Roll`.
4. Blend signed roll toward that target with
   `alpha = min(1, 5 * DeltaTime)`.
5. At or below the 10,000 threshold, blend signed roll directly toward zero with
   `alpha = min(1, 8 * DeltaTime)`.

Positive pawn-local Y acceleration produces positive encoded UE1 roll;
negative produces negative roll. Target formation and both blends use x87
`FISTP` under the engine's active FPU rounding mode, rather than a `fixedTurn`.

The threshold constant is read at VA `0x10473810`, the high-acceleration blend
constant 5 at `0x10476584`, the low-acceleration constant 8 at `0x104737e8`,
and the bank scale 28,000 is an immediate at `0x103e5b48`. The broom defaults
make the target clamp `+/-6000` and the divisor 1024.

This banking response is a major part of broom feel: script yaw changes the
forward acceleration direction, `physFlying` curves velocity toward it, and
`physicsRotation` turns that actual lateral velocity change into visible roll.

## `BroomHarry.HitWall`

The script event layered on native collision:

- ignores almost perfectly flat overhead ceilings (`HitNormal.Z < -0.9999`);
- gates the initial impact sound, damage, bump animation, and referee event
  with `bHitWall` until `AnimEnd` clears it;
- scales impact sound and integer damage by `VSize(Velocity)/AirSpeedNormal`;
- applies no automatic avoidance to floor-like surfaces with
  `abs(HitNormal.Z) >= 0.985`;
- otherwise finds the wall tangent and chooses an escape direction, preserving
  the previous choice for one second to avoid corner oscillation;
- caps the requested avoidance delta at `+/-0x6000` (135 degrees).

The literal offset from the wall tangent is 1000 rotator units, approximately
5.49 degrees. A source comment calls it 10 degrees, but the compiled constant
wins. Evidence: compiled export 831 and embedded source lines 43813-43885.

`Lev_Tut2.unr:BroomHarry0` (export 74) overrides `WallDamage=0`, `Physics=4`,
`CollisionHeight=37`, and `bAlignBottom=true`. Tutorial wall hits must therefore
still bump, sound, notify the referee, slide, and trigger avoidance, but not
reduce health. No other broom map actor among the 14 maps containing
`BroomHarry` overrides pitch limits, rotation rates, or normal/boost speeds.

## Camera and non-interactive states

`Harry.PostBeginPlay` establishes an `HPBase.BaseCam` view and creates a
`CamTarget` used as `StandardTarget`. `Lev2_RemChase.unr` serializes
`BaseCam0` (export 683) with `CameraType` byte 1. The compiled enum establishes
byte 1 as `CAM_Quiditch`, so `BaseCam.SetCamera()` selects `QuidditchState`;
this state is not inferred merely from the map's theme.
`BaseCam.PostBeginPlayIP` initially sets `CameraDistance=80`, `CameraSpeed=2`,
`CameraRotSpeed=10`, and `RealCameraDistance=80` before calling `SetCamera()`.

`QuidditchState.BeginState` (compiled export 2521) then establishes:

- `StandardTarget.TargetOffset=(100,0,50)`;
- `CameraHeight=60`;
- `CameraDistance=150`;
- `CameraAimOffsetState=(0,0,0)`.

The compiled `QuidditchState.PositionCamera` is materially different from a
plausible reading of the embedded source. Export 2019 always calls, in order,
`GeneralStationaryModeCamera(DeltaTime)`, `CheckCollisionState(DeltaTime)`, and
`SetCollisionState()`. It contains no call to `GeneralMoveModeCamera`. The
compiled `FarState.PositionCamera` export 2880 provides the control comparison:
its moving branch calls name-table entry 298 (`GeneralMoveModeCamera`) and its
other branch calls entry 113 (`GeneralStationaryModeCamera`); Quidditch export
2019 calls only entry 113. Therefore the apparent moving/stationary branch in
the embedded Quidditch source is stale. In particular, the 16-position history
and `trackingDistance` are not on the active normal Remembrall camera path.

For a moving broom, the active camera goal from compiled
`GeneralStationaryModeCamera` export 3106 is:

```text
elevated = p.Location
elevated.Z += p.TargetEyeHeight + CameraHeight
elevated.Z -= 10                         # while p.bStationary is false
direction = Normal(elevated - p.StandardTarget.Location)
goal = p.Location
     + direction * RealCameraDistance
     + (CameraOffset >> locRot)
trackingPoint = goal - camera.Location
```

`CameraOffset` is zero in the chase map/default path. Normal Quidditch movement
then calls `throttleTrack`, which scales `trackingPoint` by
`min(1, CameraSpeed*DeltaTime)`, hence `min(1, 2*DeltaTime)`, and passes that
delta to `MoveSmooth`. `smoothRotate` separately advances camera yaw toward
`p.ViewRotation.Yaw` by `10*DeltaTime` times the yaw difference, clamped to
`+/-1024` rotator units per call; it copies roll but does not make player thrust
follow the camera.

Compiled `CamTarget.seeking.setTarget` export 2097 computes its normal desired
point as `p.Location + (TargetOffset >> p.ViewRotation)`. The bytecode contains
only that rotated-offset expression, not the embedded source's apparent
velocity-prediction expression. It moves only the portion of the error beyond
30 units and therefore deliberately retains at most 30 units of target lag.

Compiled `CheckCollisionState` export 2272 is only 23 raw bytes. It may clear
`bCollide` when `CameraCanSee(p)` fails, but then unconditionally assigns
`RealCameraDistance=CameraDistance`. The embedded source's apparent temporary
30-unit distance and four-second recovery are not compiled. `SetCollisionState`
then disables camera collision. Thus the normal chase's target distance remains
150 after the first positioning update; that first update can still use the
initial `RealCameraDistance=80` before `CheckCollisionState` synchronizes it to
the state's 150. Retail does not intentionally collapse it toward Harry because
of a wall.

The camera follows the pawn after movement and does not supply the `Rotation`
used by `GetAxes` for thrust. A parity implementation must reproduce the live
target, fixed 150-unit goal, and the authored tracking filter. Substituting the
unused historical `GeneralMoveModeCamera` path can leave the camera following
an old pawn position and can put a fast broom at or behind the camera.

Special broom states intentionally replace this view:

- `Catching` switches to `ReverseState`, offset `(-100,10,50)`, with target
  pitch 5000;
- `BroomDying` switches to `TopDownState` and uses `PHYS_Falling`. Top-down
  export 2120 tracks `p.Location+(0,0,200)` through the same
  `min(1,2*DeltaTime)` filter; its begin state sets target offset `(25,0,0)`,
  camera height 200, and collision off;
- scripted `FlyingOnPath`/catch transitions use `PHYS_None`, disable collision,
  and delegate movement to an `InterpolationManager`.

The Remembrall referee has one deliberate close-camera exception. During
`GameBump` it switches to `StandardState`, distance 50, height 100, target
offset `(100,10,50)`, and target rotation `(0x1000,0x1000,0)`. Its ten-second
timer restores `QuidditchState`, distance 150, height 60, and offset
`(100,0,50)`. Normal chase flight is QuidditchState, while this short bump-Draco
prompt is legitimately closer.

No direct `Tut2.u` call selecting `QuidditchState` was found. The exact authored
cutscene or map action that establishes the first broom lesson's camera state
remains unresolved; it must not be assumed from the class name alone.

### Actor-local camera ordering and visual jitter

The two reported maps serialize the same `CAM_Quiditch` camera type, but their
relevant actors occupy opposite positions in the retail `ULevel::Actors`
array. Decoding the shipped `Level` exports gives:

| Map | Earlier actor slot | Later actor slot |
| --- | --- | --- |
| `Lev2_RemChase.unr` | 615: `BaseCam0` (export 683) | 616: `BroomHarry0` (export 692) |
| `Lev_Tut2.unr` | 270: `BroomHarry0` (export 74) | 271: `BaseCam0` (export 77) |

`Harry.PostBeginPlay` calls `makeCamTarget()`; compiled `Harry.makeCamTarget`
export 675 spawns the `CamTarget` used as `StandardTarget`. The target's
compiled `seeking.Tick` export 1830 calls `setTarget` every tick. It is therefore a
dynamically spawned actor later than both authored actors in these startup
paths.

Retail tick ordering is actor-local rather than phase-global. `AActor::Tick`
body `0x103b3840` advances actor animation at approximately
`0x103b39e1..0x103b3d7a`. Its normal local path dispatches `PlayerInput` at
`0x103b4159..0x103b417c`, dispatches `PlayerTick` at
`0x103b417f..0x103b419b` for a player, or ordinary `Tick` at
`0x103b4217..0x103b423a` for a normal actor. These branches are mutually
exclusive for the active local player: the player path jumps directly to
virtual `ProcessState` at `0x103b4248..0x103b424d`, bypassing ordinary `Tick`.
A `PlayerPawn` without the local player object can instead take the ordinary
actor branch. The shared path then advances `TimerRate`/`TimerCounter` and may
dispatch `Timer` at `0x103b4250..0x103b42bb`, decrements `LifeSpan` and may
dispatch `Expired` at `0x103b42bd..0x103b4306`, and only then calls virtual
`performPhysics` at `0x103b4331..0x103b433c` (or the alternate branch
`0x103b4344..0x103b434c`). Only after that actor returns does `ULevel::Tick`
body `0x103b6db0` advance to the next ascending actor slot; the loop and virtual
tick call are at `0x103b7177..0x103b71a2`.

This ordering gives the maps deliberately different one-frame relationships:

- Remembrall's camera runs before Harry, so it uses the previous completed
  Harry position. Harry then completes script, state, and flying physics, and
  the later spawned `CamTarget` observes that post-physics position.
- Tutorial Harry completes physics first; the following camera observes his
  current position, and the spawned target then observes the same position.

Before the actor-order correction, OpenHP1 preserved ascending actor order only
within global event, state, and physics phases. Consequently the spawned
`CamTarget` always observed the pre-physics Harry position. In Remembrall the
camera already used the preceding position by authored actor order, while its
target was one additional physics update older. That changed
`Normal(elevated_player_position - StandardTarget.Location)` precisely during
translation and turns, producing an incorrect camera goal even when every
camera constant and bytecode instruction is otherwise correct. Since Harry is
rendered relative to that moving view, this can also present as small Harry
screen-space jumps rather than an obvious camera-only error.

OpenHP1 now executes each actor's event/player, state/latent, timer/lifespan,
and automatic-physics work together before advancing to the next actor. This
restores the post-physics target sample used by retail without camera-distance
tuning or render-only interpolation. Animation advancement remains a shared
pre-pass; the separate animation symptom below is a coordinate-space defect,
not evidence that animation advancement must move into the actor-local pass.

### Skeletal tween coordinate space

Retail does not freeze tween-source vertices in render/world space.
`USkeletalMesh::ApplyAnim` is export ordinal 935 (thunk RVA `0x46ba`, body VA
`0x1041ba60`). Every call rebuilds the actor's current mesh coordinates through
`GetMeshCoords` at `0x1041bc57..0x1041bc79`. The tween path derives its blend
factor from the actor animation fields at `0x1041befc..0x1041bf21`, blends the
previous and new **bone transforms** (quaternion slerp plus translation) at
`0x1041d40e..0x1041d520`, and stores that still-mesh-local bone result at
`0x1041d522..0x1041d540`.

`USkeletalMesh::GetFrame` with the vertex-count reference is export ordinal
1252 (thunk RVA `0x1983`, body VA `0x1041df50`). It calls `ApplyAnim` at
`0x1041dfa8..0x1041dfb1`, then composes the cached mesh/bone pose with the
caller's current render `FCoords` at `0x1041e002..0x1041e13d`. Only afterward
does it transform vertices into the output coordinate space, directly at
`0x1041e3a1..0x1041e40c` or through the weighted path at
`0x1041e4ca..0x1041e55b`. Actor translation or rotation therefore carries the
entire tween pose immediately; tween progress affects articulation, not how
much of the actor's world movement reaches the displayed mesh.

Before the correction, OpenHP1 captured `tween_from` from already transformed
render vertices. Location and rotation updates advanced the live animation
transform and displayed vertex buffer but left that source pose behind. During
the one-second broom steering tweens, the next animation sample consequently
lerped between an old world-space Harry and a current world-space Harry. The
visible actor lagged or snapped precisely while turning even when runtime
`Location` was smooth. Within OpenHP1's existing world-space tween
representation, the narrow equivalent of retail is to apply every actor
translation/rotation delta to `tween_from` and the cached world-space bone
positions as well as to the live animation transform.

## Remembrall death and restart

The chase has a complete authored restart path; a dead broom rider never enters
an endless floor-slide state in the retail scripts:

1. `BroomHarry.KillHarry` (compiled export 1104) enters `BroomDying`.
2. The `BroomDying` state (export 882) plays `Fall`, waits for the animation,
   calls `Referee.OnPlayerDying()`, loops `Hang`, changes the camera to
   `TopDownState`, selects `PHYS_Falling`, and starts a one-shot ten-second
   timer.
3. Native falling physics calls the state's compiled `Landed(HitNormal)` export
   1306 when the falling collision resolves against a walkable floor.
   `Landed` plays `Q_Harry_Crash`, calls `Referee.OnPlayersDeath()`, and cancels
   the timer. It does not wait for horizontal velocity to reach zero.
4. If no floor landing occurs, compiled `Timer` export 892 calls the same
   `OnPlayersDeath()` after ten seconds.
5. `RemembrallReferee.GamePlay.OnPlayersDeath` (compiled `Hub2.u` export 646)
   enters `GameLost`; state export 657 sleeps 0.5 seconds and calls
   `Level.Game.RestartGame()`.
6. Compiled `Engine.GameInfo.RestartGame` export 3999 calls
   `Level.ServerTravel("?Restart", false)`. Compiled `LevelInfo.ServerTravel`
   export 3666 stores `NextURL="?Restart"` and `bNextItems=false`, then calls
   `Game.ProcessServerTravel`. In a non-network game compiled export 5085 sets
   `Level.NextSwitchCountdown=0`; the native engine consumes that pending URL
   and reloads the current map.

`OnPlayerDying` is only the pre-landing notification here. RemembrallReferee
does not override it to restart the game. The landing event or watchdog invokes
the distinct plural `OnPlayersDeath`, and the referee owns the delayed restart.
The normal latency is therefore landing plus 0.5 seconds; if no landing is
reported, the watchdog path is about 10.5 seconds.

## OpenHP1 parity implications (inspection snapshot)

The following comparison describes OpenHP1 before the broom correctness fixes
made from this research. Line numbers may move during implementation.

### Conclusive mismatches

1. **PlayerPawn banking is skipped.**
   `crates/openhp1-runtime/src/world/physics/dynamics.rs` had a normal
   `PlayerPawn` early return in `tick_rotating` around lines 1243-1252. Retail
   always calls `physicsRotation` after `physFlying` when roll rate is positive,
   even when current and desired rotations otherwise match. This removes the
   original lateral-acceleration bank and roll relaxation from broom flight.

2. **The physics cadence is different.**
   `crates/openhp1-runtime/src/world/physics.rs:13,258-302` split every actor
   physics update into fixed 0.02-second steps, rerunning flying and rotation
   without rerunning `BroomHarry.PlayerMove`. Retail `APawn::performPhysics`
   calls each exactly once with the incoming actor tick delta. For frame deltas
   above 0.02, the fixed loop changes nonlinear fluid steering, acceleration
   integration/capping, collision order, integer script-versus-physics cadence,
   and roll blending.

3. **Flying corner resolution omits the retail two-wall adjustment.**
   `tick_flying` in
   `crates/openhp1-runtime/src/world/physics/dynamics.rs:637-694` projected a
   first slide and emitted a second hit, but did not apply the retail
   `TwoWallAdjust` result before the next move. This changes corners and can
   amplify the script's wall-avoidance behavior.

4. **Broom speed bindings are hard-coded instead of using active input
   bindings.** `crates/openhp1-game/src/app.rs` maps right mouse or Shift to
   boost and left mouse or Z to brake. Shift is not a shipped boost default,
   A is omitted, and X cannot become brake through the options binding shown to
   the player. The UI in `app/ui.rs` displays Z and X as static text while the
   runtime input remains right-mouse/Shift and left-mouse/Z. Retail instead
   resolves whatever keys currently bind `bBroomBoost`/`bBroomBrake`.

5. **Authored `?Restart` server travel is not consumed by the host.** OpenHP1
   implements `PlayerPawn.ClientTravel` as an `ActorAction::ClientTravel`, but
   has no corresponding action or game-loop consumer for the
   `LevelInfo.NextURL`/`NextSwitchCountdown` set by compiled `ServerTravel` and
   `ProcessServerTravel`. Consequently the Remembrall loss path can reach
   `GameInfo.RestartGame` and prepare `?Restart` without reloading the map.
   OpenHP1's `phys_landed` changes a still-falling pawn to `PHYS_Walking` and,
   correctly for an ordinary pawn landing, retains pawn velocity. In this
   failure case that retained velocity manifests exactly as the reported dead
   Harry sliding along the floor after `BroomDying.Landed` has canceled its
   watchdog timer.

### Confirmed matches or non-gaps

- OpenHP1 maps input into the dedicated `aBroom*` and button properties before
  invoking the shipped script (`world/actor/player.rs:647-665`).
- Its shared frame normalization of float mouse axes is structurally consistent
  with retail `UInput::ReadInput`; the additional desktop event calibration is
  a host adaptation whose exact factor remains unproven.
- `tick_flying` preserves all three recomputed velocity components, matching
  the retail binary rather than reference-engine code that zeroes flying Z.
- The script's upward-positive pitch convention and forward `GetAxes` direction
  are shared engine semantics, not a broom-only sign exception.

### Behavior that should remain authored

The following are not tuning knobs to replace with host heuristics:

- 20,000 script yaw rate, 24,000 pitch rate, and +/-60-degree pitch limits;
- 400 normal speed, 800 boost speed, and the two-stage stored deceleration law;
- the low-speed 10/390 tutorial and chase launch setup;
- `HitWall`'s impact gate, referee event, surface tests, and one-second avoidance
  direction memory;
- `BaseCam` state selection and offsets;
- `PHYS_None` interpolation and `PHYS_Falling` death transitions.

## Remaining unresolved points

- The retail input device's raw-unit-to-modern-window-pixel calibration is not
  recoverable from the axis normalization alone. It needs a controlled retail
  capture or an equivalent device-path reconstruction.
- The exact tutorial transition into a `BaseCam` state was not located in
  `Tut2.u`; authored cutscene/map actions need a separate trace if tutorial
  framing remains visibly wrong.
- After correcting actor-local tick ordering, a frame trace of
  `BaseCam.Location`, `BroomHarry.Location`, `StandardTarget.Location`,
  `p.bStationary`, and `RealCameraDistance` remains the appropriate validation
  for any residual Remembrall camera error; inventing a replacement follow
  distance is not justified.
- `Deceleration` is integer-backed. Its exact per-tick rounding should be tested
  through the VM conversion path if small frame-rate-dependent speed differences
  remain after native cadence is corrected.
- Live retail trajectories and camera recordings would still be valuable as an
  end-to-end validation, but are not needed to establish the formulas and call
  ordering above.

## Reproduction commands

These commands inspect the copyrighted local installation without modifying or
exporting it:

```sh
target/debug/examples/package_inspect res/System/HarryPotter.u
target/debug/examples/script_inspect res/System/HarryPotter.u 937
target/debug/examples/script_inspect res/System/HarryPotter.u 812
target/debug/examples/script_inspect res/System/HarryPotter.u 820
target/debug/examples/script_inspect res/System/HarryPotter.u 831
target/debug/examples/script_inspect res/System/HarryPotter.u 892
target/debug/examples/script_inspect res/System/HarryPotter.u 1104
target/debug/examples/script_inspect res/System/HarryPotter.u 1306
target/debug/examples/script_inspect res/System/HarryPotter.u 675
target/debug/examples/class_defaults res/System/HarryPotter.u 1577
target/debug/examples/property_inspect res/Maps/Lev_Tut2.unr 74

target/debug/examples/property_inspect res/Maps/Lev2_RemChase.unr 683
target/debug/examples/property_inspect res/Maps/Lev2_RemChase.unr 692
target/debug/examples/script_inspect res/System/HPBase.u 2019
target/debug/examples/script_inspect res/System/HPBase.u 2880
target/debug/examples/script_inspect res/System/HPBase.u 3106
target/debug/examples/script_inspect res/System/HPBase.u 2272
target/debug/examples/script_inspect res/System/HPBase.u 2097
target/debug/examples/script_inspect res/System/HPBase.u 1830
target/debug/examples/script_inspect res/System/HPBase.u 2120
target/debug/examples/script_inspect res/System/HPBase.u 2521
target/debug/examples/script_inspect res/System/HPBase.u 2787
target/debug/examples/script_inspect res/System/HPBase.u 3676
target/debug/examples/script_inspect res/System/HPBase.u 3681
target/debug/examples/class_defaults res/System/HPMenu.u 408
target/debug/examples/script_inspect res/System/HPMenu.u 1457
target/debug/examples/script_inspect res/System/HPMenu.u 1480

target/debug/examples/script_inspect res/System/Hub2.u 646
target/debug/examples/script_inspect res/System/Hub2.u 657
target/debug/examples/script_inspect res/System/Engine.u 3999
target/debug/examples/script_inspect res/System/Engine.u 3666
target/debug/examples/script_inspect res/System/Engine.u 5085

strings -a res/System/HarryPotter.u | nl -ba | sed -n '43037,44062p'
strings -a res/System/HPBase.u | nl -ba | sed -n '25203,25311p;27154,27178p;29242,29368p;29524,29866p;31212,31308p'
strings -a res/System/Engine.u | nl -ba | sed -n '8898,9031p'
strings -a res/System/Engine.u | nl -ba | sed -n '15500,15510p'
strings -a res/System/Engine.u | nl -ba | sed -n '14945,14985p;19563,19577p'
strings -a res/System/Hub2.u | nl -ba | sed -n '1480,1562p'
strings -a res/System/HPMenu.u | nl -ba | sed -n '194620,195329p'
nl -ba res/System/DefUser.ini | sed -n '1,125p;234,240p'

shasum -a 256 res/System/Hub2.u res/System/HPMenu.u res/Maps/Lev2_RemChase.unr

objdump -p res/System/Engine.dll | rg 'ReadInput|performPhysics|physFlying|calcVelocity|physicsRotation|TwoWallAdjust|processHitWall|processLanded|eventLanded|ServerTravel'
objdump -d -M intel --start-address=0x103e5520 --stop-address=0x103e56eb res/System/Engine.dll
objdump -d -M intel --start-address=0x103e5950 --stop-address=0x103e5e2b res/System/Engine.dll
objdump -d -M intel --start-address=0x103f13a0 --stop-address=0x103f1a30 res/System/Engine.dll
objdump -d -M intel --start-address=0x103a5ba0 --stop-address=0x103a5e60 res/System/Engine.dll
objdump -d -M intel --start-address=0x103b3840 --stop-address=0x103b4360 res/System/Engine.dll
objdump -d -M intel --start-address=0x103b6db0 --stop-address=0x103b71c0 res/System/Engine.dll
objdump -p res/System/Engine.dll | rg 'ApplyAnim@USkeletalMesh|GetMeshCoords@USkeletalMesh|GetFrame@USkeletalMesh'
objdump -d -M intel --start-address=0x1041aef0 --stop-address=0x1041b220 res/System/Engine.dll
objdump -d -M intel --start-address=0x1041ba60 --stop-address=0x1041d770 res/System/Engine.dll
objdump -d -M intel --start-address=0x1041df50 --stop-address=0x1041e610 res/System/Engine.dll
```

SurrealEngine was consulted only after the shipped artifacts, as a licensed
secondary cross-check. Its broad flying-velocity structure agrees with the
retail binary, but its unconditional flying-Z reset contradicts this retail
`Engine.dll`; the shipped binary is authoritative here.
