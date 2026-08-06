# Death, respawn, and level travel in the original game

This note records the original-game paths relevant to void deaths, normal
fainting, save-point reloads, and end-of-level travel. The evidence order is:
active compiled exports from the shipped packages, their embedded
UnrealScript text, and then the locally installed SurrealEngine reference.
Embedded source is called out separately because comments and even whole
statements can differ from the compiled bytecode.

## Reproducing the package evidence

The export identities below can be checked without extracting or modifying
the original files:

```sh
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-package \
  --example package_inspect -- res/System/HPBase.u
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- res/System/HPBase.u 2907
strings -a -n 1 res/System/HPBase.u
```

Use the corresponding package and export number for the other compiled
functions cited below. `strings` exposes the embedded `TextBuffer`; it is not
proof that a statement compiled, so every consequential path below is also
checked against its function or state export.

## Falling out of the world

`Engine.u` distinguishes an ordinary actor from a pawn:

- `Actor.FellOutOfWorld`, active function export 2258, calls
  `SetPhysics(PHYS_None)` and destroys the actor. The embedded source is in
  `Actor.ScriptText`, export 5401.
- `Pawn.FellOutOfWorld`, active function export 4525, has a role guard, sets
  `Health = -1`, calls `SetPhysics(PHYS_None)`, clears `Weapon`, and dispatches
  `Died(None, 'Fell', Location)`. The embedded source is in
  `Pawn.ScriptText`, export 4863. The compiled export contains the assignments,
  native physics call, and virtual `Died` call; this is active behavior rather
  than a source comment.
- `ZoneInfo.ScriptText` declares `bKillZone` as a const editable boolean whose
  stated purpose is to kill actors entering the zone instantly. A read-only
  parse of map actor tagged properties finds 14 active ZoneInfo instances with
  both `bKillZone=True` and `bPainZone=True`. Examples include
  `Lev_Tut3.unr` export 1029 (`DamagePerSec=1000`), `Lev_Tut1b.unr` exports
  2194 and 2346 (`DamagePerSec=200`), `Lev3_Dungeon.unr` exports 330, 593,
  716, and 2057 (`DamagePerSec=500`), and `Lev5_Final.unr` exports 877 and
  889 (`DamagePerSec=200`). These are authored lethal/void regions, not a
  proposed map heuristic.

The licensed SurrealEngine physics reference independently checks
`Region().ZoneNumber == 0` before walking, falling, swimming, flying, and
rolling physics, dispatches `FellOutOfWorld`, and returns immediately. See
`SurrealEngine/UObject/UActor.cpp` lines 475-485 and 649-655 in the local
SurrealEngine checkout. This is the important distinction: zone zero is an
out-of-world physics region even though rendering may legitimately use zone
zero as an ambient fallback.

OpenHP1 originally lost both death signals at the shared zone seam:

- `world::zone_actor_at` correctly returns `None` for BSP zone zero, but
  `zone_physics` converted that value into an error. The existing
  `None => FellOutOfWorld` branches in walking, falling, swimming, flying, and
  rolling physics were therefore never reached.
- `ZonePhysics` samples water, pain, and damage type, but not `bKillZone`.

That was the shared cause of void/kill regions behaving like ordinary space.
`zone_physics` now preserves the out-of-world `None` result and treats an
authored `bKillZone` as lethal. Both reuse the existing shared physics dispatch
instead of adding a height or map-specific workaround.

The original PC manual says both depleted stamina and a fall from a great
height faint Harry and restart from the last save point. `baseHarry.stateDead`
is the shipped implementation of that presentation and reload: `KillHarry`
plays `faint`, waits, and calls `LoadSelectedSlot`. Out-of-world physics routes
`baseHarry` through `KillHarry(True)` while other actors retain the engine's
ordinary `FellOutOfWorld` event.

The engine death path also calls static functions through class objects such
as `LocalMessage`. UE1 class exports have no object class reference; their
export class is `None`. Context dispatch must therefore use the class defaults
for such a receiver rather than trying to instantiate it as an ordinary
object. Otherwise `Pawn.Died` stops before its death state completes.

Harry also has a separate pain-damage path. `HarryPotter.u` active
`Harry.TakeDamage` export 298 tests damage types `ZonePain` and `pit`, enters
`hit_InstantPitDeath`, hides Harry, and then makes its compiled final call to
`baseHarry.TakeDamage`. Active state export 1043 performs the hide/drop logic.
That path should not be substituted for `FellOutOfWorld`; it is authored pain
damage rather than the engine's outside-world event.

## Enemy damage, fainting, and save-point reload

The shipped normal-damage path is explicit:

1. `HPBase.u` `baseHarry.TakeDamage`, active export 2922, reduces
   `lifePotions`. Its compiled branch calls `KillHarry(True)` when the result is
   at or below zero.
2. `baseHarry.KillHarry`, active export 2909, stores the argument in
   `bAllowHarryToDie` and executes `GotoState('stateDead')`.
3. `stateDead.BeginState`, active export 2895, zeros horizontal movement and
   calls `PlayAnim('faint', rate)`.
4. The latent body of active `stateDead` export 2907 finishes the animation,
   moves to the current location, sleeps for 0.5 seconds (2 seconds for slow
   death), tests `bAllowHarryToDie`, and calls virtual function
   `LoadSelectedSlot` through `baseConsole(player.console)`. It then loops.
   Name index 388 in this package is `LoadSelectedSlot`.

The embedded source is `baseHarry.ScriptText`, export 3547, and agrees with
that compiled path. It also contains lines for `SaveGameExists`, direct
`ConsoleCommand("open save9.usa")`, and `Level.Game.RestartGame`, but those
lines do **not** appear in active state export 2907. They are inactive
commented-out alternatives and must not be implemented as the original path.

The selected-slot call continues through shipped menu code:

- `HPMenu.u` active `HPConsole.LoadSelectedSlot` export 1378 calls
  `MenuBook.LoadSelectedSlot` and `StopFastforward`.
- Active `FEBook.LoadSelectedSlot` export 1152 forwards to its selected slot
  page.
- Active `FESlotPage.LoadSelectedSlot` export 1808 builds `open save99.usa`
  for the fallback or `open saveN.usa` for the selected slot and sends it
  through `ConsoleCommand`. Its embedded source is `FESlotPage.ScriptText`,
  export 1901.

OpenHP1 already has the final host operation: `ConsoleCommands` recognizes
`open saveN.usa`, emits `ConsoleCommandAction::OpenSave`, and the game host
reloads the saved snapshot. The missing bridge is earlier. The local host
dispatches `Possess`, but does not construct or assign the UE1 viewport
`Player` and `Player.Console` UObject chain; searches of runtime initialization
show no assignment to either property. A context call through a null
`player.console` therefore cannot reach the active `HPConsole.LoadSelectedSlot`
body. This matches the observed state: the authored faint completes, input is
still ignored by `stateDead`, and its reload call has no effective receiver.

OpenHP1 now supplies narrow host-backed `Player`, `Console`, and `MenuBook`
identities for the player pawn. They survive the authored casts and route
`SaveSelectedSlot` and `LoadSelectedSlot` from either the console or its menu
book into the existing console-command host. Until the frontend supplies an
explicit selected slot, this uses the shipped `FESlotPage` fallback slot 99.
Building the entire legacy UWindow object graph, adding a death timer, or
resetting the map would bypass the authored selected-slot behavior.

## End-of-level travel

The active authored route is server travel, not local `ClientTravel`:

1. `HPBase.u` `TriggerChangeLevel.ProcessTrigger`, active export 2824, finds
   `baseHarry` and calls virtual `ChangeLevel(NewMapName, True)`. The embedded
   `TriggerChangeLevel.ScriptText`, export 3236, also contains a direct
   `a.Level.ServerTravel` line, but active export 2824 contains only the
   `ChangeLevel` call; the extra source line is stale/inactive.
2. `HPMenu.u` active `HPConsole.ChangeLevel` export 1373 calls
   `viewport.Actor.Level.ServerTravel(lev, flag)` and then sets
   `bLoadNewLevel = true`. Its virtual name index 413 is `ServerTravel`.
3. `Engine.u` active `LevelInfo.ServerTravel` export 3666 checks `NextURL`,
   stores `bNextItems` and `NextURL`, and calls
   `Game.ProcessServerTravel(URL, bItems)` (or zeroes the countdown if there is
   no game). Its embedded source is `LevelInfo.ScriptText`, export 5261.
4. Active `GameInfo.ProcessServerTravel` export 5085 notifies network clients
   with `ClientTravel`, but calls `PreClientTravel` for the local player. In a
   standalone game it sets `Level.NextSwitchCountdown = 0`. Its embedded source
   is `GameInfo.ScriptText`, export 4458.
5. The native host watches `LevelInfo.NextURL`, advances
   `NextSwitchCountdown`, and opens the new map when the countdown reaches
   zero. SurrealEngine models this host responsibility in
   `SurrealEngine/Engine.cpp` lines 210-244, including relative travel and
   `bNextItems` handling.

Serialized map instances prove that this is used for real progression. The
destination string sits 22 bytes into each cited `TriggerChangeLevel` actor
payload:

| Map | Actor export | Serialized destination |
| --- | ---: | --- |
| `Lev_Tut2.unr` | 443 | `lev_tut3.unr` |
| `Lev2_Inc_B.unr` | 2598 | `Lev2_HogFront_2.unr` |
| `Lev3_Lumos.unr` | 5916 | `Lev3_PreDungeon.unr` |
| `Lev4_Sneak.unr` | 1634 | `lev4_Sneak2.unr` |

Before the fix, OpenHP1 missed two consecutive boundaries in this route. First,
`TriggerChangeLevel` reaches `HPConsole.ChangeLevel` through the same null
`player.console` object chain described above, so the authored call may be
skipped before `LevelInfo.ServerTravel` runs. Second, even after that call is
bridged, the game host does not observe
`LevelInfo.NextURL`/`NextSwitchCountdown` and therefore never loads the
destination. The existing `ActorAction::ClientTravel` does not cover this
local route: standalone `ProcessServerTravel` does not invoke `ClientTravel`,
and the game currently forwards external actions only to its audio handler
anyway.

OpenHP1 now bridges the active `HPConsole.ChangeLevel` operation and carries
its URL plus `bItems` through the existing external travel action. The game
host consumes that action, preserves URL options while resolving the map
component case-insensitively inside the installed Maps directory, and queues
the existing fresh-level loader. No filename sorting or hard-coded next-level
table is involved; each authored trigger remains the source of its
destination.
