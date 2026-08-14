# Lev3_Troll bathroom encounter evidence

## Report reproduction

The latest capture, `report-1786694287-162744000.md`, reports that Harry and
Ron are not hurt by the troll's projectiles and that the troll continues
throwing after the club falls. The capture contains no deferred runtime call or
runtime error.

The captured positions expose the club failure:

- `BathroomClub0` rests at `(16078.9, -2139.6, -512.9)`.
- `BathroomTroll0` remains in `Combat` at `(16078.9, -2002.7, -375.3)`.
- Their horizontal separation is 136.9 units, entirely on Y. The club reached
  the troll's earlier horizontal position and fell after the troll had
  continued moving.

The nearby-actor list also contains many projectile fragments clustered around
Harry. This establishes that thrown objects reached world collision and
exploded; it does not establish that their `Touch` callbacks ran before that
destruction.

### Follow-up capture

`report-1786723601-021126000.md` was captured after the first parity changes.
It shows that the walking-arrival correction worked but did not finish the
encounter:

- `BathroomClub0` and `BathroomTroll0` now have the same X/Y position,
  `(16078.9, -2002.7)`. The club rests 146.5 units above the troll, at Z
  `-228.7`, while the troll remains in `Combat` at Z `-375.2`.
- The report contains 20 deferred-call occurrences. Three distinct new
  diagnostics come from toilet projectiles executing `Touch` and then
  `HitWall`: their native `IsA` call cannot inspect actor 1793 while that club
  instance is active. The separate `Trigger8` re-entry diagnostic was already
  present in the preceding capture and is not part of this encounter path.
- The capability section has 13,644 entries versus 13,640 in the preceding
  report. All but the four deferred-call diagnostics are pre-existing scene
  projection notices, dominated by `Texture`, `LODBias`, `Fatness`, and light
  properties. They are unrelated to the troll state transition and are not
  suppressed by this fix.

## Shipped script path

The active compiled bytecode, not only the embedded source text, establishes
the encounter contract:

- `Hub3.u` `BathroomClub.Levitating.BeginState` snapshots
  `ClubTarget.Location` into `_TargetLoc`. `MovingState` travels to that fixed
  point and then enters `FallingState`.
- `BathroomClub.FallingState.BeginState` enables collision and falling.
  `FallingState.HitWall` (export 134) sends a `BathroomTroll` target to
  `Concussed`.
- `BathroomTroll.Concussed` (state export 81, `BeginState` export 106) plays the
  knockout sequence and ends the encounter. `Combat` itself contains no actor
  movement command.
- `HProps.u` `BaseToiletObject.FlyingState.Touch` (export 588) calls its
  `HitWall`; `HitWall` (export 592) calls `TakeDamage` for `baseChar` and
  `baseHarry`, then explodes the projectile.
- `Hub3.u` `BathroomRon.TakeDamage` (export 55) enters Ron's hurt state and
  moves the club backward.

The club therefore must be aimed after scripted character movement has really
arrived, and projectile `Touch` must execute before a same-move world impact
can destroy the projectile.

## Original engine evidence

The matching shipped `Engine.dll` supplies the three relevant semantics:

1. `APawn::moveToward` flattens the destination delta for `PHYS_Walking` and
   reports arrival only when horizontal distance squared is below `256`
   (16 units). It clears acceleration on that walking success path.
   OpenHP1 instead used the generic `distance² < velocity² * 0.05` test, which
   can complete a fast character's movement more than 100 units early and let
   residual velocity carry it away from a location sampled immediately by
   script.
2. `ULevel::MoveActor` calls `AActor::BeginTouch` before returning its blocking
   hit. `BeginTouch` invokes the moving actor's `Touch` synchronously, then the
   other actor's callback if the first callback did not invalidate the touch.
   OpenHP1 queued both callbacks for scene processing. Falling physics could
   consequently run `HitWall`, destroy a thrown object, and later discard its
   queued `Touch` because the actor was already destroyed.
3. `AActor::physFalling` has a separate `bBounce` collision branch. That branch
   invokes the authored `HitWall` event directly, checks whether the callback
   stopped physics, and only then reflects the remaining movement. It does not
   route through `AActor::processHitWall`. This is the path used by
   `BathroomClub.FallingState`, whose compiled `BeginState` sets `bBounce=True`
   and whose compiled `HitWall` sends a `BathroomTroll` target to `Concussed`.

The native also confirms that `AActor::processHitWall` deliberately suppresses
`HitWall` for a pawn collision on the non-bouncing path. Removing that generic
guard to special-case the club would contradict the original engine; the
correct fix is to preserve the native's distinct bouncing path.

The licensed SurrealEngine reference agrees that collision movement owns
`Touch` delivery before the caller continues, although its generic latent
movement threshold differs from this HP1-specific `Engine.dll`. The shipped
binary is authoritative for the threshold.

## Implemented parity

- Walking latent movement now uses the shipped 16-unit horizontal arrival
  radius. Non-walking latent movement retains its existing behavior.
- Nonblocking contacts found by actor movement now execute `Touch`
  synchronously, moving actor first, and do not call the second actor after
  either participant is destroyed. While the reciprocal callback runs, the
  active mover remains available to qualified calls such as the shipped
  projectile's `HitActor.IsA(...)`. Location-based overlap updates outside
  movement retain their existing queued path.
- Falling actors with `bBounce=True` receive `HitWall` even when the blocking
  actor is a pawn, before the bounce response. Non-bouncing pawn contacts keep
  the existing `processHitWall` suppression.

No map, actor, coordinate, projectile, or class-name workaround was added.

## Live validation

On 2026-08-15, live play confirmed that the club now knocks out the troll, the
troll dies at the end of the sequence, and the game continues into the next
authored progression. Harry and Ron still do not take damage from the thrown
objects. Projectile damage therefore remains unresolved; the synchronous
`Touch` and active-context changes are established engine parity but are not
claimed as a complete fix for that symptom.

## Reproducibility

The original files were inspected read-only:

- `Engine.dll`: `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`
- `Hub3.u`: `e4af35e141baa1937d816846b705ab457d2ded1d4bab4a1fad721ce36c54aa02`
- `HProps.u`: `be6c9937c905d3582f2af3f95d85d06b01d82832ce187c11c26ead1c9cd24342`

`package_inspect` and `script_inspect` verified the named exports and executable
bytecode. `strings` was used only to correlate that bytecode with the embedded
class text. The original assets were neither modified nor copied into the
repository.
