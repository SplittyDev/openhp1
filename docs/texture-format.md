# HP1 palette and texture exports

HP1's ordinary UE1 textures use 8-bit palette indices. A `Texture` export
refers to a separate `Palette` object and stores one byte per texel at each
mipmap level.

## Tagged properties

UObjects begin with a list of tagged properties terminated by the engine name
`None`. A tag contains:

1. a compact name-table index;
2. an info byte containing a property-type nibble, size code, and array flag;
3. an optional extended payload size;
4. a struct-name index for struct properties;
5. an optional compact array index;
6. the property payload.

Boolean properties are exceptional: their value occupies the tag's array bit
and they have no payload bytes.

Texture properties observed in HP1 include `Palette`, `USize`, `VSize`,
`UClamp`, `VClamp`, `UBits`, `VBits`, `MipZero`, `MaxColor`, and
`InternalTime`. The decoder reads the references and dimensions it needs and
skips the remaining tagged values by their declared sizes.

## Palette

After its tagged properties, a `Palette` export stores:

1. a compact color count, normally 256;
2. that many four-byte colors in red, green, blue, alpha byte order.

The stored alpha byte is normally not the surface transparency. Masked UE1
surfaces conventionally make palette index zero transparent; ordinary surfaces
render every palette entry as opaque.

## Texture mipmaps

After its tagged properties, a `Texture` stores an 8-bit mip count followed by
each mip:

| Field | Encoding |
| --- | --- |
| Lazy data end | absolute `u32` package offset |
| Palette-index count | compact index |
| Palette indices | one byte per texel |
| Width | `u32` |
| Height | `u32` |
| Width bits | `u8` |
| Height bits | `u8` |

The lazy-array end equals the absolute stream position immediately after the
palette indices. OpenHP1 validates this instead of treating it as padding.
For the uncompressed P8 textures decoded so far, the index count equals
`width * height`.

## WetTexture source scaling

`WetTexture` renders its water refraction over the paletted image named by
`SourceTexture`. The shipped `Fire.dll` creates a target-sized
`LocalSourceBitmap` when the wet texture is at least as large as its source in
both dimensions. Each source texel is expanded by the power-of-two size ratio;
for example, `HP_Water.Water.water2` expands its 128x128 `TBwater1` source to
256x256. If either source dimension is larger than the wet texture, the engine
discards the source instead. An accepted source also supplies the wet texture's
palette.

OpenHP1 performs the same nearest-neighbor expansion on palette indices before
starting the water simulation. Resizing RGBA output instead would bypass the
palette-index refraction performed by the original engine.

## WaveTexture and the shared water core

`UWaveTexture` is not a renamed `WetTexture`. Both inherit the native
`UWaterTexture` simulation: two half-resolution byte fields, a 1,536-byte
water table, parity-switched kernels, eight-byte drop records, and Fire's
process-global RNG. Wave maps the resulting gradients through its own exact
1,024-byte lighting table into base-mip palette indices; Wet applies a distinct
source-refraction output.

OpenHP1's current full-resolution float water model and fixed 30 Hz accumulator
are approximations. Exact replacement is tracked as `BASE-009C`. The optimized
retail kernels at [`Ghidra_Fire.c:12658`](../res/Ghidra_Fire.c#L12658) have
corrupted decompiler aliases, so their scalar equations must come from shipped
x86 disassembly or complete injected-state golden vectors rather than guessed
from the invalid C output.

The only shipped Wave export is `Detail.WaterDE2`. It is referenced as
`DetailTexture` by twelve unused `Liquids` textures, and a full-package scan
finds no map or class import of those owners. This removes a shipped live test
case, not the engine-compatibility requirement.

## Macro and detail attachments

Regular texture exports may carry independent `MacroTexture` and
`DetailTexture` object references. OpenHP1 preserves both through the shared
BSP material path. Attachment palettes are expanded without the base
surface's masked-index-zero rule because the shipped D3D attachment calls pass
zero poly flags to `SetTexture`; this also means the auxiliary draws do not
alpha-test against the base image. Their normalized coordinates remove the BSP
pan already present in the mesh coordinates, then apply the attachment's own
dimensions and `DrawScale`, matching `FTextureInfo`. Macro and detail always
sample smoothly even when the base texture authors `bNoSmooth`. Macro adds the
native half-texel center offset; detail does not. The base texture's current
generic-animation frame owns the two attachment references, while only the
non-null raw root `FBspSurf.Texture`'s authored `bPortal` contributes to stable
portal classification and detail suppression; a raw-null surface does not
inherit portal state from `LevelInfo.DefaultTexture`. When `AnimCurrent` changes, OpenHP1
switches the material attachment identities and UV normalization for the newly
bound dimensions without changing portal state. The selected attachment object
is locked directly, so its own `AnimNext` chain is not followed.

The shipped corpus has no reachable owner of either attachment: the 24
non-null detail properties are confined to otherwise-unused texture exports,
and no non-null macro property exists. Synthetic checks therefore protect the
decode, UV, pass-order, saturation, and detail-band equations.
