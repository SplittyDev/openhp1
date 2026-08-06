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

`FEOptionsPage` uses `HPMenuOptionCombo`, not cycling buttons, for resolution,
colour depth, texture detail, and object detail. Its shipped combo list uses
`FEComboListSmall` for at most three entries, `FEComboListLarge` otherwise,
and stretches `FEComboListBox` across the hovered row. OpenHP1 obtains modern
resolution entries from winit's display modes and exposes only the 32-bit
colour format supported by the wgpu renderer; the detail lists and localized
display names come directly from `HPMenu`.

The Quidditch page's `globalconfig unlocked` value is a three-state progression
stored under `HPMenu.FEQuidMatchPage` in `HP.ini`: `0` locks both choices, `1`
unlocks broomstick practice, and `2` unlocks the league. The authored `Tut2`
`GamePass` state calls `UnlockQuidditch("Broom")` after the first non-replay
lesson pass. The non-league `Hub2` `GameWinning` path calls
`UnlockQuidditch("League")`. These calls cross the host UI bridge and persist
the same value; map names and save-slot progress are not used as proxies.

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
