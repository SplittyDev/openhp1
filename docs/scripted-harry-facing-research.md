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

The remaining shared gap is broader than `CutMovingTo`: shipped `Engine.u`
explicitly says both latent `MoveTo` and `MoveToward` rotate the pawn toward
their destination, but the runtime's existing `PlayerPawn` rotation gate only
yields to `CutMovingTo` or matching latent `TurnTo`/`TurnToward`. Harry has
authored latent `MoveTo` and `MoveToward` paths outside `CutMovingTo`. The
source-backed fix is narrow: make that gate also yield for a matching latent
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

## SurrealEngine comparison and uncertainty

The local SurrealEngine calls virtual `TickRotating` after every non-`PHYS_None`
physics step. Its `UPawn::TickRotating` consumes `DesiredRotation`, but its
`UPlayerPawn::TickRotating` is effectively a no-op. That cannot satisfy the
shipped HP scripts above without another HP-specific path, so it is useful for
the general tick order but not authoritative for Harry's rotation ownership.

The original native `Engine.dll` implementation was not disassembled for this
note. The package evidence establishes who writes each rotation value, but not
whether the retail executable gives HP's `PlayerPawn` a native special case.
The package-backed runtime change is to allow matching latent `MoveTo` and
`MoveToward` through the existing `PlayerPawn` rotation gate, alongside matching
`TurnTo` and `TurnToward`. The latent-movement regression drives the matching
`PlayerPawn` through the physics rotation path and verifies that it turns toward
the movement-authored heading. An authored `ChessMode` replay remains useful
because its own `PlayerTick` may intentionally take precedence.
