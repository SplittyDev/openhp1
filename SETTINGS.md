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
[OpenHP1.Renderer]
ResolutionX=1024
ResolutionY=768
WindowSizeX=1280
WindowSizeY=800
Renderer=Classic

[OpenHP1.Renderer.Classic]
Brightness=0.6
ColorMode=32Bit

[OpenHP1.Renderer.Modern]
ToneMapper=Reinhard
Brightness=0.33
Contrast=1.24
AmbientOcclusion=SSAO
AntiAliasing=SMAA
Bloom=true
```

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

Both values in a width and height pair must be valid. OpenHP1 limits each pair
to no more total pixels than 3840x2160. If a pair is incomplete, invalid, or too
large, OpenHP1 uses its default instead.

`ResolutionX` and `ResolutionY` control the game image, not the physical window.
This lets you keep the original 1024x768 presentation inside a larger window.

## `[OpenHP1.Renderer.Classic]`

These settings apply when `Renderer=Classic`.

| Key | Default | Accepted values | What it does |
| --- | --- | --- | --- |
| `Brightness` | `0.6` | `0.2` to `1.0` | Adjusts the image from darker to brighter. Values outside this range are moved to the nearest limit. |
| `ColorMode` | `32Bit` | `32Bit`, `RGB565` | Uses full colour or emulates the original 16-bit RGB565 output. |

For compatibility, `ColorMode` also accepts `32`, `TrueColor`, and `RGBA8888`
as names for `32Bit`, and `16` or `16Bit` as names for `RGB565`. OpenHP1 writes
the preferred `32Bit` or `RGB565` spelling back to the file.

## `[OpenHP1.Renderer.Modern]`

These settings apply when `Renderer=Modern`.

| Key | Default | Accepted values | What it does |
| --- | --- | --- | --- |
| `ToneMapper` | `Reinhard` | `AgX`, `Reinhard`, `ACES` | Chooses how the Modern renderer turns its brighter image into colours your display can show. |
| `Brightness` | `0.33` | `0.2` to `1.0` | Adjusts the image from darker to brighter. Values outside this range are moved to the nearest limit. |
| `Contrast` | `1.24` | `0.5` to `2.0` | Adjusts the difference between dark and bright parts of the image. Values outside this range are moved to the nearest limit. |
| `AmbientOcclusion` | `SSAO` | `Off`, `SSAO`, `XeGTAO` | Chooses the Modern renderer's screen-space contact-shadow method, or disables it. |
| `AntiAliasing` | `SMAA` | `Off`, `FXAA`, `SMAA` | Chooses the Modern renderer's edge-smoothing method, or disables it. |
| `Bloom` | `true` | `true`, `false` | Turns the soft glow around bright areas off or on. |

`Bloom` also accepts `1`, `on`, `0`, and `off`. `ToneMapper=Classic` is accepted
as an older name for `Reinhard`, and `AmbientOcclusion=GTAO` is accepted as a
shorter name for `XeGTAO`.

## Recovering from a bad setting

If OpenHP1 cannot understand a setting, it uses the default for that setting.
To restore every default, close the game and rename or delete `OpenHP1.ini`.
OpenHP1 will generate a fresh copy the next time it starts.
