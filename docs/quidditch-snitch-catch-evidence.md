# Quidditch Snitch chase and catch evidence

## Scope and conclusion

This note investigates the report that, after the Quidditch catch sequence
starts, Harry follows the Snitch slowly and there is no apparent way to catch
it. It also checks the separate perception that the Snitch is too slow.

The evidence supports two current defects. The second became provable only
after a fresh launch exposed the loaded level clock:

| Finding | Status | Evidence boundary |
| --- | --- | --- |
| The catch interaction is absent | **Confirmed OpenHP1 defect** | Shipped bytecode makes the catch a timed `BaseQHudGame` interaction. `QuidHud.PostRender` lazily creates, advances, and paints it. OpenHP1 does not execute that Canvas path or project an equivalent catch state into its host HUD. |
| Harry follows the Snitch during the catch phase | **Authored behavior** | Compiled `GamePlay.Tick` enters `GameCatch` without stopping or attaching the Snitch. `GameCatch.BeginState` puts Harry in `Pursue`; successful HUD input later stops the Snitch and calls `CatchTarget`. |
| The Snitch is already very slow when play starts | **Confirmed OpenHP1 defect** | The map serializes an editor-time `LevelInfo.TimeSeconds` value of `743.39136`. OpenHP1 retained it, so the scripted 240/300/360-second slowdowns all ran during startup. The shipped engine resets that field to zero before startup events. The authored path speed remains 350 and is not tuned. |

The apparent inactivity was therefore not user confusion and should not be
addressed by changing pursuit physics. The original catch UI and its update
lifecycle were missing before the implementation described below.

## Evidence set

The primary evidence is the installed game's shipped data, compiled
UnrealScript, configuration, localization, and native engine binary:

| File | SHA-256 |
| --- | --- |
| `res/Maps/Lev2_Quid1.unr` | `206238162d51518633aabb2b5d04d9caa254dc65b01fa45d5c4b74e900d168cd` |
| `res/System/Hub2.u` | `b44c845961a45d6b34577a59309c569c4c8236ec9ff7f7bb82526e7f499e39d1` |
| `res/System/HPBase.u` | `0cec62e098ded3a16024ee15dbc982bf9662b443f630cd19890b7b5d325bf503` |
| `res/System/HPMenu.u` | `42da2a2f43ac6a15ea87eace4ebd59a69bab7685cda854e9e7a86e7e6d9c6dbd` |
| `res/System/HarryPotter.u` | `5f18066ac7d6a64ba315a19753308613c0819b3944da551a17bd0f710560cf60` |
| `res/System/Engine.u` | `b3661a1d2afb1f730ba5bb3cbd2a6716efaf55fd0d8cefa753b69026a8bc5a85` |
| `res/System/Engine.dll` | `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc` |

The latest pre-fix diagnostic capture is
`~/Library/Application Support/OpenHP1/Reports/report-1786638381-118618000.md`.
It records `Lev2_Quid1` and the previously fixed eager boolean evaluation in
the referee's gameplay tick. It does not capture the newly reachable
`GameCatch` phase and therefore provides no measured post-fix Snitch velocity.

Shipped compiled bytecode and serialized defaults take precedence over
embedded script text. This matters here: embedded `Hub2` source contains an
older commented catch block and nearby source text that suggests the Snitch is
stopped and attached before `GameCatch`. The compiled `GamePlay.Tick` export
does not contain those actions. Treating the embedded text as active would
produce the wrong diagnosis.

The shipped evidence was sufficient to decide the catch behavior and the
clock reset. SurrealEngine was used only as a licensed cross-check of the
fresh-level startup flag sequence.

## Authored Snitch path sequence

`Lev2_Quid1.unr` export 587 is `Snitch1`, class `Hub2.Snitch`. Its serialized
properties are:

- `Path[0] = IPSnitch0`
- `Path[1] = IPSnitchCutLoop`
- `Path[2] = IPSnitchIntro`
- `Path[3] = IPSnitch0`
- `bSwitchPathsOnTrigger = true`
- `fLaunchSpeed = 300`
- `fHoopSpacing = 0.2`
- `HoopTrailLen = 5`
- `InitialHoopTrailEnd = 5`
- `bCollideWorld = false`

The controlling compiled `Hub2.QuidditchPawn` functions are
`PostBeginPlay` (export 45), `FinishedInterpolation` (export 264),
`ChooseNextPath` (export 374), `SwitchPaths` (export 376), `FlyOnPath`
(export 405), `CheckIfTimeForSpeedChange` (export 386), and
`ApplySpeedChange` (export 389).

`PostBeginPlay` initializes `CurPath` to -1, selects the next path, and starts
it through `InterpolationManager.Init(next, 1, false)`. The map's `CutScene0`
(export 429) contains two authored `Trigger Snitch` commands, one after the
`SnitchScene` wait and one after `GetGoingHarry`. Together with natural path
completion, the sequence is:

```text
IPSnitch0 -> IPSnitchCutLoop -> IPSnitchIntro -> IPSnitch0 (gameplay loop)
```

`fLaunchSpeed` is used by the pawn's launch-at-target behavior. It is not the
routine interpolation speed and is not evidence for raising gameplay path
speed.

## Exact gameplay speed evidence

### Serialized path values

All 16 interpolation points on the repeating gameplay path `IPSnitch0`
explicitly serialize `DesiredSpeed = 350`. `Engine.InterpolationPoint` class
export 54 supplies the inherited default `bConstantSpeed = true`. Every point
has a nonzero authored `PathDist`:

| Position | DesiredSpeed | PathDist |
| ---: | ---: | ---: |
| 0 | 350 | 561.22205 |
| 10 | 350 | 562.1041 |
| 20 | 350 | 520.64716 |
| 30 | 350 | 765.5679 |
| 40 | 350 | 1249.095 |
| 50 | 350 | 911.65063 |
| 60 | 350 | 1091.9961 |
| 70 | 350 | 1147.2555 |
| 80 | 350 | 905.77893 |
| 250 | 350 | 1351.5475 |
| 270 | 350 | 703.4918 |
| 280 | 350 | 1550.0255 |
| 290 | 350 | 857.49524 |
| 300 | 350 | 858.6953 |
| 310 | 350 | 1395.1064 |
| 320 | 350 | 1253.7174 |

The distances total approximately 15,685.3965 units, giving a nominal loop
time of about 44.815 seconds at 350 units/s.

The cinematic paths intentionally use different speeds. `IPSnitchIntro` has
350 at position 0, inherits the `Engine.InterpolationPoint` default 900 at
position 10, uses 1200 at position 20, and 350 at position 40.
`IPSnitchCutLoop` inherits 900 at positions 0 and 50, with explicit values
700, 600, 300, and 700 at positions 10 through 40.

### Authored long-match slowdown

The compiled Snitch defaults set `fSpeedChangeFactor = 0.8`, the first speed
change at 240 seconds, and `MaxSpeedChanges = 3`; the inherited period is 60
seconds. `ApplySpeedChange` multiplies all path-point `DesiredSpeed` values and
`fLaunchSpeed` by that factor. The gameplay speed is therefore deliberately:

| Match time | Authored gameplay speed |
| ---: | ---: |
| Before 240 s | 350 |
| At 240 s | 280 |
| At 300 s | 224 |
| At 360 s | 179.2 |

A very slow Snitch late in a match is therefore original authored behavior.
The defect was that OpenHP1 entered a fresh match with the level clock already
past all three thresholds.

### Native interpolation rate

`Engine.dll` exports
`?performPhysics@AInterpolationManager@@UAEXM@Z` at virtual address
`0x10301d02`, which jumps to the implementation at `0x103f7ba0`. Relevant
fields observed in that routine are manager destination `+0x25c`, alpha
`+0x260`, rate `+0x264`, and remaining pause `+0x268`; owner `IPSpeed` is at
`+0x138`; destination `Prev`, `DesiredSpeed`, and `PathDist` are read at
`+0x3b4`, `+0x3e0`, and `+0x3e4`.

- `0x103f7c4a` tests the destination interpolation point's constant-speed flag.
- `0x103f7ca0..0x103f7cf0` uses positive owner `IPSpeed` divided by
  destination `PathDist`.
- `0x103f7cf8..0x103f7dc6` otherwise reads previous and destination
  `DesiredSpeed`; `0x103f7d76..0x103f7d94` linearly blends them by alpha and
  divides by destination `PathDist`.
- `0x103f7e64..0x103f7eb8` multiplies by interpolation rate and frame delta,
  then advances and clamps alpha.

OpenHP1 implements that active constant-speed path in
`crates/openhp1-runtime/src/world/physics/dynamics.rs`: it prefers positive
`IPSpeed`, otherwise linearly blends previous and destination `DesiredSpeed`,
then advances alpha by `rate * speed / PathDist`. OpenHP1 does not yet model
the native non-constant-speed branch, but that is not a demonstrated cause for
`IPSnitch0` because its active inherited value is `bConstantSpeed = true`.

Consequently, the shipped path values and native formula do **not** authorize
speed tuning. The confirmed defect is the fresh-level clock: the map's
`LevelInfo` actor serializes `TimeSeconds = 743.39136`, while the shipped
`Engine.LevelInfo` class default is zero. A pre-startup OpenHP1 diagnostic
showed the serialized value surviving actor registration.

The shipped `UGameEngine::LoadMap` implementation is exported at
`0x10303477` and enters at `0x1039c3d0`. Immediately before the startup event
loops, `0x1039dc3e` clears `EBX` and `0x1039dce1` stores that zero to the
loaded `LevelInfo` field at `+0x3b4`. `ULevel::Tick`
(`0x103010a5 -> 0x103b6db0`) writes the running level time to that same field
at `0x103b70dc`. The load routine then sets the begun/startup bits at
`0x1039dd75..0x1039dd89`, dispatches `InitGame`, `PreBeginPlay`, `BeginPlay`,
`PostBeginPlay`, and `SetInitialState`, and clears the startup bit at
`0x1039e105..0x1039e10f`.

OpenHP1 now performs the same fresh-level reset and startup flag transition.
A direct 11-second launch of `Lev2_Quid1` completed 1,060 startup events with
zero deferred calls and emitted none of the three scripted `Speeds changed`
messages. The authored 350 speed, `fLaunchSpeed`, `PathDist`, and interpolation
formula are unchanged.

## Compiled catch state sequence

### Entry into `GameCatch`

The decisive behavior is in compiled `Hub2.QuidditchReferee.GamePlay.Tick`,
export 557. At bytecode offset `0x038c`, a conditional checks whether
`fProgressPercent >= 100`. At `0x039b`, the taken branch immediately executes
native 113 `GotoState` with `NameConst GameCatch` at `0x039c`.

That compiled branch does **not** clear Harry's look target, play the catch
animation, stop the Snitch's path, move it to Harry's weapon location, set its
owner, or attach it as a trailer. Those actions suggested by embedded source
text are not active compiled behavior.

`GameCatch` is state export 586. Its compiled `BeginState` (export 587):

1. copies `SnitchMaxCatchTries` to `CatchTriesLeft` (default 3);
2. starts a hard-coded 10-second one-shot timer;
3. sends Harry to `Pursue`;
4. stops the Bludgers;
5. calls `QuidHud.PlayHUDGame(true)` and selects the Quidditch HUD game;
6. locks the camera around Harry at distance 200 and target rotation
   `(5000, 5000, 0)`.

`HarryPotter.BroomHarry.Pursue` is state export 862, with `PlayerTick` export
868. It points Harry toward `LookForTarget`, accelerates forward, and chooses
`AirSpeed` from target velocity: 1.0 times under 150 units, 1.25 times under
300 units, and 1.9 times farther away. `LookForTarget` remains the Snitch from
the preceding authored state.

Harry following behind the still-flying Snitch is therefore the intended
background of a timed HUD interaction. It is not a collision-based catch and
does not complete automatically.

### Required timing interaction

`GameCatch.OnActionKeyPressed` (Hub2 export 592) calls
`QuidHud.HUDGameGrab`. On success it stops the Snitch path and calls
`Harry.CatchTarget(Snitch, 'IPHarry_Win')`, then hides the halo, destroys the
popup, stops the opposing Seeker, and enters `GameWon`. On failure it consumes
one of the three tries. Exhausting the tries returns the referee, camera, and
Harry to gameplay. `GameCatch.Timer` (export 593) performs the same fallback
after 10 seconds, and `EndState` (export 594) resets progress to 75.

`HPMenu.QuidHud.HUDGameGrab` (export 1720) delegates to
`QHudGame.Grab`. The active minigame is `HPBase.baseQHudGame`:

- class export 1354 defaults to `iTargetPos = 128`, `iCatchPos = 48`,
  `iAimPoint = 128`, and `fDuration = 30`;
- compiled `Tick` export 3446 moves the target on a 256-pixel bar toward
  randomized aim points at 256 units/s;
- compiled `Grab` export 3451 succeeds only when
  `iTargetPos < iCatchPos`;
- compiled `SetQuidditchMatch` export 3449 selects the Snitch and hand images
  and localizes `catch_snitch_text_02`.

The original interaction is thus: press the action on a fresh edge while the
Snitch icon is within the leftmost 48 pixels, where the open-hand catch zone is
shown. Merely approaching or colliding with the world Snitch cannot win.

`res/System/Pickup.int` identifies the instruction as “Press JUMP key to catch
the Snitch!”. In the shipped retail `DefUser.ini`, the `Jump` alias sets
`bBroomAction`; `Ctrl=Jump`, and right mouse executes `jump` while also holding
the broom boost button. Retail therefore accepts Control or right mouse with
those shipped defaults. `HarryPotter.BroomHarry.PlayerInput` compiled export
937 detects a rising edge on `bBroomAction` and calls the referee action.

OpenHP1's host mapping is deliberately different and broader:

- `crates/openhp1-game/src/app.rs` maps a fresh Space, either Control key, or a
  fresh right-click to the one-frame `PlayerInput.jump` action;
- `crates/openhp1-runtime/src/world/actor/player.rs` writes that action to
  `bBroomAction`;
- the current controls screen advertises **Space or Right Mouse** for Jump.

Once the missing catch UI exists, the clearest current OpenHP1 instruction is
therefore **press Space or right-click when the Snitch icon enters the open
hand**. Control also reaches the action but is not the control displayed by the
OpenHP1 options UI.

## Confirmed OpenHP1 divergence and repair

The minigame object is not created by `GameCatch.BeginState`.
`HPMenu.QuidHud.PostRender` compiled export 1718 owns its lifecycle. While
`bPlayQHUDGame` is true, it lazily spawns `BaseQHudGame` when `QHudGame` is
`None`, assigns the player, configures the appropriate match, and on subsequent
renders calls the minigame's `Paint(Canvas)`. The minigame's Tick supplies the
moving timing target.

OpenHP1 currently does not execute the original UE1 Canvas/PostRender drawing
path. Its host projects selected HUD state through `PlayerUiState`, whose
fields cover health, counters, countdown, bosses, and letters. There is no
Quidditch catch-game state. The game host HUD similarly draws health,
counters, countdown, and boss state, but no hand/Snitch timing bar. No other
runtime or host implementation was found for `QHudGame`, `BaseQHudGame`,
`HUDGameGrab`, `iTargetPos`, or `iCatchPos`.

The pre-fix sequence explains the report exactly:

```text
progress reaches 100
  -> compiled GameCatch starts Harry's authored Pursue behavior
  -> bPlayQHUDGame becomes true
  -> QuidHud.PostRender never runs in OpenHP1
  -> BaseQHudGame is never created, ticked, or displayed
  -> the instruction/open-hand timing zone is invisible
  -> HUDGameGrab has no minigame instance that can report success
```

The repair stays at that shared HUD lifecycle/projection seam. Once
`bPlayQHUDGame` becomes true, the runtime now mirrors `QuidHud.PostRender` by
spawning the shipped `BaseQHudGame`, assigning its `Player`, and dispatching
the compiled match initializer. The ordinary actor tick and compiled `Grab`
continue to own movement, the 10-second phase, the three attempts, and the
success threshold. The host only projects and paints the resulting fields with
the original `HPBase.Icons` textures and `baseWarning` popup state. Pursuit,
collision, and authored speed values are unchanged.

## Reproduction commands

The named exports and compiled bytecode can be reproduced with the repository
inspectors:

```sh
cargo run -q -p openhp1-package --example package_inspect -- res/Maps/Lev2_Quid1.unr
cargo run -q -p openhp1-package --example package_inspect -- res/System/Hub2.u
cargo run -q -p openhp1-package --example package_inspect -- res/System/HPBase.u
cargo run -q -p openhp1-package --example package_inspect -- res/System/HPMenu.u
cargo run -q -p openhp1-package --example package_inspect -- res/System/HarryPotter.u
cargo run -q -p openhp1-script --example script_inspect -- res/System/Hub2.u 557
cargo run -q -p openhp1-script --example script_inspect -- res/System/Hub2.u 587
cargo run -q -p openhp1-script --example script_inspect -- res/System/Hub2.u 592
cargo run -q -p openhp1-script --example script_inspect -- res/System/HPMenu.u 1718
cargo run -q -p openhp1-script --example script_inspect -- res/System/HPBase.u 3451
```

The tagged property/default tables above were decoded read-only through the
existing package/class-default readers. The native routine was inspected with
`llvm-objdump` over `Engine.dll` virtual addresses `0x103f7ba0` onward. No
original file was modified and no extracted game asset is included in this
repository note.
