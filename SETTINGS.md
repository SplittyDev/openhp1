# OpenHP1 settings

OpenHP1 stores its user-editable settings in `OpenHP1.ini`. The file is created
automatically the first time you launch the game, so you do not need to create
it yourself.

## File location

| Operating system | Default location |
| --- | --- |
| Windows | `%APPDATA%\OpenHP1\OpenHP1.ini` |
| macOS | `~/Library/Application Support/OpenHP1/OpenHP1.ini` |
| Linux and other Unix systems | `$XDG_CONFIG_HOME/openhp1/OpenHP1.ini`, or `~/.config/openhp1/OpenHP1.ini` when `XDG_CONFIG_HOME` is not set |

If `OPENHP1_SETTINGS_DIR` is set, OpenHP1 uses that directory instead. This is
mainly useful for portable installations and troubleshooting.

Close OpenHP1 before editing the file. Your changes are read the next time the
game starts. Section names, key names, and named values are not case-sensitive,
so `XeGTAO`, `xegtao`, and `XeGtAo` all mean the same thing.

## Default file

A newly generated file looks like this:

```ini
[OpenHP1.GameData]
Root=/path/to/Harry Potter TM
Language=eng

[OpenHP1.Renderer]
ResolutionX=1024
ResolutionY=768
WindowSizeX=1280
WindowSizeY=800
Renderer=Classic
DetailTextures=false

[WinDrv.WindowsClient]
ScreenFlashes=true

[OpenHP1.Renderer.Classic]
Brightness=0.5
ColorMode=32Bit

[OpenHP1.Renderer.Modern]
ToneMapper=Reinhard
ReinhardBrightness=0.66
ReinhardContrast=1.05
ACESBrightness=0.64
ACESContrast=0.75
AgXBrightness=0.6
AgXContrast=0.9
AmbientOcclusion=XeGTAO
AntiAliasing=SMAA
Bloom=false
VolumetricLighting=false
```

## `[OpenHP1.GameData]`

| Key | Default | Accepted values | What it does |
| --- | --- | --- | --- |
| `Root` | Automatically detected when possible | An absolute directory path | Selects the original game folder containing `Maps` and `System`. |
| `Language` | The first language shipped with the selected game files | A language code offered by the launcher, such as `eng`, `fre`, or `ger` | Selects the original game's localized text, speech, and textures. |

The launcher discovers languages from the selected game files and validates both
values before writing them. When `Root` is absent, OpenHP1 checks for a local
`res` directory and the standard Windows installation directory. A configured
but unavailable root or language produces an error instead of silently selecting
different game files.

## `[OpenHP1.Renderer]`

These settings choose the game resolution, initial window size, and render
pipeline.

| Key | Default | Accepted values | What it does |
| --- | --- | --- | --- |
| `ResolutionX` | `1024` | `320` to `8192` | Sets the width of the internally rendered game image. |
| `ResolutionY` | `768` | `320` to `8192` | Sets the height of the internally rendered game image. |
| `WindowSizeX` | `1280` | `320` to `8192` | Sets the width of the window when OpenHP1 starts. The window remains resizable. |
| `WindowSizeY` | `800` | `320` to `8192` | Sets the height of the window when OpenHP1 starts. The window remains resizable. |
| `Renderer` | `Classic` | `Classic`, `Modern` | Chooses the original-style or enhanced render pipeline. |
| `DetailTextures` | `false` | `true`, `false` | Enables the original three-band close-range detail-texture overlay in both renderers. Macro textures remain enabled independently. |

Both values in a width and height pair must be valid. OpenHP1 limits each pair
to no more total pixels than 3840x2160. If a pair is incomplete, invalid, or too
large, OpenHP1 uses its default instead.

`ResolutionX` and `ResolutionY` control the game image, not the physical window.
This lets you keep the original 1024x768 presentation inside a larger window.

## `[WinDrv.WindowsClient]`

| Key | Default | Accepted values | What it does |
| --- | --- | --- | --- |
| `ScreenFlashes` | `true` | `true`, `false` | Shows authored viewport flashes and fades. When false, their runtime timing continues but the rendered image is unchanged. |

`ScreenFlashes` also accepts `1`, `on`, `0`, and `off`.

## `[OpenHP1.Renderer.Classic]`

These settings apply when `Renderer=Classic`.

| Key | Default | Accepted values | What it does |
| --- | --- | --- | --- |
| `Brightness` | `0.5` | `0.2` to `1.0` | Adjusts the image from darker to brighter. Values outside this range are moved to the nearest limit. |
| `ColorMode` | `32Bit` | `32Bit`, `RGB565` | Uses full colour or emulates the original 16-bit RGB565 output. |

For compatibility, `ColorMode` also accepts `32`, `TrueColor`, and `RGBA8888`
as names for `32Bit`, and `16` or `16Bit` as names for `RGB565`. OpenHP1 writes
the preferred `32Bit` or `RGB565` spelling back to the file.

## `[OpenHP1.Renderer.Modern]`

These settings apply when `Renderer=Modern`.

| Key | Default | Accepted values | What it does |
| --- | --- | --- | --- |
| `ToneMapper` | `Reinhard` | `AgX`, `Reinhard`, `ACES` | Chooses how the Modern renderer turns its brighter image into colours your display can show. |
| `ReinhardBrightness` | `0.66` | `0.2` to `1.0` | Sets brightness when using Reinhard. Values outside this range are moved to the nearest limit. |
| `ReinhardContrast` | `1.05` | `0.5` to `2.0` | Sets contrast when using Reinhard. Values outside this range are moved to the nearest limit. |
| `ACESBrightness` | `0.64` | `0.2` to `1.0` | Sets brightness when using ACES. Values outside this range are moved to the nearest limit. |
| `ACESContrast` | `0.75` | `0.5` to `2.0` | Sets contrast when using ACES. Values outside this range are moved to the nearest limit. |
| `AgXBrightness` | `0.6` | `0.2` to `1.0` | Sets brightness when using AgX. Values outside this range are moved to the nearest limit. |
| `AgXContrast` | `0.9` | `0.5` to `2.0` | Sets contrast when using AgX. Values outside this range are moved to the nearest limit. |
| `AmbientOcclusion` | `XeGTAO` | `Off`, `SSAO`, `XeGTAO` | Chooses the Modern renderer's screen-space contact-shadow method, or disables it. |
| `AntiAliasing` | `SMAA` | `Off`, `FXAA`, `SMAA` | Chooses the Modern renderer's edge-smoothing method, or disables it. |
| `Bloom` | `false` | `true`, `false` | Turns the soft glow around bright areas off or on. |
| `VolumetricLighting` | `false` | `true`, `false` | Turns depth-aware atmospheric scattering around authored lights off or on. |

`Bloom` and `VolumetricLighting` also accept `1`, `on`, `0`, and `off`.
`ToneMapper=Classic` is accepted as an older name for `Reinhard`, and
`AmbientOcclusion=GTAO` is accepted as a shorter name for `XeGTAO`. Older shared
`Brightness` and `Contrast` values are used for the selected tone mapper when
its named values are not present.

## Recovering from a bad setting

If OpenHP1 cannot understand a setting, it uses the default for that setting.
To restore every default, close the game and rename or delete `OpenHP1.ini`.
OpenHP1 will generate a fresh copy the next time it starts.
