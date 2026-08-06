# Quidditch lesson exit behavior in the original game

The flying lesson in `Lev_Tut2.unr` has two deliberately different exit
paths. Normal story progression does not use Escape: after Harry passes, the
game runs a scripted exit cutscene and travels to `Lev_Tut3.unr`. The lines
that tell Harry to press Escape belong only to practice launched directly from
the Quidditch menu. In that mode Escape opens a quit confirmation and returning
to the main menu is the intended exit.

This note is based on the active compiled exports, embedded UnrealScript, map
actor properties, and localization shipped in `res/`. Embedded source is used
to name constants and arguments, but consequential branches are also checked
against compiled bytecode because embedded text can contain inactive lines.

## Reproducing the evidence

```sh
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-package \
  --example package_inspect -- res/System/Tut2.u
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- res/System/Tut2.u 102
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- res/System/Tut2.u 109
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- res/System/Tut2.u 110
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- res/System/Tut2.u 113
env RUSTC_WRAPPER= cargo run --release -q -p openhp1-script \
  --example script_inspect -- res/System/Tut2.u 114
strings -a -n 4 res/System/Tut2.u
strings -a -n 4 res/System/HPBase.u
strings -a -n 4 res/System/HPMenu.u
sed -n '900,916p' res/System/hpdialog.int
sed -n '48,66p' res/System/DefUser.ini
```

The map evidence below comes from a read-only tagged-property decode of
`Lev_Tut2.unr`; the original package was not extracted or modified.

## How the game chooses story or practice mode

`Tut2.u` `BroomPracticeReferee.OnPlayerPossessed`, active function export 72,
sets `bReplay` from its `PlayMode`:

- `PM_Auto` uses `!HPConsole(Console).bInHubFlow`.
- `PM_InHubFlow` forces `bReplay = false`.
- `PM_MenuDirect` forces `bReplay = true`.

`HPBase.u` `GameReferee.ScriptText` defines `PM_Auto` as enum value zero.
`Lev_Tut2.unr` actor `BroomPracticeReferee0`, export 526, has no serialized
`PlayMode` override, so it uses `PM_Auto`.

The frontend establishes the distinction. `HPMenu.u`
`FEQuidMatchPage.Notify`, active export 1659, handles the practice button by
setting `FEBook.bPlayingQuidditch = true` and calling
`RunURL("Lev_Tut2.unr", false)`. `FEBook.Tick`, active export 1079, copies that
false travel-items argument into `HPConsole.bInHubFlow` before travel. Normal
storybook and level-select launches call `RunURL(..., true)`, so story flow
sets `bInHubFlow = true`.

Therefore, hearing `hootch_new_12` or `hootch_new_13` during normal story
progression is not the original first-completion behavior. It means the lesson
has been classified as menu-direct practice because the launch's hub-flow state
was not preserved.

## First completion in story flow

After any passing result cutscene triggers the referee,
`BroomPracticeReferee.GamePass.OnCutSceneEvent`, active export 102, adds the
earned points and hides the hoop HUD. Its compiled branch goes to
`GameReplay` only when `bReplay` is true; otherwise it goes to `GameExit`
(possibly after the optional `GameSecret` cutscene; active export 106 has the
same replay-versus-exit decision).

`GameExit.BeginState`, active export 113, calls `TriggerEvent('Exit')`. This
starts the map's `CutScene14`, `Lev_Tut2.unr` export 817, whose serialized tag
is `Exit`. The authored scripts identify the visible sequence precisely:

- cast 2 is `BroomHooch1`. It says `hootch_new_10` (practice is now unlocked),
  then `HOOCH_026`: “Time for your Charms lesson, now. Good day, Mr. Potter.”
- cast 3 is `CutHarry0`. After the `TeleportHarry` cue, it teleports to
  `cutmark10`, moves to `cutmark10`, faces Hooch, waits for
  `HoochDoneTalking`, moves to `cutmark5`, emits the `Changelevel` cue, and
  continues toward `cutmark6`. This is the authored automatic run toward and
  through the exit; it is not player input.
- cast 0 waits for `Changelevel`, sleeps one second, then executes
  `changelevel lev_tut3.unr`.

`HPBase.u` `CutScene.handleCast`, active export 3563, parses `CHANGELEVEL` as
`CUT_CHANGELEVEL` and dispatches
`baseConsole(playerHarry.player.console).ChangeLevel(var1, true)`. Thus the map
cutscene itself owns the transition. `GameExit.OnCutSceneEvent`, active
`Tut2.u` export 114, also contains a compiled
`ChangeLevel("Lev_Tut3.unr", true)` call; the following direct `ServerTravel`
line visible in embedded source is absent from this active bytecode export.

The next map is therefore unambiguously `Lev_Tut3.unr`, the Wingardium Leviosa
lesson. Escape is not part of first-completion progression. If `HOOCH_026`
plays but Harry does not move, the exit cutscene has started and the remaining
gap lies in its cue/`CutMoveTo`/`CHANGELEVEL` execution. If a press-Escape line
plays instead, the earlier `bInHubFlow`/`bReplay` mode decision is already
wrong.

## Repeat and menu-practice behavior

For menu-direct practice, the same passing branch enters `GameReplay`.
`GameReplay.BeginState`, active export 109, triggers either `Replay` or
`ReplayAltPath`. The corresponding serialized map actors are:

- `Lev_Tut2.unr` `CutScene22`, export 827, tag `Replay`, which says
  `hootch_new_17` and `hootch_new_12`.
- `Lev_Tut2.unr` `CutScene23`, export 826, tag `ReplayAltPath`, which says
  `hootch_new_9` and `hootch_new_13`.

`hpdialog.int` localizes `hootch_new_12` as “Have another go at it, if you'd
like; press <ESC> if you'd rather leave,” and `hootch_new_13` as the equivalent
line for a different set of hoops. Both cutscenes end with
`Trigger GameReferee`; `GameReplay.OnCutSceneEvent`, active export 110,
immediately returns to `GameTrial`. Repeating is therefore automatic unless
the player opts out with Escape.

Although `DefUser.ini` contains `Escape=quit`, the shipped console consumes the
key first. `HPConsole.KeyEvent`, active `HPMenu.u` export 1832, handles an
`IK_Escape` press by calling `MenuBook.EscFromConsole()` and returning true.
`FEBook.EscFromConsole`, active export 1021, checks `bPlayingQuidditch`; for a
menu-launched practice or match it opens the UWindow frontend and displays the
localized Yes/No prompt “Are you sure you want to quit this game?”

`FEBook.WindowDone`, active export 1033, handles the response. Yes clears
`bGamePlaying` and returns to `MainPage`; No closes the book when Quidditch is
active and resumes play. It does not load `Lev_Tut3.unr`. A pause/frontend
screen or confirmation after Escape is consequently expected for replay; it
must not be reused as the story lesson's exit.

## OpenHP1 host-flow requirement

OpenHP1's native frontend does not run `FEBook.RunURL`, so the game host must
seed the emulated `HPConsole.bInHubFlow` value before map startup. A level tied
to a save slot is normal story flow; a direct level launch from the Quidditch
menu is not. This distinction must follow authored level travel as well as new
games and loaded saves.

The host-flow value must be correct when the initial player `Possess` event
runs. `BroomPracticeReferee.OnPlayerPossessed` not only derives `bReplay`; it
also triggers either `Intro` or `ReplayIntro`. Repeating `Possess` after save
restoration would start a second intro cutscene and leave competing cast and
camera state active.

A save written after the wrong mode was selected contains the wrong cutscene's
entire actor state, not only a stale `bReplay` boolean. Such a checkpoint must
be regenerated at the same level entrance under the correct host-flow mode;
changing the boolean or replaying lifecycle events is not a safe migration.
