# OpenHP1 Agent Guide

## Purpose and upkeep

OpenHP1 is an open-source Rust reimplementation of the original Harry Potter
and the Philosopher's/Sorcerer's Stone PC game. It must load and run files from
a legally obtained original installation rather than replacing them with
committed copies.

- Treat the workspace manifests and current code as authoritative for the
  crate list and implemented features.
- Keep this file updated in the same logical change set whenever its
  architecture, workflow, or durable constraints change.
- Keep focused documentation under `docs/` up to date with the implementation.
  Record important discoveries about formats, runtime behavior, coordinates,
  compatibility, or validation there instead of leaving them only in chat,
  commit messages, or temporary notes.
- Keep progress reports, completed milestones, corpus counts, and dependency
  version snapshots out of this file.

## Repository and copyright rules

- `res/` contains the local original game installation and is gitignored.
- Never commit original files, extracted assets, embedded script source,
  generated asset dumps, or binary fixtures copied from the game.
- Public tests must construct small synthetic packages or data.
- Local integration tests may read `res/`, but must never modify it or be
  required by public CI.
- Do not use leaked proprietary source code. Reverse engineering must be based
  on behavior, documented formats, legally obtained binaries/packages, and
  properly licensed reference projects.
- Keep a clear non-affiliation/trademark notice for public releases.
- Use the local SurrealEngine clone at
  `/Users/splitty/Developer/SurrealEngine` rather than downloading temporary
  copies. UE Viewer is also an acceptable licensed reference.

## Architecture

- Reuse existing crates, modules, helpers, and data flow before adding another
  abstraction. Add a crate only for a real independent responsibility or
  dependency boundary.
- Keep the dependency graph acyclic. Package decoding is the foundation; typed
  map, mesh, texture, and script decoding sits above it; scene/runtime assembly
  sits above those; renderers and executables consume decoded data.
- The renderer must not know package paths, byte offsets, imports, exports, or
  filesystem resolution. Keep decoded CPU assets separate from GPU resources.
- Keep Unreal-to-render coordinate conversion in one module. Do not scatter
  axis swaps, handedness changes, winding fixes, or rotator sign changes across
  loaders and shaders.
- Keep crate roots as small module indexes or executable entry points. Preserve
  public re-exports when reorganizing internals.
- Package-specific mesh decoding and pose sampling belong in `openhp1-mesh`.
  Keep playback orchestration with its current consumer until another consumer
  creates a genuinely reusable animation responsibility.
- Package-specific audio decoding and platform playback belong in
  `openhp1-audio`; keep game/runtime audio policy out of the renderer.
- `openhp1-viewer` uses eframe for its UI and displays the renderer's offscreen
  target. `openhp1-render` must remain independent of eframe.
- `openhp1-game` owns its winit/wgpu event loop. Do not make eframe own the game
  loop, raw input, cursor grabbing, fullscreen behavior, or frame scheduling.
- Use `kira` for PCM playback, mixing, spatial sound, and streaming. Keep
  package decoding and platform playback in `openhp1-audio`, and keep the
  original game's playback policy in the runtime and game.
- Preserve UE1 BSP collision and movement semantics rather than substituting a
  generic physics engine.
- Prefer the standard library and existing dependencies. Do not add an ECS,
  Tokio, a job system, a general asset database, dependency injection, or other
  infrastructure without a current demonstrated need.

## Package and object invariants

Detailed package and map notes live in
[`docs/unreal-package-format.md`](docs/unreal-package-format.md) and
[`docs/map-format.md`](docs/map-format.md).

- `.unr`, `.utx`, `.uax`, `.umx`, and `.u` share the Unreal package container
  with little-endian magic `0x9E2A83C1`. Support the local package-version range
  from 61 through 76.
- Discover packages from configured `[Core.System] Paths`, confirm package
  magic, and resolve package names and Windows-era filenames
  case-insensitively. Do not trust only the final extension; localized files
  use suffixes such as `.int_uax`, `.spa_uax`, and `.hun_utx`.
- Use a small bounds-checked archive over `Read + Seek` or an in-memory buffer.
  Validate table counts and offsets, export ranges, integer conversions, and
  allocation sizes before use.
- Malformed or unsupported input must return an error with package path and byte
  offset, not panic or allocate an attacker-controlled amount of memory.
- Implement signed compact indices once and test all byte counts, signs, and
  boundary cases.
- Model object references explicitly: zero is none, positive values select
  exports, and negative values select imports.
- Use stable `(package_id, object_index)` identities for cyclic UObject graphs.
  Resolve outer chains and qualified names centrally.
- Preserve unknown properties and unsupported exports for inspection. Never
  guess a serialized layout silently.
- Actor properties depend on class metadata and defaults; do not decode map
  actors as isolated tagged-property bags.
- Follow the `Level` export's exact world-model reference. Do not replace it
  with a largest-export heuristic.
- Keep the loader read-only. Do not build a package writer until explicitly
  required.

## Rendering, mesh, and runtime invariants

Keep the focused notes in
[`docs/renderer.md`](docs/renderer.md),
[`docs/texture-format.md`](docs/texture-format.md),
[`docs/mesh-format.md`](docs/mesh-format.md), and
[`docs/runtime.md`](docs/runtime.md) aligned with behavior.

- Serialized palette colors are RGBA; do not swap red and blue.
- Base textures and UE1 lightmaps modulate directly in display space using
  UE1's 2x modulation. Do not insert an sRGB-to-linear conversion.
- Zone zero is valid and inherits ambient settings from the active `LevelInfo`.
- Actor transforms use positive yaw, negative pitch, and negative roll in
  yaw/pitch/roll composition order; this is not the inverse view rotation.
- UE1 skeletal meshes use a mirrored ActorX local Y axis. Mirror Y, reverse
  triangle winding, negate `RotationOrigin.Yaw`, and conjugate ActorX bone
  orientations before hierarchy composition.
- State frames retain their instruction pointer and locals across latent
  actions. Runtime actions must update persistent actor state and the
  corresponding scene state without later animation ticks undoing them.

## Testing and implementation style

- Keep parsing deterministic, read-only, and independently testable without a
  GPU or window.
- Leave one focused runnable check for each non-trivial parser or conversion.
- Use synthetic unit fixtures for package headers, compact indices, object
  references, and malformed offsets.
- Local corpus scans may exercise `res/`, but keep paths and expected output out
  of the repository.
- Use UE Viewer and SurrealEngine for differential inspection where useful,
  while keeping OpenHP1 independent of them.
- For original-game behavior, inspect the shipped packages first: embedded
  UnrealScript, compiled bytecode and class defaults, configuration, and native
  binaries. Use reference engines only after that evidence is exhausted.
- Treat embedded source comments and property dumps as leads, not active
  behavior; verify them against compiled defaults or bytecode before changing
  compatibility behavior. Do not tune guessed heuristics before tracing the
  authored and native path end to end.
- Prefer Semble's `search` tool for code searches. Query by symbol or behavior,
  follow the returned file and line, and fall back to `rg` or another
  alternative only when Semble does not find the intended code.
- Prefer explicit data structures and boring control flow over macros or
  generic parser frameworks.
- Fix defects at the shared archive/object/runtime seam rather than adding
  guards to individual callers.
- Preserve unsupported data and diagnostics instead of hiding incomplete
  support.
- Prefix Cargo commands with `env RUSTC_WRAPPER=` to avoid sccache privilege
  issues.
- Run targets with `--release` unless the task specifically requires a debug
  build.
- Run focused formatting, compilation, and tests for touched crates. State
  exactly what ran and whether tests used the local copyrighted corpus.
- Run `cargo fmt --all` before each commit; format rather than using `--check`.
- Commit each logical change set separately instead of packing unrelated work
  into one large commit.
