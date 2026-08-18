# Game UI

OpenHP1 boots `Maps/startup.unr` by default. All shipped language variants of
`System/Default.ini` set `Engine.Engine.LocalMap=Startup.unr`; `Entry.unr` is
the engine's empty transition map and has no `PlayerStart` or authored startup
sequence.

The original UI is split across two existing owners:

- `startup.unr` owns the 3D logo/cutscene scene.
- `HPMenu.HPConsole` launches `FEBook` on its first tick. `FEBook` owns the
  splash, main, slot, report, folio, chapter, save, options, and Quidditch
  pages. `HPHud` and its `baseHud` parent own gameplay indicators.

The implementation order follows those dependencies:

1. Boot `startup.unr` and preserve its authored runtime sequence.
2. Render the shipped splash/main-menu assets and reproduce `FEBook` page
   transitions and input behavior.
3. Connect slot selection to the existing save/open actions, then implement
   options through the existing config/settings owner.
4. Reuse the same book shell for pause/report, Folio Magi, chapter/storybook,
   and save pages.
5. Project `HPHud` state into Canvas-compatible draw commands for health,
   beans, stars, house points, seeds, messages, timers, enemy health, and the
   Quidditch/flying HUD variants.

UI presentation stays in `openhp1-game`, the sole window/input owner. Runtime
state and console actions stay in `openhp1-runtime`; original package texture
decoding stays in `openhp1-texture`. A separate UI crate is not warranted
unless a second executable needs the complete game UI.

Every successful transition from the front end or a save into gameplay must
recapture the reused winit window. `Graphics` rebuilds reset `InputState`, so
leaving it uncaptured would discard gameplay keys and mouse buttons while the
global Escape handler continued to work.

Selecting or creating one of the six game slots also establishes the active
slot used by the authored `SaveSelectedSlot` and `LoadSelectedSlot` console
calls. Their host marker `99` resolves to that active slot; direct level starts
without a slot retain `save99.usa` as the shipped `FESlotPage` fallback.
Creating or replacing a slot follows `FESlotPage.CreateSelectedSlot`: it plays
the 14 compiled pages of story 3 before loading `Lev_Tut1.unr`. The four-piece
page art comes from `StoryBookTest`, narration and localized captions come from
the matching `AllDialog`/`HPDialog` entries. The compiled `FEStoryBookPage`
defaults show each page for 1.9 seconds before starting narration in
`SLOT_Interact`, then leave 1.1 seconds after its decoded sound duration before
advancing. `HPConsole.ChangeLevel` marks the destination as newly loaded, and
its first destination tick saves the selected slot before play continues.
OpenHP1 performs that same entry save after a new game and each authored level
travel, so quitting during the opening lesson resumes at the entered level
rather than replaying the storybook.

The shipped `FEOptionsPage` exposes resolution, colour depth, texture detail,
object detail, and brightness directly. OpenHP1 replaces that Video block with
one Graphics Settings button while retaining the authored controls, audio,
navigation, and book layout around it. The separate OpenHP1-owned page exposes
the internal render resolution and Classic/Modern renderer selection. Classic
adds brightness and optional final-frame RGB565 emulation; Modern adds its own
brightness and contrast values for each tone mapper, Off/SSAO/XeGTAO
selection, Off/FXAA/SMAA anti-aliasing, bloom, and volumetric lighting. Classic
remains the default. Brightness and contrast sliders show their numeric value
and a reset arrow that restores the active renderer or tone mapper's default.
The original texture and object detail controls are omitted
because the OpenHP1 renderer does not currently consume those configuration
values.

Graphics changes preview immediately and are persisted together under
the `[OpenHP1.Renderer]`, `[OpenHP1.Renderer.Classic]`, and
`[OpenHP1.Renderer.Modern]` sections in the writable `OpenHP1.ini` overlay.
OpenHP1 writes every setting on first launch so the internal resolution, window
size, and renderer options can also be edited directly. Named values are
case-insensitive. `ColorMode=32Bit` selects the unfiltered output and
`ColorMode=RGB565` selects the current 16-bit emulation, leaving distinct names
available for future colour filters. Changes are saved when the page closes.
The defaults are a 1024x768 internal frame, a 1280x800 window, the Classic
renderer, 32-bit colour, and the midpoint Classic brightness. They survive
authored level travel and save loading. The resolution list keeps the
four shipped 4:3 sizes, adds enhanced 4:3 and 16:9 presets, and retains an active
non-preset value as Custom. This setting controls the composed internal frame
rather than the independently resizable winit window, which starts at 1280x800.

The Quidditch page's `globalconfig unlocked` value is a three-state progression
stored under `HPMenu.FEQuidMatchPage` in `HP.ini`: `0` locks both choices, `1`
unlocks broomstick practice, and `2` unlocks the league. The authored `Tut2`
`GamePass` state calls `UnlockQuidditch("Broom")` after the first non-replay
lesson pass. The non-league `Hub2` `GameWinning` path calls
`UnlockQuidditch("League")`. These calls cross the host UI bridge and persist
the same value; map names and save-slot progress are not used as proxies.
The value belongs in the main OpenHP1 settings directory, not its `Saves`
subdirectory. Builds that wrote it there are detected and migrated on startup.
At level `2`, `FEQuidMatchPage` starts the compiled six-round schedule against
Slytherin, Ravenclaw, and Hufflepuff on the shipped A maps and then the three B
return fixtures. The in-game `FinishGame` call reports Gryffindor's score back
through the host UI bridge. The page applies the original parallel-match score
simulation and standings rules, selects the leading two houses, and uses the
compiled `Quid_*C.unr` table when Gryffindor reaches the final. This league
state follows authored level travel within the current session, as the original
long-lived `FEQuidMatchPage` object did.

During gameplay, Escape opens the authored `FEReportPage` rather than merely
releasing the cursor. The page reads Harry's live `lifePotions`, beans, wizard
cards, personal points, and four house totals from the runtime instance used by
UnrealScript and save games. Its `FEReportBackTexture*`, badge,
sand, and coloured button textures and their coordinates come from the shipped
`HPMenu.u` class. Resume closes the book, Options returns to the report page,
and Quit Game returns to `startup.unr` after the localized confirmation.

`FEFolioPage` reads the same 25-element `WizardCards` struct array as
UnrealScript. Its six normal and six Harry-page backgrounds, missing-card art,
25 small/large card pairs, page arrows, card positions, and seven-page layout
come from `HPMenu.u`; collected IDs select the matching shipped art and
localized `wizard_card_new_*` description. The original `FEBook.ShowTabs`
returns before enabling its dormant side tabs, so OpenHP1 follows the active
Report button and return-arrow navigation instead of displaying them.

The gameplay overlay uses the compiled `HPHud`/`baseHudItem` defaults: Harry's
full/empty health art remains at the upper left, while fire seeds, stars, house
points, and beans use the shipped counter textures at x=160, 320, 480, and 480
on the 640-wide reference canvas. The counters appear for five seconds when
their corresponding live Harry field changes, matching each authored `Show()`
call; bean totals also layer the four original bean-pile textures. The same
runtime projection reads `baseHud.bCountingDown`, `fCountdownTime`, and
`fStartCountdown` for the lower-right spell-learning timer. `HPHud` enemy
health follows Harry's live `BossTarget`: Quirrell/Voldemort, Peeves,
BossRailMove/BroomDraco, and Fluffy use their shipped bar art and authored
health fields, including the separate awake/asleep state of Fluffy's three
heads.
