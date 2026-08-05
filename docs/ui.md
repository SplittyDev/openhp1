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
