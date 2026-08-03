# UnrealScript runtime behavior

This document records durable runtime semantics that are easy to lose when
extending native calls, actor state, or animation actions.

## Module layout

`openhp1-runtime` keeps its public interface in `lib.rs`. The bytecode VM is
split into frame state, execution, opcode decoding, value operations, and
focused frame tests. `ScriptRuntime` remains the single owner of package-backed
world state; its implementation is grouped under `world/` by actor lifecycle,
script execution, instance decoding, collision/movement, native functions,
physics modes, and state lookup. Nested modules hold narrower responsibilities
such as scalar natives, spawning, sound, collision geometry, player state, and
physics callbacks. Preserve this ownership when adding behavior instead of
introducing parallel runtime objects.

The 100,000-step frame guard counts executable statements, not expression
tokens within a statement. This retains protection against runaway control flow
without rejecting finite iterator bodies merely because they have nested calls.

## Actor identity and state

Actors use stable package/export identities. Class defaults followed by actor
tagged-property overrides initialize persistent instance state. Remote actor
contexts must resolve registered actor handles so field reads, writes, and
calls affect the target actor rather than a temporary copy.
Signed `ObjectConst` package references are resolved relative to the function
or state package before context operations use their runtime handles.
`ClassContext` reads instance-variable expressions from the referenced class's
inherited default object. Function calls through a class object execute against
that same default object, including inherited static functions.

Nested remote calls may inspect or call back into their caller, so the caller's
live instance remains addressable while the remote context executes. Serialized
`UStruct` child chains may contain any `UField`; non-property fields are skipped
by following their shared `Next` link.

Runtime actions update both persistent actor state and the corresponding scene
state. In particular, later animation ticks must not undo `SetLocation` or
other transform changes. Direct `PrePivot` assignments also move rendered
geometry; HP1 uses this while temporarily shrinking Harry's mount collision.

## Native randomness

Random natives share the runtime's deterministic stream. `RotRand` draws yaw,
then pitch, from the full `0..=65535` Unreal angle range; its optional `bRoll`
defaults to false and draws roll from that same stream only when true.
`Sin`, `Cos`, and `Tan` take radians.
`vector * vector` multiplies the matching X, Y, and Z components.
`Cross_VectorVector` computes the ordinary `A × B` in Unreal coordinates;
render-coordinate conversion remains at the renderer boundary.

## State execution

Persistent state frames retain their decoded instruction pointer and local
values across latent `Sleep` and `FinishAnim` actions. `GotoState`,
`GotoLabel`, and `Stop` operate on that retained frame rather than restarting
the state body.

Entering a different state restores that state's authored event probe mask.
Runtime `Enable` and `Disable` changes last only for the current activation;
re-entering a state must not inherit an event disabled during an earlier visit.

Context latent calls suspend the caller's state while movement and animation
completion are polled on the actor that received the call.
`FinishInterpolation` resumes the retained frame when mover physics clears
`bInterpolating`.
`Pawn.StopWaiting` zeros only a receiving pawn's pending `Sleep` delay, so the
normal state tick resumes it without discarding its retained frame or locals.

Nested state execution restores the caller's active-state context. A
Dispatcher may therefore trigger another actor and then enter `Sleep` without
the nested state making that latent call appear to come from ordinary function
code.

Label lookup uses the final top-level `LabelTable` in canonical decoded
bytecode. Serialized state metadata offsets are not canonical decoded-byte
offsets.

Integer scalar shifts preserve UE1's 32-bit behavior: shift counts are masked
to five bits, left-shift results wrap, signed right shifts retain sign extension,
and logical right shifts operate on unsigned 32-bit values.
Native `SubtractEqual_IntInt` (`0x0a2`) stores and returns the wrapped 32-bit
difference.
Native `MultiplyEqual_IntFloat` (`0x09f`) stores its `f32` product back in its
integer target. `DivideEqual_IntFloat` (`0x0a0`) calculates from the original
integer value. Both truncate valid results toward zero and use the x86
integer-indefinite value (`-2147483648`) for non-finite or out-of-range
results. `DivideEqual_IntFloat` stores and returns zero for either floating-point
zero divisor.

## Animation actions

- `PlayAnim` and `LoopAnim` use the scene's existing animation path.
- Their native calls also update the persistent UE animation fields before the
  actor's next script tick. `AnimFrame` advances before `Tick`, including
  tweening and velocity-scaled rates, so authored transition guards observe
  the displayed animation state.
- Animation completion occurs at `AnimLast`, before the sampler wraps toward
  frame zero.
- Repeated `LoopAnim` calls preserve the current phase.
- Mesh animation notifications dispatch their named actor functions when
  forward playback crosses the authored normalized time, before the actor's
  next `Tick`.
- `FinishAnim` ends the current loop.
- Tween-time arguments blend from the displayed pose.
- HP1's `RootBone='Move'` argument extracts skeletal root translation from the
  rendered pose and applies it through UE1 smooth movement so a blocked forward
  component can slide upward during a mount.
- HP1's numeric `BonePos` native uses the current sampled skeletal bone origin
  after the mesh and actor transforms, in Unreal coordinates. Pose updates run
  before script ticks and preserve the displayed tween interpolation.
- HP1's native `FindPath` follows the level's serialized reach specifications,
  respecting pruned links and the pawn's collision size.
- `IsAnimating` reflects active `PlayAnim` and `LoopAnim` actions.

Unsupported actions should remain nonfatal actor diagnostics until their
subsystem exists; they must not silently claim successful behavior.

`TraceActors` traces colliding actors from its authored `Start` toward `End`,
orders hits from that start, and does not insert a BSP pseudo-actor. Starting
inside an actor does not report that existing overlap. `VisibleActors` skips
hidden actors and BSP-occluded locations; HP1's engine treats an omitted or
zero radius as unbounded, then its scripts apply their own distance filters.
`FastTrace` uses its required `TraceEnd` and optional `TraceStart` (defaulting
to the receiver location) for a zero-extent world-BSP trace; it ignores actors.
Qualified `TraceActors` and `VisibleActors` calls use the receiver's location
and collision context.
`Self` is resolved to the current actor at the common actor-call boundary
before arguments are consumed; a qualified call that passes `Self` retains
the caller's identity.

Runtime assignments to `bHidden` and `DrawType` both update scene visibility.
Runtime assignments are compared with the previous effective value before
scene work or capability reporting, including inherited and typed zero
defaults. Effective `DrawType`, `Mesh`, `Style`, `Skin`, and `SkelAnim`
changes reuse the ordinary actor assembly path. Replacement geometry is
appended while the previous bounded range is collapsed; the current animation
sequence and phase survive display-only rebuilding. Hot scalar state stays
in-place: `DrawScale` resizes mesh or sprite vertices and bounds,
`AmbientGlow` and `ScaleGlow` relight the current vertex range, and `Opacity`
updates the actor's blended materials. `Mesh=None` removes a weapon's
standalone geometry, while its `ThirdPersonMesh` remains independently
attached at the pawn transform, matching UE1's carried-weapon path.

Effective `LightBrightness` changes update movable actor vertex lighting and
rebuild only BSP lightmaps whose serialized light list references that actor.
The renderer uploads those changed atlas rectangles, including their filtering
gutters, without rebuilding the atlas.

Static `bMeshEnviroMap` uses camera-relative reflection coordinates. The
shipped `spellEcto` case supplies its actor `Texture`; ZoneInfo and LevelInfo
environment-map fallback remains intentionally unimplemented because no
shipped class or map authors either value. Effective but unsupported
`Texture`, `MultiSkins`, `bUnlit`, or dynamic `bMeshEnviroMap` assignments
remain deduplicated diagnostics. `ParticleFX` is excluded because its live
instance state is synchronized separately each frame.

## Particle effects

`ParticleFX` configuration remains live UnrealScript instance state. The scene
samples inherited and script-mutated emission, lifetime, source, size, speed,
gravity, texture, style, and unlit fields each frame. A zero `ParticlesMax`
means unlimited total emission; `ParticlesAlive` limits the live set, with the
maximum authored emission rate multiplied by maximum finite lifetime used when
that limit is zero. A particle lifetime of zero means that it remains alive
until its emitter is removed, matching UE1's native `UParticle::Update`
semantics. `ParticlesEmitted` is synchronized back into UnrealScript so the
original `Shutdown` logic can stop finite effects. Removing a `ParticleFX`
actor also removes its live particles. World-relative emitters
interpolate emission between locations, while `bSystemRelative` particles
remain attached to their moving system. `bVelocityRelative` adds the owner's
current velocity once when each particle is emitted. Authored size growth,
delay, and end scale, `DripTime`, and sprite `SpinRate` are applied over each
particle's lifetime. Particle velocity uses the authored exponential `Damping`
decay. A
`PPRIM_Liquid` particle (`RenderPrimitive=2`) uses a world-horizontal quad,
rather than a camera billboard, and spins about its vertical normal.
`Gesture` assigned to
`Pattern` places emissions along its authored point segments; `Period` selects
the active normalized range, which is how spell lessons progressively draw
their visible template. Authored particle modes that are not implemented are
retained as per-actor capability diagnostics rather than silently discarded.
Particle acceleration combines authored `Gravity` with the emitter's active
zone `ZoneGravity * GravityModifier`, falling back to `LevelInfo` when the BSP
zone has no actor. `WindModifier` samples active `Wind` actors, never
`ZoneInfo.ZoneVelocity`: a source first applies HP1's native
`distance_squared <= (WindRadius^2)^2` gate, then its `WindRadius^2` falloff
clamps its contribution to zero at the authored radius. BSP-blocked sources
require `bPermeating`, and their vectors sum. World-relative particles use the
emitter's once-per-tick sample; system-relative particles resample at each
particle's world position.
Current native `Fluc` values participate; time-evolving `WindFluctuation`
state is not yet simulated.
When `Damping * WindModifier > 0`, wind is the terminal-velocity term of HP1's
analytic exponential-damping integration; otherwise wind is omitted. A
nonpositive damping value uses the native ballistic gravity step. A nonzero
`Elasticity` point-traces that advance against
world BSP, stops at the trace fraction, and reflects the normal velocity by
the authored restitution; zero leaves particles non-colliding. `Chaos` applies
the native per-particle, delayed,
normalized cube-direction velocity impulse after integration and attraction;
the impulse is not scaled by frame time. OpenHP1 uses its deterministic
per-emitter random stream and explicitly starts each chaos timer at zero.
Native actor destruction marks `bDeleteMe` and dispatches the authored
`Destroyed` event before removing the actor, allowing effects such as HP1's
targeting reticle to destroy their child emitters.

## Movement and spawning

Walking physics advances when either horizontal velocity component is nonzero;
axis-aligned paths must not wait for `MoveTo` to time out.
`Actor.AutonomousPhysics` uses that same per-actor physics update and suppresses
the later scheduled physics pass for its actor in the current runtime tick.
An idle walking pawn still steps down to a reachable floor; a floor probe alone
must not leave a newly initialized or script-moved pawn suspended above it.
Latent `TurnTo` updates `DesiredRotation` toward `Focus` and resumes its state
frame once the yaw is within the UE1 arrival threshold.
Latent `TurnToward` tracks the target actor's current location while turning.
Unlike ordinary pawns, `PlayerPawn` rotation normally remains script-controlled;
generic `bRotateToDesired` physics must not turn Harry during cutscene movement.
For HP1 compatibility, native latent `TurnTo` and `TurnToward` are the exception:
their state code waits until yaw reaches the arrival threshold. The compiled
`Harry.Mounting` state blocks on `TurnTo`, so an off-angle ledge climb cannot
resume unless pawn rotation follows its `DesiredRotation`. This is not a claim
that every UE1 `PlayerPawn` movement action uses generic pawn rotation.
`MoveSmooth` first attempts the requested movement and then slides the
untraveled delta along the collision plane; it is not an alias for `Move`.
Walking players use the same wall-slide response for non-pushable actor
collisions as for BSP walls.
Actor collision honors HP1's `CollideType`: `CT_Box` uses the rotated
`CollisionRadius`, `CollisionWidth`, and `CollisionHeight` extents rather than
the default aligned cylinder, while `CT_Shape` uses the mesh's offset, rotated
primitive bounds. A sweep that starts inside an existing overlap may move out
instead of treating the exit surface as a new impact.
`GetWorldCollisionBox(true)` transforms the mesh's serialized primitive bounds
through its mesh and current actor transforms; the default form returns the
actor's collision bounds instead.
Movers participate in world collision through their transformed brush-model
hulls, including `PrePivot`, rotation, and non-uniform `MainScale`.
Pawn mounting follows HP1's native `APawn::Mount` path: only BSP surfaces with
the authored `bHighLedge` flag qualify, and the original raised, diagonal, and
destination cylinder probes must all pass before the pawn's `Mount` event runs.
The flag comes from a polygon trace because convex-hull clipping planes do not
necessarily carry the visible BSP surface's flags.
The horizontal probe offset uses the pawn's collision diameter, not its height.
Aligned cylinders sweep BSP box corners as rounded corners so the resulting
contact normal can slide a pawn through an adjacent opening.
`MakeNoise` currently validates its loudness without populating pawn noise
slots or dispatching `HearNoise`.

Runtime-spawned actors use the same class-default mesh, material, lighting, and
animation assembly as actors serialized in the map. Adding their geometry may
grow the scene topology, so render consumers reload their GPU scene resources
when an in-place vertex update no longer fits.
Local player setup lazily spawns the concrete class in `PlayerPawn.HUDType` and
stores it in `myHUD`; authored HUD types may be `HPHud` subclasses such as
`QuidHud` or `BroomHud`, and a second initialization does not spawn another HUD.
Class-valued native arguments are runtime object handles and take precedence
over numerically overlapping serialized package references.
Spawning a collision-enabled actor at an occupied blocking location fails
without allocating an actor handle. Spawned pawns link themselves into
`Level.PawnList` through `nextPawn`, matching the native `AddPawn` bookkeeping
used during `PreBeginPlay`; native `RemovePawn` unlinks the same list during
`Destroyed`.
HP1's `VisibleCollidingActors` uses the cached collision actors for a sphere
query, defaults its location and radius to the receiver, filters class and
hidden state, and does not perform a line-of-sight trace.
`PlayerCanSeeMe` walks that list and succeeds when a non-self pawn is within
500 units, has the actor in its 75-degree `ViewRotation` cone (or uses
`bBehindView`), and has a clear BSP trace from its `BaseEyeHeight`.
`LineOfSightTo` applies the receiver's `SightRadius` and accepts a clear BSP
trace from its `BaseEyeHeight` to the target's center, half-height top, or
half-height bottom; unlike `CanSee`, it does not apply peripheral vision.
`Pawn.actorReachable` is a bounded, non-mutating collision simulation: it
rejects unsuitable or player-only navigation points using their authored,
unpruned reach-spec paths sized for the pawn, rejects water and pain zones the
pawn cannot enter, checks the destination against BSP using UE1's 3x3x3
nearby-location probe, then uses the ordinary movement sweeps and wall-slide
response for up to five walking, flying, or swimming probes. Walking reaches
its horizontal goal first, then makes its final vertical sweep only toward
gravity. It is not a line-of-sight query.
`PickAnyTarget` considers non-Pawn actors with `bProjTarget` set, while
`PickTarget` considers living Pawns. Both retain only the best non-negative
fire-direction dot product within 2,500 units, require the receiver's
`LineOfSightTo` visibility, and write its aim and distance back through their
output arguments.
`SetOwner` updates the persistent `Owner` reference and sends `LostChild` and
`GainedChild` to the old and new owners.
Engine side effects without an OpenHP1 surface do not abort scripts:
`SaveConfig` is read-only, `ConsoleCommand` returns an empty string, and decal
detachment is a no-op until decals render. `PlayerPawn.ClientTravel` emits a
host action with its URL, raw UE1 travel-type byte, and `bItems` flag; the
script runtime neither parses nor opens the next map. `PlayerPawn.UpdateURL`
emits a host action carrying the option/value for case-insensitive replacement
and an optional `User.DefaultPlayer` persistence request; the runtime never
mutates map package or configuration data. OpenHP1 is a local-only host, so
`PlayerPawn.GetPlayerNetworkAddress` intentionally returns an empty string
until a network host supplies an address.
`Pawn.CheckValidSkinPackage` accepts only a scanned, parseable local package
whose skin-package name is compatible with the requested mesh; it never treats
the requested name as an arbitrary filesystem path.

Cutscene cameras use UE1 vector/rotator transforms, `Trace`, and pawn visibility
tests. BSP trace hits return the active `LevelInfo`, as UE1 UnrealScript
expects; when `bTraceActors` is true, the closest actor or BSP hit wins.
`TraceActors` returns actor and BSP hits with their output locations and
normals.
In the runtime's Unreal coordinate representation, `vector >> rotator` turns
an authored local offset into world space and `vector << rotator` reverses that
transform. Camera scripts rely on this distinction for their follow offsets.
`WarpZoneInfo.Warp` and `UnWarp` apply the corresponding inverse coordinate
transforms to their location, velocity, and rotation output parameters.
Harry's authored `PostBeginPlay` selects `BaseCam`; the game does not override
that view target. Returning Harry to `PlayerWalking` is not the end of the
cutscene camera sequence: its later `ExitCutState` action restores the saved
third-person camera state and position.
Desktop input follows the original ground controls: W/S or up/down move,
A/D or left/right turn, right click/Control jump, and left click/Alt cast.
Keyboard axes use UE1's press delta of 20; mouse axes use its raw-motion delta
of 16, followed by the authored `Speed=6.0` and UE1's
`DeltaTime * 150` rate normalization. The held cast button is exposed through
the player input properties, while its press dispatches the original `AltFire`
exec so the active state owns sound, animation, and spell logic. `PlayerInput`
and `PlayerTick` run at the player's position in the actor tick order, so later
actors observe the current frame's processed mouse values as they do in UE1.
Captured mouse-button events always reach gameplay after egui observes them, so
releasing cast cannot leave the held `bAltFire` input stuck.
Holding + (main keyboard or numpad) or F in `openhp1-game` runs 16 ordinary
world, animation, player, trigger, camera, and audio-action ticks per rendered
frame. This debug fast-forward preserves event ordering and latent callbacks
rather than jumping runtime state; held movement/casting input repeats while
one-shot jump and mouse input do not.

Script `Name` comparisons treat a missing object/name value as UE's canonical
`None` name.
`MetaCast` returns a class object only when it is the requested class or derives
from it; non-class and incompatible objects become `None`.
Numeric natives interpret a null-context scalar result as the typed zero value
that UE writes into the expression result buffer.
Switches likewise compare an untyped null-context result as zero when their
case values establish a numeric or boolean type.

HP1 `CreateAnimChannel` creates the requested channel through the normal actor
spawn lifecycle so scripts can retain and own the returned object.
`PickTarget` selects visible living pawns within 2500 units using the authored
fire direction and updates its `bestAim` and `bestDist` output parameters.
`Object.Localize` reads case-insensitive section/key values from the package's
English `.int` file and returns an empty string when no entry is available.
Native class imports without serialized class exports remain opaque class
handles so `DynamicLoadObject` can resolve qualified resources such as sounds.
`GetSoundDuration` reads embedded WAV metadata or sums MPEG Layer II frames for
dialogue timing without requiring audio playback.
`Pawn.FindStairRotation` samples the current and forward floor through the
walking collision path. It selects `5400` for a rising forward floor and
`-5000` for a falling one, otherwise neutral. Delta times above `0.33` return
the current pitch; smaller values use UE1's capped interpolation, including
its faster final 1000 rotator units. Raw pitches above `0x8000` are stored back
as `(pitch & 0xffff) - 0x10000` before sampling.
HP1 `TraceTexture` validates its authored start/end/flags form but returns no
texture until BSP collision retains surface material identities. The runtime
tracks non-transient actor sound channels until their WAV or MP2 duration
expires. `ModifySound` returns true only for a live actor/slot channel,
optionally filters by sound (`None` is a wildcard), and changes volume, radius,
or pitch for parameter values 0, 1, or 2. Slot zero uses an allocated transient
channel and is not selectable by `ModifySound`.

The release `runtime_scan` advances both world and player scripts every frame
after `Possess`, matching the game loop closely enough to expose player-tick
deferrals during local corpus scans.
