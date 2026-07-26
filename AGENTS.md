# OpenHP1 Agent Guide

## Project goal

OpenHP1 is an open-source Rust reimplementation of the original Harry Potter
and the Philosopher's/Sorcerer's Stone PC game, which was built on Unreal
Engine 1.

The implementation must load and run files from a legally obtained original
game installation. Do not replace original maps, textures, meshes, sound,
music, or game scripts with committed copies. The long-term goal is to run the
original packages unmodified.

The current milestone is a cross-platform 3D map viewer. Target macOS and
Windows first, while keeping Linux working where practical.

## Repository and copyright rules

- `res/` contains the local original game installation and is gitignored.
- Never commit original files, extracted assets, embedded script source,
  generated asset dumps, or binary fixtures copied from the game.
- Public tests must use small synthetic packages or data constructed by the
  test itself.
- Local integration tests may read `res/`, but must not require it in public
  CI and must never modify it.
- Do not use leaked proprietary source code. Reverse engineering must be based
  on behavior, documented formats, the legally obtained binaries/packages, and
  properly licensed reference projects.
- Keep a clear non-affiliation/trademark notice when preparing public releases.
- Reference implementations may be studied subject to their licenses:
  - SurrealEngine: <https://github.com/dpjudas/SurrealEngine>
  - UE Viewer: <https://github.com/gildor2/UEViewer>
- A local SurrealEngine clone is available at
  `/Users/splitty/Developer/SurrealEngine`; use it instead of repeatedly
  downloading temporary copies.

## Local file-format facts

The main Unreal extensions are not independent container formats. `.unr`,
`.utx`, `.uax`, `.umx`, and `.u` all use the Unreal package container, starting
with little-endian magic `0x9E2A83C1`. The local corpus contains package
versions from 61 through 76, so do not implement only version 76.

Use extensions as hints about conventional contents, not as parser boundaries:

| Extension | Conventional contents |
| --- | --- |
| `.unr` | `Level`, `Model`, BSP geometry, actors, and package references |
| `.utx` | `Texture`, `Palette`, mipmaps, and related texture objects |
| `.uax` | `Sound` objects, commonly containing WAV payloads |
| `.umx` | `Music` objects containing an embedded music payload |
| `.u` | Classes, compiled UnrealScript bytecode, defaults, metadata, and resources |
| `.int` | INI-like localization and public object registration data |

Some `.u` packages in this installation visibly contain embedded UnrealScript
source text as well as compiled data. That source is still copyrighted game
data and must not be committed.

Localized files include unusual suffixes such as `.int_uax`, `.spa_uax`, and
`.hun_utx`. Package discovery must therefore use the configured search paths
and package magic rather than trusting only the final filename extension.

The original configuration's `[Core.System] Paths` entries define the package
search locations. Unreal package names and Windows-era filenames must be
resolved case-insensitively on every host, including case-sensitive Linux
filesystems.

`Entry.unr` is a tiny version-72 parser fixture and `startup.unr` is also too
small to prove map rendering. A roughly 270 KB Quidditch map is a better first
real visual target.

## Architecture

Build one checked Unreal package/object foundation and place typed object
decoders above it. Do not write separate binary container parsers for every
extension.

The initial workspace should contain only crates needed by the map viewer:

```text
crates/
  openhp1-package/  Package archive, header, names, imports, exports,
                    compact indices, object references, properties, resolver
  openhp1-map/      Level, Model, BSP, surfaces, vertices, and actors
  openhp1-mesh/     Mesh, LodMesh, and SkeletalMesh geometry decoding
  openhp1-texture/  Palette, Texture, mipmaps, and pixel conversion
  openhp1-scene/    Package-backed CPU scene assembly and render-ready data
  openhp1-render/   wgpu renderer and camera
  openhp1-viewer/   eframe application and egui inspection UI
```

Keep the dependency direction acyclic:

```text
openhp1-package ──┬── openhp1-map ─────┐
                  ├── openhp1-mesh ────┼── openhp1-scene ── openhp1-render
                  └── openhp1-texture ─┘                         │
                                                                └── openhp1-viewer
```

Add these only when their implementation begins:

- `openhp1-script`: class metadata, bytecode decoding, and the UnrealScript VM
- `openhp1-audio`: sound/music extraction, decoding, and playback integration
- `openhp1-animation`: reusable skeleton/sequence playback, pose sampling, and
  animation state once animated actors are implemented; package-specific mesh
  and sequence decoding remains in `openhp1-mesh`
- `openhp1-runtime`: object lifecycle, ticking, gameplay, collision, and game state
- A separate game executable that owns the real game loop

Do not create empty crates or abstraction layers for speculative future work.
Split an existing crate when it has a real independent responsibility or
dependency boundary, not merely because it has grown.

The renderer consumes decoded render data. It must not know about package
paths, byte offsets, import/export tables, or filesystem resolution.

Keep crate roots as small module indexes or executable entry points. The
current internal responsibility split is:

| Crate | Modules |
| --- | --- |
| `openhp1-package` | checked archive cursor, object properties, package owner, configured package resolver, public summary types, index-table decoding, errors |
| `openhp1-map` | BSP records, shared decode checks, `Level`, `Model`, sky-zone actors, triangulation, static lightmaps, actor vertex lighting, errors |
| `openhp1-mesh` | classic, LOD, and skeletal mesh records, decoding, and geometry conversion |
| `openhp1-texture` | palette decoding, texture/mip decoding, shared decode checks, errors |
| `openhp1-scene` | package-backed scene loading, actor/material assembly, coordinate conversion, render-ready CPU scene data |
| `openhp1-render` | camera/bounds, GPU batching, pipelines, lightmap atlas, render targets, wgpu renderer |
| `openhp1-viewer` | executable startup, egui application/input/diagnostics, scene selection, offscreen color target |

Prefer a module for a substantial data structure and its behavior. Keep small
format records together when they share one serialization boundary; do not
create one file per trivial record. Preserve public re-exports from the crate
root so internal cleanup does not force downstream churn.

## Package loader invariants

- Use a small bounds-checked archive over `Read + Seek` or an in-memory byte
  buffer. Individual local packages are small enough that memory mapping is
  unnecessary initially.
- Validate magic, versions, table counts, table offsets, export offsets,
  serialized sizes, integer conversions, and allocation sizes before use.
- Malformed or unsupported input must produce an error with package path and
  byte offset, not panic or allocate an attacker-controlled amount of memory.
- Implement the signed UE1 compact-index encoding once and test all byte-count,
  sign, and boundary cases.
- Model package references explicitly:
  - zero means no object;
  - positive references select exports;
  - negative references select imports.
- Use stable IDs such as `(package_id, object_index)` for the cyclic UObject
  graph. Do not build a web of long-lived Rust references.
- Resolve outer chains and qualified object names centrally.
- Load exports lazily where useful and retain enough raw location information
  for diagnostics.
- Report unsupported object classes explicitly. Do not silently guess their
  serialized layout.
- Preserve unknown tagged properties/exports sufficiently for inspection.
- The project currently needs a read-only loader. Do not build a package writer
  or round-trip serializer until explicitly required.

Actor properties are serialized relative to class metadata and defaults.
Consequently, map loading will eventually require class information from `.u`
dependencies; do not create a map design that assumes every actor can be
decoded in isolation.

## Renderer conventions

- Use `wgpu` for Metal, Direct3D 12, and Vulkan support.
- Use WGSL shaders.
- Use `glam` for game and graphics math.
- Define the Unreal-to-render coordinate conversion in exactly one module.
  Include handedness, axes, polygon winding, Unreal rotator units, and the
  wgpu clip/depth convention. Do not scatter sign flips through loaders and
  shaders.
- Keep decoded CPU assets separate from GPU resources.
- The renderer should be usable with a caller-provided device, queue, command
  encoder, target view, target format, and viewport size. It must not own the
  application event loop or require eframe.
- Base palettes/textures, UE texture coordinates, invisible/fake-backdrop
  filtering, masked cutouts, one-/two-sided rendering, material diagnostics,
  translucent/modulated passes, unlit surfaces, and fixed-view sky zones
  composited through fake-backdrop polygons are implemented. Static UE1
  lightmaps are reconstructed from zone ambient colors, light actors, and BSP
  shadow masks, then packed into a shared GPU atlas.
- UE1 rendering will later require masked, translucent, and modulated surfaces,
  zones and portals, sky zones, movers, sprites, coronas, vertex/skeletal
  meshes, animated textures, and HP-specific particles. Do not implement these
  before the current milestone needs them.
- BSP collision and original movement semantics matter for compatibility. Do
  not substitute a generic physics engine before those semantics are understood.

## Viewer and GUI choice

Use `eframe` with its wgpu backend for `openhp1-viewer`. The viewer is a
UI-heavy desktop tool, so eframe should own its window, event loop, input
integration, and egui painting.

Render the 3D viewport into an offscreen wgpu color texture with its own depth
buffer, then display the color texture in the central egui panel. This makes
sidebars, inspectors, picking coordinates, and viewport resizing straightforward.

Obtain the shared wgpu device, queue, and render state from eframe's creation
context. Do not make `openhp1-render` depend on eframe types.

The eventual game executable should instead own `winit` and `wgpu` directly.
Integrate `egui-winit` and `egui-wgpu` there only for debug overlays. Do not
make eframe own the eventual game loop, raw input, cursor grabbing, fullscreen
behavior, or frame scheduling.

As of 2026-07-25, `eframe`/`egui-wgpu` 0.35 use wgpu 29 while wgpu 30 is
available. Keep one compatible wgpu version across the workspace rather than
compiling two incompatible wgpu type universes. Re-check this relationship
before dependency upgrades.

## Chosen supporting libraries

- Stable Rust; use nightly only for a demonstrated requirement.
- `thiserror` for structured library errors.
- `anyhow` for executable-level context and top-level error reporting.
- `tracing` and `tracing-subscriber` for structured diagnostics.
- `bytemuck` for checked plain-data conversion of GPU buffer types.
- `kira` later for PCM playback, mixing, spatial sound, and streaming.

Kira is an audio engine, not necessarily the decoder for the payload embedded
in every `.umx`. First extract and identify the actual `Music` export payloads.
Add a tracker/module decoder such as libxmp or libopenmpt only if the corpus
requires it, then feed decoded PCM into the audio layer.

Prefer the standard library for the first package reader. `binrw` is acceptable
if it demonstrably shortens fixed-record parsing, but UE version branches,
compact indices, export-relative seeks, and class-dependent serialization will
still need explicit code.

Do not add Tokio, an ECS, a physics engine, a job system, a general asset
database, or a dependency-injection framework without a current use case.

## First milestone order

1. **Package inspector**
   - Parse header, name table, import table, and export table.
   - Print versions, qualified object names, classes, offsets, and sizes.
   - Scan every local Unreal package without panicking.
2. **Dependency resolver**
   - Follow configured package paths.
   - Resolve package names case-insensitively.
   - Resolve import/export references and outer chains.
3. **Texture inspector**
   - Decode `Palette` and `Texture`.
   - Decode P8 mipmaps and display or export the first mip.
4. **Untextured BSP viewer**
   - Decode `Level` and `Model`.
   - Reconstruct and triangulate BSP polygons.
   - Add a free-fly camera.
5. **Textured map viewer**
   - Resolve referenced textures.
   - Implement correct coordinates, winding, palettes, filtering, masking,
     translucency, and modulation.
   - Add lightmaps after base textures are correct.

The map-viewer milestone is complete when it:

- scans every local package without a parser panic;
- loads one real map and its dependencies;
- renders textured BSP with correct winding and UVs;
- provides a usable free camera and egui object/diagnostic inspection;
- identifies unsupported classes with actionable errors;
- never changes files in the original installation.

## Current implementation status

The package inspector has scanned all 248 magic-detected packages in the local
installation. The package, tagged-property, configured package resolver, P8
palette/texture, static `WetTexture`/`FireTexture` preview, `Level`, and inline
`Model` decoders are implemented. The eframe viewer renders base-textured BSP
with depth and a free camera.
`Lev5_Chess.unr` is visually verified with all 961 surfaces resolving to 15
unique textures. Invisible surfaces are omitted, fake-backdrop surfaces
screen-sample a separate BSP pass from the decoded `SkyZoneInfo` viewpoint,
masked palette index zero is alpha-tested, unlit surfaces bypass diagnostic
shading, and surface/texture two-sided flags control culling. Translucent and
modulated surfaces use their UE1 blend equations,
depth-test without writing depth, and are sorted per BSP surface. `WetWater` in
`Lev2_HogFront.unr` is visually verified without the fallback checkerboard.
Serialized palette colors are RGBA; do not swap the red and blue bytes.
Static lightmaps reconstruct zone ambient and per-light shadow-mask
contributions and use UE1's 2x modulation. Zone zero is valid: like UE1, it
inherits ambient settings from the active `LevelInfo` actor. The reconstruction
successfully loads all 41 local maps. Base textures and lightmaps modulate
directly in display space to match UE1's fixed-function renderer; do not insert
an sRGB-to-linear conversion into that path. Procedural textures and light
effects are static snapshots until runtime ticking exists.
Do not replace the exact `Level` world-model reference with a largest-export
heuristic.

## Testing and implementation style

- Keep parsing deterministic, read-only, and independently testable without a
  GPU or window.
- Leave one focused runnable check for each non-trivial parser or conversion.
- Use synthetic unit fixtures for package headers, compact indices, object
  references, and malformed offsets.
- Add a local corpus scan that exercises all packages under `res/`; keep its
  expected output and paths out of the repository.
- Use UE Viewer and SurrealEngine for differential inspection where useful,
  but keep OpenHP1's runtime independent of them.
- Prefer explicit data structures and boring control flow over macros or
  generic parser frameworks.
- Reuse an existing helper before adding a new one. Add dependencies only when
  they remove more complexity than they introduce.
- Fix parsing defects at the shared archive/object seam rather than adding
  guards to individual map or texture callers.
- Preserve unsupported data and diagnostics instead of turning incomplete
  support into silent corruption.
- Run focused formatting, compilation, and tests for touched crates. State
  exactly what ran and whether tests depended on the local copyrighted corpus.
