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

Bytecode `0x60` is `ExtendedNative`/`HighNative0`, not a conversion: its next
byte is the low native-index byte, followed by expression arguments through
`EndFunctionParms`. UE1's `MaxConversion = 0x60` is an enum boundary sentinel;
the HP1 binary dispatches this opcode through its high-native handler.
HP1 bytecode can use `ByteToInt` (`0x3a`) for a boolean instance value; that
conversion uses the boolean's UE1 byte representation, zero or one.

## Actor identity and state

Actors use stable package/export identities. Class defaults followed by actor
tagged-property overrides initialize persistent instance state. Remote actor
contexts must resolve registered actor handles so field reads, writes, and
calls affect the target actor rather than a temporary copy.
The intrinsic `Object.Class` field returns that registered runtime class
identity, so authored class comparisons such as `spell.Class == class'spellFlip'`
use the same object handles as class constants.
Signed `ObjectConst` package references are resolved relative to the function
or state package before context operations use their runtime handles.
`ClassContext` reads instance-variable expressions from the referenced class's
inherited default object. Function calls through a class object execute against
that same default object, including inherited static functions.
`FinalFunction` bytecode executes its serialized function export directly,
bypassing virtual and state lookup; native exports use the normal native
dispatcher, and any target failure propagates through the calling frame.
Serialized `name` indices are package-local. Calls into another package convert
name arguments to their text identity before binding them; otherwise a call such
as Tut2's `TriggerEvent('Intro')` is interpreted as an unrelated Engine name.

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
Completing or cancelling latent `MoveTo` and `MoveToward` clears the movement
receiver's acceleration before the caller's resumed or replacement state code
runs. Context calls retain caller-owned state frames while the latent action
identifies the receiver, so a controller's own acceleration is not changed when
it drives another pawn.
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

## Timers

`Actor.SetTimer` accepts only finite positive rates; a non-positive rate clears
the timer. Timers run after physics in ascending actor order. A looping timer
dispatches every firing elapsed in a host tick, preserving its fractional phase.
Each firing advances the timer before its `Timer` callback, which is dispatched
immediately rather than batched. The callback can therefore reset or clear its
timer, switch it to a one-shot rate, or destroy its actor without stale firings.

## Animation actions

- `PlayAnim` and `LoopAnim` use the scene's existing animation path.
- A request for a sequence absent from the actor's mesh is a successful no-op:
  it does not change the persistent animation fields or report a renderer
  capability gap. Sequence metadata is retained for non-rendered actor meshes
  so this decision follows the authored asset rather than draw visibility.
  Metadata uses the same source as rendering: an explicit `SkelAnim`, otherwise
  the skeletal mesh's default animation, and otherwise legacy mesh sequences.
  Decode failures remain actor capability diagnostics instead of becoming an
  empty sequence list silently.
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
- HP1's native `FindPath` follows the level's serialized reach specifications
  while respecting pruned links. Unlike Pawn `FindPathTo`, this HP1-specific
  native does not reject authored links based on the pawn's collision size.
- Pawn `FindPathTo` clears authored navigation endpoint/cost state unless its
  optional `bClearPaths` is false. It sorts player-eligible navigation points
  within 500 units, checks only the nearest four with `FastTrace` from the
  requested destination to each node's eye height, and then uses the first
  visible candidate. It marks up to eight endpoints within 1000 units only
  when the full `ActorReachable` path accepts them, then searches reachspecs
  backward from the target while preserving pruned, collision-size, and
  player-only eligibility. The resulting route cache is passed through the
  first node's authored `SpecialHandling`; its `bCanDoSpecial` and
  `SpecialGoal` state control the returned next actor. If that handler selects
  an unreachable different actor, the guarded nested navigation lookup clears
  `RouteCache` and returns none while preserving `SpecialGoal`.
- `IsAnimating` reflects active `PlayAnim` and `LoopAnim` actions; its HP1
  root-bone overload resolves the parent's skeletal bone and reports that
  bone's animation channel instead.

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
that limit is zero. Unlimited zero-lifetime emitters grow their GPU particle
storage when full instead of silently stopping at their spawn-time capacity. A
particle lifetime of zero means that it remains alive until its emitter is
removed, matching UE1's native `UParticle::Update` semantics.
`ParticlesEmitted` is synchronized back into UnrealScript so the original
`Shutdown` logic can stop finite effects. Removing a `ParticleFX` actor also
removes its live particles. World-relative emitters
interpolate emission between locations, while `bSystemRelative` particles
remain attached to their moving system. `bVelocityRelative` adds the owner's
current velocity once when each particle is emitted. Authored size growth,
delay, and end scale, `DripTime`, and sprite `SpinRate` are applied over each
particle's lifetime. Particle velocity uses the authored exponential `Damping`
decay. `DIST_OwnerMesh` (`Distribution=2`) samples the owner's mesh surface,
including source geometry retained while `DrawType=None`; Lev_Tut2 uses that
combination to draw each training hoop entirely from particles. A
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
Zone `DamageType` and pawn `ReducedDamageType` accept serialized NameProperty
values, including `Name("None")`, so zone physics remains runnable.
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
`Actor.SetLocation` validates its finite target before changing persistent or
scene state. When `bCollideWorld` or `bCollideWhenPlacing` is set, the target
cylinder or box is checked at the target and its nearby UE1 placement grid;
failure to find a clear point returns false without a location action. Actor
occupancy does not reject this placement, but a successful placement updates
the cached collision location, emits its scene action, sends `Touch` only for
overlapping collidable non-based actors, and sends `UnTouch` for ended
contacts. Unlike swept `Move`, `SetLocation` does not carry based actors.
Actor collision honors HP1's `CollideType`: `CT_Box` uses the rotated
`CollisionRadius`, `CollisionWidth`, and `CollisionHeight` extents rather than
the default aligned cylinder. As in the original `Engine.dll`, a zero
`CollisionWidth` falls back to `CollisionRadius`. `CT_Shape` uses the mesh's
offset, rotated primitive bounds. A sweep that starts inside an existing
overlap may move out instead of treating the exit surface as a new impact.
In the original `res/System/Engine.dll`, `execSetRotation` calls
`ULevel::MoveActor` through vtable slot `0x8c` with a zero vector and proposed
rotator; `execSetLocation` uses FarMove slot `0x90`. OpenHP1 follows that
shared movement path: zero-delta BSP sweeps do not create a new BSP hit, while
rotated actor bounds use the ordinary blocking,
`Bump`, `Touch`, and `UnTouch` processing. A successful rotation updates the
persistent transform, collision index, and scene action; its based actors turn
around the base's yaw, and every `Pawn` subclass receives that yaw in
`ViewRotation`.
`GetWorldCollisionBox(true)` transforms the mesh's serialized primitive bounds
through its mesh and current actor transforms; the default form returns the
actor's collision bounds instead.
Movers participate in world collision through their transformed brush-model
hulls, including `PrePivot`, rotation, and non-uniform `MainScale`.
Normal movement continues to collide with an actor's own base so walking floor
probes stay supported; only movement imparted by that base ignores it.
Mover solidity comes only from the mutual `bBlockActors` or `bBlockPlayers`
flags. A real mover contact separately evaluates the mover's virtual
`IsRelevant` callback to decide whether to send `Bump`. This preserves the
authored `bProjTarget` path: `spellFlip` can pass through and activate a
GridMover once, while the same mover remains solid to Harry through its block
flags. Ordinary `BumpType` behavior remains in `Mover.IsRelevant` for
non-projectiles. Callback instance mutations and emitted actions are retained
in call order. A relevant non-blocking mover receives `Bump` at the swept
contact location before the other actor finishes crossing it, so scripts such
as `GridMover.Bump` observe the actual impact side. Collision-only probes such
as `test_move_actor` and `actorReachable` use only the physical blocking
predicate and do not execute `Mover.IsRelevant`; arbitrary virtual script can
mutate more than instance fields and therefore cannot be made observational by
discarding only its returned actions.

The shipped `GridMover.Bump` derives `KeyPos[1]` from `Location - BasePos`,
then applies `MoveIncrement` on the dominant impact axis, subtracting for a
positive offset and adding for a negative offset. It enters the `BumpMove`
`Move` label; that state calls `DoOpen`, waits in `FinishInterpolation`, and
only then completes the opening sequence. OpenHP1 therefore treats relevance
evaluation, the key-position update, state entry, and latent interpolation as
one ordered authored path rather than replacing it with a fixed destination.
Moving brushes with `bCollideWorld` are swept against static BSP using their
transformed primitive bounds with each extent reduced by 0.51 units, matching
the original `ULevel::MoveActor` path. This lets authored movers fit flush
against level geometry while a subsequent move into the wall stops immediately.
Before keyframe interpolation, the original `physMovingBrush` also integrates
the mover's velocity along `ZoneGravity` and moves it by that velocity plus
half the gravity acceleration for the tick. Any actual gravity displacement is
added to both `OldPos` and the active `KeyPos`, so the interpolation path follows
the falling brush. Supported movers remain fixed; a mover over an opening falls
until its transformed brush bounds reach the BSP below.
After a brush moves, blocking actor overlaps run the mover's synchronous
`EncroachingOn` event and restore its previous transform when the event returns
true; accepted overlaps receive `EncroachedBy`.
Pawn mounting follows HP1's native `APawn::Mount` path: the surface must have
the authored `bHighLedge` flag, whether it belongs to the level BSP or an
actor-owned brush such as a mover. The original raised, diagonal, and
destination cylinder probes query the whole collision scene before the pawn's
`Mount` event runs, and a successful mover mount bases the pawn on that mover.
The flag comes from a polygon trace through the primitive that produced the hit
because convex-hull clipping planes do not necessarily carry the visible
surface's flags.
The horizontal probe offset uses the pawn's collision diameter, not its height.
Aligned cylinders sweep BSP box corners as rounded corners so the resulting
contact normal can slide a pawn through an adjacent opening.
`MakeNoise` records its pawn instigator's two short-lived noise slots, coalesces
nearby equivalent noises, then synchronously dispatches `HearNoise` to linked
pawns that pass the original class, team, range, stimulus, and BSP-visibility
checks.

Runtime-spawned actors use the same class-default mesh, material, lighting, and
animation assembly as actors serialized in the map. Adding their geometry may
grow the scene topology, so render consumers reload their GPU scene resources
when an in-place vertex update no longer fits.
Local player setup lazily spawns the concrete class in `PlayerPawn.HUDType` and
stores it in `myHUD`; authored HUD types may be `HPHud` subclasses such as
`QuidHud` or `BroomHud`, and a second initialization does not spawn another HUD.
Class-valued native arguments are runtime object handles and take precedence
over numerically overlapping serialized package references.
`Actor.Spawn` runs a `FindSpot`-style BSP placement check before allocating an
actor handle when `bCollideWorld` or `bCollideWhenPlacing` is set. It uses the
class-default local collision extents, searches the adjacent axis and corner
spots, and ignores existing actors. The shared placement query uses a rotated
AABB for `CT_Box` and for `CT_Shape` once primitive bounds are registered;
cylinders and shapes without bounds use the shared cylinder query. Pre-allocation
Spawn has no scene-registered primitive bounds, so it currently takes the latter
path for `CT_Shape`; this is not claimed as HP1 Spawn parity. One-second
release replays attempted all 41 shipped maps (27 completed; 14 stopped in
pre-existing paths) and observed 1,927 Spawn invocations across 53 classes,
all `CT_None`, so the corpus has no exercised `CT_Shape` Spawn case. A found
spot
updates both `Location` and `OldLocation`. Spawned pawns link themselves into
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
`SetBase` maintains the reverse direct-base index used to carry attachments
during movement. It updates the old and new base's saturating `StandingCount`
before their `Detach` and `Attach` events, rejects self/descendant cycles, and
then sends the child's `BaseChange`. Destroying a base clears its direct
children through the same path after `Destroyed`. An actor's serialized
`Level` base is retained for base-chain reads but is never a direct attachment:
it receives no reverse child, `StandingCount`, `Attach`, or `Detach` update.
`SaveConfig` writes only `config` and `globalconfig` properties. Ordinary
`config` properties use the receiving class's `[Package.Class]` section;
`globalconfig` properties use their declaring class's section and config name.
Classes apply those values while constructing defaults, resolving a missing
derived `ClassConfigName` through its base class. `SaveConfig` refreshes cached
defaults afterwards, including cached derived classes that share a
`globalconfig` field.
The writable OpenHP1 settings directory holds the executable-named INI for
`System`, `User.ini` for `User`, and one INI per other declared config name.
`OPENHP1_SETTINGS_DIR` overrides the location; otherwise it is OpenHP1 under
macOS Application Support, Linux XDG config (or `~/.config`), or Windows
`APPDATA`.
Missing files are seeded from their read-only installed counterparts and,
respectively, `Default.ini` or `DefUser.ini`. Each update is atomic; package
files and all installed INIs remain read-only. Other engine side effects
without an OpenHP1 surface do not abort scripts: decal detachment is a no-op
until decals render.
Config serialization is intentionally type-directed: scalars, named byte
enums, package object/class references, `Color`, `Vector`, `Rotator`, dynamic
string arrays, and fixed string/name arrays round-trip through the same parser.
Object paths resolve case-insensitively. Invalid scalar or enum text reports a
configuration error rather than changing an authored default to zero; structs
outside those representations are not written.
The shipped Engine metadata declares `Actor.ConsoleCommand(string)` and
`PlayerPawn.ConsoleCommand(string)` with string returns, while
`Console.ConsoleCommand(coerce string)` returns bool. The runtime preserves
those contracts: Actor and PlayerPawn return the host output, and Console
returns whether the host handled the command. The production game installs
`ConsoleCommands` before level events; `runtime_scan` installs its deterministic
headless equivalent. A runtime without that host reports the named native as
unimplemented rather than inventing an empty result.

The host reads configuration from the shared settings overlay. `FLUSH` writes
only queued changes through that overlay (and only the headless scan's
in-memory changes are discarded); it never modifies the installed `System`
directory. `SaveGame N` writes `Saves/saveN.usa` below the same settings
directory. `open` and `start` with that save name load it, while `Snap N` and
`Shot` are captured by the game surface as numbered top-down 32-bit BMP files.
The queue accepts a command only after it has a game owner; actual asynchronous
readback or file errors remain game diagnostics because these shipped calls
discard their return values.

Save files contain an OpenHP1-owned, versioned snapshot rather than a copied
map package: a normalized map identifier and stable package-stem/export
identities identify mutable state. The decoder bounds file size, collection
counts, nesting, finite floats, names, and version before restoring into a
freshly registered authored map. It rejects snapshots taken during an active
iterator or script execution. Restore rebuilds runtime caches, projects saved
instance fields through the ordinary scene-property path, and resumes saved
animations at their saved phase. Platform mixer voices are intentionally
transient: they are omitted from a save and an empty audio host is used after
load.

`PlayerPawn.ClientTravel` emits a host action with its URL, raw UE1 travel-type
byte, and `bItems` flag; the script runtime neither parses nor opens the next
map. `PlayerPawn.UpdateURL` emits a host action carrying the option/value for
case-insensitive replacement and an optional `User.DefaultPlayer` persistence
request; the runtime never mutates map packages. OpenHP1 is a local-only host, so
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
The shipped game authors its horizontal `FOVAngle` for a 640x480 viewport.
Wider viewports preserve that 4:3 vertical span and extend the horizontal view,
so authored follow cameras do not lose their subjects above or below the frame.
In the runtime's Unreal coordinate representation, `vector >> rotator` turns
an authored local offset into world space and `vector << rotator` reverses that
transform. Camera scripts rely on this distinction for their follow offsets.
Positive pitch points toward positive Unreal Z, matching the actor transform;
`rotator()` and `vector()` preserve that inverse relationship. Broom acceleration
and spell-target rays both consume this shared direction conversion.
`WarpZoneInfo.Warp` and `UnWarp` apply the corresponding inverse coordinate
transforms to their location, velocity, and rotation output parameters.
Harry's authored `PostBeginPlay` selects `BaseCam`; the game does not override
that view target. Returning Harry to `PlayerWalking` is not the end of the
cutscene camera sequence: its later `ExitCutState` action restores the saved
third-person camera state and position.
Lev_Tut2 intentionally possesses its authored `BroomHarry0`; `CutHarry0` and
`CutPotionHarry0` inherit `baseChar` and exist only for cutscenes. The broom
pawn's `Possess` calls `BroomPracticeReferee.OnPlayerPossessed`, which triggers
the `Intro` cutscene and transfers the view to its `BaseCam`.
Desktop input follows the original ground controls: W/S or up/down move,
A/D or left/right turn, right click/Control jump, and left click/Alt cast.
The original `DefUser.ini` leaves W/S unbound and maps arrow up/down to
`bBroomPitchUp`/`bBroomPitchDown`; its `bInvertBroomPitch` default is false.
OpenHP1 retains that arrow-key mapping while its added WASD controls use W to
pitch up and S to pitch down. A/D and left/right feed broom yaw. Right click or
Shift boosts, left click or Z brakes, and ordinary jump activates broom action.
This is the original non-inverted behavior when `bInvertBroomPitch` is false;
setting it true gives flight-stick controls (W/down and S/up) and reverses mouse
broom pitch too.
The shipped `DefUser.ini` gives vertical `aMouseY` speed `6.0` and
`aBroomPitch` speed `-6.0`; OpenHP1 preserves those opposite signs.
`PHYS_Flying` retains all three measured velocity components after movement;
only walking flattens velocity to the ground plane. This lets BroomHarry's
authored `Acceleration = 200000 * vector(Rotation)` sustain its full vertical
speed at the configured 60-degree pitch limits.
Keyboard axes use UE1's press delta of 20; mouse axes use its raw-motion delta
of 16, followed by the authored `Speed=6.0` and UE1's
`DeltaTime * 150` rate normalization. Desktop raw motion receives an additional
2.5x scale so a full-height spell gesture fits within a modern trackpad stroke;
its downward-positive window Y is inverted to the original upward-positive
`aMouseY` axis before the separate negative `aBroomPitch` binding is applied.
The held cast button is exposed through the player input properties, while its
press dispatches the original `AltFire` exec so the active state owns sound,
animation, and spell logic. `PlayerInput`
and `PlayerTick` run at the player's position in the actor tick order, so later
actors observe the current frame's processed mouse values as they do in UE1.
Captured mouse-button events always reach gameplay after egui observes them, so
releasing cast cannot leave the held `bAltFire` input stuck.
Holding + (main keyboard or numpad) or F in `openhp1-game` runs 16 ordinary
world, animation, player, trigger, camera, and audio-action ticks per rendered
frame. This debug fast-forward preserves event ordering and latent callbacks
rather than jumping runtime state; held movement/casting input repeats while
one-shot jump and mouse input do not.

The `` ` `` key toggles a resizable developer console pinned to the bottom of
the game window. While it is open, egui receives keyboard and mouse input and
game input remains released. Its vertically scrollable, wrapping output and
command history last for the current process. Command metadata and handlers
live under `crates/openhp1-game/src/app/console/commands`; the same registry
dispatches commands and generates `help`, so adding a command in one handler
module also documents it in the console.

The registered developer commands are:

- `load <level>` resolves a case-insensitive `.unr` name inside the active
  installation's `Maps` directory and starts a fresh level runtime.
- `reset` starts the current level again from its original state.
- `respawn` restores the most recent authored save point reached in the current
  level; it reports an error when no save point has been reached yet.
- `fly` preserves the player while enabling the no-clip camera. WASD moves,
  Q/E moves vertically, the mouse looks around, and Shift increases speed.
  `play` returns to the normal player camera, while `here` places the player at
  the fly camera through the runtime's normal collision-aware placement seam.
- `report <issue>` writes a timestamped Markdown file under the writable
  settings `Reports` directory. It records level/player/camera/runtime/renderer
  state, current errors and capability diagnostics, and named actors within
  2048 Unreal units of the player.
- `help [command]` is generated from the command registry.

Console scrollback and command history survive fresh and saved level loads for
the lifetime of the game process.

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
HP1 `TraceTexture` performs a zero-extent world BSP trace, never an actor
trace. It returns the hit surface's base texture and writes its `Flags` out
parameter as the surface and texture polyflags combined; a miss returns `None`
and writes zero. `bTraceDecals` falls back to that base texture when no decal is
attached. The runtime
tracks non-transient actor sound channels until their WAV or MP2 duration
expires. Authored sound volumes use UE1's compressed playback curve rather than
becoming literal linear gain; for example, `1.0` remains `1.0` while `3.2`
becomes `1.55`. An explicit zero remains silent. `ModifySound(parameter, value,
optional sound, optional slot)` returns true only for a live actor/slot channel,
optionally filters by sound (`None` is a wildcard), and changes volume, radius,
or pitch for parameter values 0, 1, or 2. Slot zero uses an allocated transient
channel and is not selectable by `ModifySound`.

The release `runtime_scan` advances both world and player scripts every frame
after `Possess`, matching the game loop closely enough to expose player-tick
deferrals during local corpus scans.

## Console commands and saved games

The shipped Engine metadata declares `Actor.ConsoleCommand(string)` and
`PlayerPawn.ConsoleCommand(string)` with string returns, while
`Console.ConsoleCommand(coerce string)` returns bool. Actor and PlayerPawn
return host output; Console returns whether the command was accepted. The game
installs the production host before level events and `runtime_scan` installs a
deterministic headless host. A runtime without either host leaves the named
native unimplemented rather than inventing an empty result.

`FLUSH` uses the shared writable settings overlay and never writes installed
INI files. `SaveGame N` stores `Saves/saveN.usa` below that same directory;
`open` and `start` with a save name restore it. `Snap N` and `Shot` are queued
for game-surface BMP capture. The shipped call sites discard the asynchronous
action result, so later readback/file errors remain game diagnostics.

An `.usa` file contains OpenHP1-owned, versioned state rather than copied map
bytes. It records a normalized map identifier and stable package-stem/export
identities, bounds decoding before restore, rejects active iterators or active
script execution before write, rebuilds runtime caches after loading the
authored map, projects restored fields through the normal scene-property path,
and resumes animation at its saved phase. Platform mixer voices are transient:
they are omitted and the new host starts empty after load.
