# Lev3 Dungeon pillar-gate failure

## Conclusion

The two reported symptoms had the same confirmed runtime cause: OpenHP1 did
not collect actor collision results while a moving actor was a brush. A
`GridMover` therefore neither blocks another `GridMover` nor begins touching
the nonblocking class-proximity trigger under the bridge. The shipped engine
does both through the ordinary `ULevel::MoveActor` / `CheckEncroachment` path.

The shared moving-brush encroachment path now classifies every final overlap:
blocking actors receive `EncroachingOn`/`EncroachedBy`, while nonblocking
actors receive `Touch`. No map-specific workaround was added.

## Primary artifacts

- `res/Maps/Lev3_Dungeon.unr`
- `res/System/Engine.u`
- `res/System/Engine.dll`
  - PE32 x86, image base `0x10300000`, timestamp `2001-10-29`
  - SHA-256
    `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`

The binary identity is reproducible with:

```sh
rtk file res/System/Engine.dll
rtk shasum -a 256 res/System/Engine.dll
rtk proxy objdump -p res/System/Engine.dll | \
  rtk proxy rg 'ImageBase|Time/Date|MoveActor|CheckEncroachment|IsBlockedBy|BeginTouch|ActorEncroachmentCheck'
```

## Authored puzzle chain

A read-only decode of the listed `Lev3_Dungeon.unr` exports gives:

| Export | Actor | Relevant authored data |
| --- | --- | --- |
| 2741 | `GridMover15` (`GridMover`) | `Event=Judy`, `MoveIncrement=96`, `KeyPos[1]=(0,0,-304)`, radius 48, height 96 |
| 2743 | `GridMover14` (`GridMover2`) | `Event=Bob`, `MoveIncrement=96`, `KeyPos[1]=(0,0,-304)`, radius 48, height 96 |
| 1283 | `Trigger11` | class proximity `GridMover`, `Tag=Judy`, `Event=Doubletall`, event/tag matching enabled, radius 96 |
| 1302 | `Trigger9` | class proximity `GridMover2`, `Tag=Bob`, `Event=Doubletall`, event/tag matching enabled, radius 96 |
| 2732 | `Counter8` | `Tag=Doubletall`, `Event=Releaseblock` |
| 2230 | `Mover47` | `Tag=Releaseblock`, `TriggerToggle`, `KeyPos[1]=(0,0,176)` |
| 2231 | `Mover48` | same, with a 0.5-second delay |

The shipped `Engine.u` class defaults establish the remaining facts:

- `Mover`: `MoverEncroachType=ME_ReturnWhenEncroach`, `bCollideActors=true`,
  `bBlockActors=true`, and `bBlockPlayers=true`.
- `GridMover`: inherits `Mover`, adds `bCollideWorld=true`, and starts in
  `BumpMove`.
- `Trigger`: `bInitiallyActive=true`.
- `Counter`: `NumToCount=2`.

The compiled source embedded in that same package confirms the execution
chain:

1. `GridMover.BumpMove` changes `KeyPos[1]`, calls `DoOpen`, completes the
   interpolation, then calls `FinishedOpening`.
2. `Trigger.Touch` accepts the matching class and matching mover `Event`/trigger
   `Tag`, then `Activate` triggers every actor tagged `Doubletall`.
3. `Counter.Trigger` decrements `NumToCount`; at zero it triggers every actor
   tagged `Releaseblock`.
4. Both gate movers then open by 176 units.

Useful source extraction commands:

```sh
rtk proxy rg -a -n -A 100 'class GridMover extends Mover;' res/System/Engine.u
rtk proxy rg -a -n -A 130 'function bool IsRelevant\( actor Other \)' res/System/Engine.u
rtk proxy rg -a -n -A 80 'class Counter extends Triggers' res/System/Engine.u
```

## Shipped native behavior

The exported functions are jump thunks to these implementations:

| Function | Export RVA / VA | Implementation VA |
| --- | --- | --- |
| `ULevel::MoveActor` | `0x404d` / `0x1030404d` | `0x103aa3a0` |
| `ULevel::CheckEncroachment` | `0x26fd` / `0x103026fd` | `0x103ab5f0` |
| `AActor::BeginTouch` | `0x1690` / `0x10301690` | `0x10379fe0` |
| `AActor::IsBlockedBy` | `0x169a` / `0x1030169a` | `0x10352140` |
| `FCollisionHash::ActorEncroachmentCheck` | `0x130c` / `0x1030130c` | `0x103658b0` |

Confirmed control flow:

- `MoveActor` iterates collision results at `0x103ab121`. After base-chain
  exclusions, it calls `IsBlockedBy` at `0x103ab15e`. A nonblocking result
  calls `BeginTouch` at `0x103ab16d`. It subsequently scans the four `Touching`
  slots and calls `EndTouch` at `0x103ab19f` when an overlap ended.
- `CheckEncroachment` explicitly checks the moving actor and candidates against
  `ABrush::PrivateStaticClass` (`0x105e91a0`) rather than excluding brushes. At
  `0x103ab6cf` it calls collision-hash vtable slot `+0x1c`, which resolves to
  `ActorEncroachmentCheck`. Candidate processing calls `IsBlockedBy` at
  `0x103abb36`; blocked actors take the encroachment path, while eligible
  nonblocking actors call `BeginTouch` at `0x103abb7b`.
- `IsBlockedBy` walks both actors' class ancestry against `ABrush` at
  `0x10352169..0x1035218e` and `0x10352204..0x10352229`, then applies their
  collision/block flags. It has no blanket brush exclusion. With the shipped
  mover defaults, mover-versus-mover is blocking.
- `BeginTouch` updates both actors' four-slot touch arrays and dispatches the
  UnrealScript `Touch` event (`0x1037a297..0x1037a2bd`).

Focused reproduction:

```sh
rtk proxy objdump -d --start-address=0x103ab0d0 --stop-address=0x103ab1a7 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x103ab5f0 --stop-address=0x103abba8 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x10352140 --stop-address=0x10352388 res/System/Engine.dll
rtk proxy objdump -d --start-address=0x10379fe0 --stop-address=0x1037a310 res/System/Engine.dll
```

## OpenHP1 divergence and exact replay

In [`movement.rs`](../crates/openhp1-runtime/src/world/movement.rs):

- Before this fix, `movement_hit` only called `actor_sweeps` when
  `current.brush.is_none()`. A moving `GridMover` consequently received no blocking mover
  hit and no nonblocking trigger hit.
- The later `moving_brush_encroached` pass also discarded every candidate whose
  `other.brush.is_some()`, so it could not recover mover-versus-mover
  blocking.
- Stationary touch discovery likewise returned immediately for a brush, so it
  could not recover the missing trigger touch after placement.

A clean `Lev3_Dungeon` runtime replay moved the actors by their authored
96-unit grid increments to the exact reported final positions:

```text
GridMover15: (655.40924, -5200, -2415.4102)
GridMover14: (560,       -5200, -2415.4102)
```

Neither downward crossing emitted a `DispatchEvent`; `Counter8` remained
untriggered and `Mover47`/`Mover48` stayed at Z `-2208`. Calling the existing
`Counter8.Trigger` path twice in the same clean runtime moved both gate movers
to Z `-2032`, exactly the authored `+176`. This isolates the failure before
the counter: the mover collision/touch results never enter the script chain.

## Resolution and verification

The fix stays at the shared native movement seam:

- moving-brush encroachment runs after partial as well as complete movement;
- blocking brush overlaps use the existing actor contact margin and the
  authored `EncroachingOn` behavior;
- nonblocking overlaps enter the normal `Touch`/`UnTouch` lifecycle.

The synthetic runtime regression covers trigger touch, blocking-brush return,
and adjacent-brush contact. All 184 `openhp1-runtime` tests pass. A clean
`Lev3_Dungeon` replay placed the pillars at `(655.40924, -5200, -2415.4102)`
and `(560, -5200, -2415.4102)`; both class-proximity triggers fired and the two
gate movers reached their authored open Z position of `-2032` without manual
event injection.
