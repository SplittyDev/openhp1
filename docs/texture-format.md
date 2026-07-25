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
2. that many four-byte colors in blue, green, red, alpha byte order.

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

