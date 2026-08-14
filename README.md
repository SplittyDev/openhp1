# Open Harry Potter 1
An open-source re-implementation of the first Harry Potter game.

Experience the first Harry Potter PC game as if you were sitting in front of your old Windows 98/XP PC, but on current hardware and operating systems, and without the glitches and difficulties that come with installing and running an old game.

![Logo](./splash.jpg)

OpenHP1 requires the original game files to run.

## Features


A lot of stuff is already working very well. The main menu, intro sequence, first levels, spell learning sequences, Quidditch lessons and more already work with only minor issues and are fully playable. Some animations might not look exactly as intended and there might be minor graphical or behavioral differences, but none of them (to my knowledge) are game-breaking. For all intents and purposes, you can simply play the game.

### Goodies
While our primary goal is to keep as true as possible to the original game experience, there are lots of nice optional tweaks and additions that make the game nicer to play on modern hardware.
The graphics settings in particular have been replaced by a fully custom menu with a lot more options.

#### Resolutions
OpenHP1 supports a wide range of resolutions, from the original low-res 4:3 to modern high-fidelity widescreen formats. The internal resolution is fully independent of window size.

| Kind | Description |
| ---- | ----------- |
| Classic| Legacy resolutions of the original game, keeping a true 4:3 aspect ratio. |
| Enhanced | Same as Classic, but at higher resolutions that didn't exist in the original game. |
| Widescreen | Fills the window, showing more of the level on the sides instead of forcing 4:3. |

#### Renderers
OpenHP1 ships with multiple configurable renderers, so you can get exactly the look you want.

| Name | Description |
| ---- | ----------- |
| Classic | True to the original game, with optional 16-bit emulation. |
| Modern | HDR, Tone Mapping, Brightness/Contrast, Ambient Occlusion and more. |

**What is 16-bit emulation?**  
While the game defaults to 32-bit, allowing it to display a wide range of colors, we also ship an emulated 16-bit mode (RGB565). This essentially compresses the available color spectrum to approximate the look of the game on old 16-bit hardware.

**Which renderer should I use?**  
If you like the look of the original game, just keep the default. The classic renderer very closely approximates the look of the original game running on old hardware. It's fast, simple, and renders the game in the way it was supposed to be rendered.

The modern renderer is exactly what it says on the package. A much more modern renderer with fancy post-processing, anti-aliasing, ambient occlusion, bloom and more. It's certainly not what the game looked like back then, but everything is carefully fine-tuned to not change the look and feel too much while still delivering an elevated experience worthy of modern hardware.

## Installation
> This is heavily work-in-progress, and no pre-built binaries are available yet.

OpenHP1 requires original game data but does not distribute it. Start
`openhp1-launcher`, choose the original game folder containing `Maps` and
`System`, select one of its available languages, then select **Play**. The
launcher remembers the validated folder and language in `OpenHP1.ini`.

For building the game yourself, please refer to the [Development](#development) section below.

## Development

### Prerequisites

Place the original game files in the `res/` directory. It should look something like this:
```
res/
├── Maps/
│   ├── Lev_Tut1.unr
│   ├── Lev_Tut2.unr
│   └── ...
├── Music/
├── Sounds/
├── ...
```

You will also need a modern Rust toolchain installed. You can install Rust with [rustup](https://rustup.rs/).

### Map Viewer
The OpenHP1 viewer is a cross-platform, real-time 3D renderer for the original Harry Potter 1 maps and actors.
It does not implement the full game logic, but it lets you fly through the original levels and inspect the actors, meshes, and textures.

You can build and run the viewer with:

```sh
cargo run -p openhp1-viewer -- res/Maps/Lev_Tut1.unr # debug build
cargo run -p openhp1-viewer --release -- res/Maps/Lev_Tut1.unr # release build
```

Drag over the viewport to look around. Use `WASD` to move, `Q`/`E` to move
down/up, and hold Shift to move faster. The Actors panel searches the current
level's actors and shows their identity, decoded state, render ranges, and
diagnostics. You can switch levels within the viewer.

### Game
The OpenHP1 game is a re-implementation of the original game logic. It intends to faithfully reproduce the original gameplay.

You can build and run the game with:

```sh
cargo run -p openhp1-launcher # launcher and game-data configuration
cargo run -p openhp1-game # debug build
cargo run -p openhp1-game --release # release build
```

Hold + (main keyboard or numpad) or F to run the normal game simulation at 16x speed. This debug
fast-forward still processes scripts, animation completions, triggers, and
other runtime events in order.

Press the `` ` `` key to open the bottom developer console, enter commands, and
press it again to return to the game. The console keeps command/output history
for the current session; use up/down to revisit commands and `help` to list
registered commands.

### Docs

See [`docs/`](docs/) for the reverse-engineered package, map, texture, and
rendering notes.

### Resources

Original game data may remain in an external folder selected through the launcher.
For development, the gitignored `res/` directory is also detected automatically.
Original game data must never be committed or distributed with OpenHP1.
