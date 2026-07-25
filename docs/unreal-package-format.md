# Unreal Engine 1 package format in Harry Potter 1

This document records behavior observed in a legally obtained local HP1
installation. It intentionally describes structure without reproducing
copyrighted game data.

## One container, several extensions

The files conventionally named `.unr`, `.utx`, `.uax`, `.umx`, and `.u` share
the same Unreal package container. Their extensions describe expected contents,
not different binary envelopes:

- maps normally export `Level` and `Model` objects;
- texture packages export `Texture` and `Palette`;
- sound packages export `Sound`;
- music packages export `Music`;
- system packages export classes, functions, defaults, bytecode, text buffers,
  and sometimes ordinary assets.

All inspected packages begin with the little-endian value `0x9E2A83C1`.

## Header variants

The local corpus spans package versions 61, 68, 69, 72, 73, 75, and 76.
Supporting only the most frequent version 76 would reject original assets such
as `Detail.utx` (61) and `Entry.unr` (72).

A magic-based scan of the local installation found 248 packages containing
139,773 name entries, 11,520 imports, and 125,110 exports. Their version
distribution is:

| Version | Packages |
| ---: | ---: |
| 61 | 6 |
| 68 | 3 |
| 69 | 16 |
| 72 | 1 |
| 73 | 2 |
| 75 | 13 |
| 76 | 207 |

The common header prefix is:

| Field | Encoding |
| --- | --- |
| Magic | `u32` |
| Package version | `u16` |
| Licensee version | `u16` |
| Package flags | `u32` |
| Name count and offset | two `i32` values |
| Export count and offset | two `i32` values |
| Import count and offset | two `i32` values |

Before version 68 the prefix is followed by a heritage-table count and offset.
Version 68 and later instead store a 16-byte package GUID, a generation count,
and pairs of export/name counts for each generation.

## Names

Before package version 64, name entries use a zero-terminated byte string.
Later versions prefix the string with a compact signed length. A positive
length counts single-byte characters including the terminator; a negative
length counts little-endian UTF-16 code units including the terminator. Each
name is followed by a `u32` flags field.

Package objects refer to this table by compact name index. Names behave as
identifiers in the engine and should be resolved without imposing the host
filesystem's case rules.

## Compact indices

UE1 frequently stores signed 32-bit values in one to five bytes:

- bit 7 of the first byte is the sign;
- bit 6 says another byte follows;
- the first byte contributes six value bits;
- bytes two through four contribute seven value bits and use bit 7 as their
  continuation bit;
- the fifth byte contributes only four value bits.

Small values and references therefore occupy one byte. The parser rejects
negative zero, a continuation after the fifth byte, and non-zero unused bits.

## Object references

Serialized object references are signed compact indices:

- `0` means no object;
- `n > 0` means export `n - 1`;
- `n < 0` means import `abs(n) - 1`.

This representation naturally forms cyclic graphs through imports, exports,
and outer objects. OpenHP1 represents references as stable indices rather than
long-lived Rust borrows.

## Index tables

The import table identifies an object's class package, class name, outer
object, and object name.

The export table identifies its class, superclass, outer object, object name,
flags, and serialized payload range. The payload is class-specific; parsing
the table does not imply that the payload can be interpreted.

Every count, offset, and payload range is validated before allocation or
slicing. Unsupported payload classes remain inspectable instead of being
silently interpreted with a guessed layout.

## Package discovery

The installation's INI configuration lists package search globs under
`[Core.System] Paths`. Localized assets do not always end in their conventional
extension, including examples ending in `.int_uax`, `.spa_uax`, and
`.hun_utx`. Discovery should follow the configured paths, compare package names
case-insensitively, and confirm the package magic rather than filtering solely
by extension.
