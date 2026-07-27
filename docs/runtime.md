# UnrealScript runtime behavior

This document records durable runtime semantics that are easy to lose when
extending native calls, actor state, or animation actions.

## Actor identity and state

Actors use stable package/export identities. Class defaults followed by actor
tagged-property overrides initialize persistent instance state. Remote actor
contexts must resolve registered actor handles so field reads, writes, and
calls affect the target actor rather than a temporary copy.
Signed `ObjectConst` package references are resolved relative to the function
or state package before context operations use their runtime handles.
`ClassContext` reads instance-variable expressions from the referenced class's
inherited default object.

Nested remote calls may inspect or call back into their caller, so the caller's
live instance remains addressable while the remote context executes. Serialized
`UStruct` child chains may contain any `UField`; non-property fields are skipped
by following their shared `Next` link.

Runtime actions update both persistent actor state and the corresponding scene
state. In particular, later animation ticks must not undo `SetLocation` or
other transform changes.

## State execution

Persistent state frames retain their decoded instruction pointer and local
values across latent `Sleep` and `FinishAnim` actions. `GotoState`,
`GotoLabel`, and `Stop` operate on that retained frame rather than restarting
the state body.
`FinishInterpolation` resumes the retained frame when mover physics clears
`bInterpolating`.

Nested state execution restores the caller's active-state context. A
Dispatcher may therefore trigger another actor and then enter `Sleep` without
the nested state making that latent call appear to come from ordinary function
code.

Label lookup uses the final top-level `LabelTable` in canonical decoded
bytecode. Serialized state metadata offsets are not canonical decoded-byte
offsets.

## Animation actions

- `PlayAnim` and `LoopAnim` use the scene's existing animation path.
- Animation completion occurs at `AnimLast`, before the sampler wraps toward
  frame zero.
- Repeated `LoopAnim` calls preserve the current phase.
- `FinishAnim` ends the current loop.
- Tween-time arguments blend from the displayed pose.
- `IsAnimating` reflects active `PlayAnim` and `LoopAnim` actions.

Unsupported actions should remain nonfatal actor diagnostics until their
subsystem exists; they must not silently claim successful behavior.

## Movement and spawning

Walking physics advances when either horizontal velocity component is nonzero;
axis-aligned paths must not wait for `MoveTo` to time out.
Latent `TurnTo` updates `DesiredRotation` toward `Focus` and resumes its state
frame once the yaw is within the UE1 arrival threshold.
`MoveSmooth` first attempts the requested movement and then slides the
untraveled delta along the collision plane; it is not an alias for `Move`.
`MakeNoise` currently validates its loudness without populating pawn noise
slots or dispatching `HearNoise`.

Runtime-spawned actors use the same class-default mesh, material, lighting, and
animation assembly as actors serialized in the map. Adding their geometry may
grow the scene topology, so render consumers reload their GPU scene resources
when an in-place vertex update no longer fits.
Spawning a collision-enabled actor at an occupied blocking location fails
without allocating an actor handle. Spawned pawns link themselves into
`Level.PawnList` through `nextPawn`, matching the native `AddPawn` bookkeeping
used during `PreBeginPlay`.
`SetOwner` updates the persistent `Owner` reference and sends `LostChild` and
`GainedChild` to the old and new owners.
Engine side effects without an OpenHP1 surface do not abort scripts:
`SaveConfig` is read-only, `ConsoleCommand` returns an empty string, and decal
detachment is a no-op until decals render.

Cutscene cameras use UE1 vector/rotator transforms, BSP `Trace`, and pawn
visibility tests. `TraceActors` currently yields no actor hits; add actor
iteration when gameplay needs its output locations and normals.
The game startup camera fallback is cleared when the player leaves the `Cut*`
state family so normal `PlayerCalcView` output resumes with player control.

Script `Name` comparisons treat a missing object/name value as UE's canonical
`None` name.

HP1 `CreateAnimChannel` creates the requested channel through the normal actor
spawn lifecycle so scripts can retain and own the returned object.
