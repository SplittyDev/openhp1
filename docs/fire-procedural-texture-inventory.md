# Shipped Fire procedural-texture inventory

Status: native binary/decompilation inventory with OpenHP1 implementation
tracking. This document reconciles the complete shipped `Fire.dll` body/export
set and records the implementation contracts for Fire, Ice, Water, Wave, Wet,
and their shared fractal scheduler. It does not treat editor painting tools or
compiler-generated bodies as game rendering features.

Primary evidence is the legally obtained shipped `Fire.dll`, SHA-256
`029ff9562c68ee502ab0a02963341e2b00ab42c4e2d1a33c268f306dc3c13041`,
and [`res/Ghidra_Fire.c`](../res/Ghidra_Fire.c). Direct `objdump` disassembly is
used where Ghidra's x87/register aliases lose a formula.

## Reproducible coverage reconciliation

The inventory used `objdump -p res/System/Fire.dll`, `objdump -d`, and a
structural count of every standalone top-level opening brace. Three
control-block false positives were removed.

| Class | Count | Reconciliation |
|---|---:|---|
| Export ordinals | 133 | 114 code and 19 data |
| Unique exported code bodies | 107 | Six EInternal allocators alias RVA `0xc720`; FDrop, FSpark, and KeyPoint assignment alias RVA `0xe330` |
| Unnamed `FUN_*` bodies | 72 | 65 product support/optimized bodies and seven CRT bodies at `0x1050efb8..0x1050f0f0` |
| Catch bodies | 92 | No independent behavior |
| Unwind bodies | 55 | No independent behavior |
| Standalone CRT/compiler bodies | 7 | No product behavior |
| **Top-level bodies** | **333** | `107 + 72 + 92 + 55 + 7` |

Total non-product CRT/compiler bodies are 14: seven among the unnamed bodies
and seven standalone. The product closure is therefore 107 unique exported
bodies plus 65 unnamed optimized/support bodies. No output-bearing algorithm
remains unexplained.

## Complete export-family mapping

| Ordinals | Exported ownership / behavior | Classification |
|---|---|---|
| 1-39 | Copy/default constructors, destructors, allocators, and assignment for Fire, Fractal, Ice, Water, Wave, Wet, and record types | Object/setup/teardown |
| 40-45 | Six vtables | Data |
| 46-101 Water | `AddDrop 0x10506f10`, `CalculateWater 0x105059f0`, `DeleteDrops 0x10507220`, clear/click/init/postload/destroy/touch/paint/mouse | Shared Water output/state plus editor input |
| 46-101 Fire | `AddSpark 0x10501130`, `CloseSpark 0x10501a00`, `DeleteSparks 0x10501a60`, `DrawSparkLine 0x10501ae0`, `FirePaint 0x10501c60`, movement `0x10501e10..0x10502316`, `DrawFlashRamp 0x10502320`, clear/click/init/postload/touch/tick/mouse/serialize | Fire output/state plus editor input |
| 46-101 Wet | `ApplyWetTexture 0x10505a40`, clear/init/postload/tick | Wet output/state |
| 46-101 Ice | Movement `0x10505b40`, blits `0x10505e90/0x10506210`, clear/click/init/postload/tick | Ice output/state plus editor input |
| 46-101 Wave | Clear/init/postload/tick | Wave output/state |
| 46-101 Fractal | Init/postload/prime/post-edit/touch | Shared scheduling/state plus editor input |
| 102-107 | Six `PrivateStaticClass` objects | Data |
| 108-112 | `RedrawSparks 0x105025e0`, `RenderIce 0x1050a600`, Fire serialization, `SetRefractionTable 0x10509d10`, `SetWaveLight 0x10509510` | Output/state |
| 113-118 | Six `StaticClass` methods | UObject scaffolding |
| 119-125 | `TempDrawSpark`, `UIceTexture::Tick`, three `TouchTexture` methods, `WaterPaint`, `WaterRedrawDrops` | Output/state or editor input |
| 126 | `DllMain` | Module glue |
| 127-133 | `GPackage` and six `autoclass*` globals | Data |

Exact behavior/setup ordinal map; constructors/operators/data are already
closed by the ranges above:

```text
 46 AddDrop(Water)             75 Init(Ice)                 108 RedrawSparks
 47 AddSpark(Fire)             76 Init(Water)               109 RenderIce
 48 ApplyWetTexture            77 Init(Wave)                110 Serialize(Fire)
 49 BlitIceTex                 78 Init(Wet)                 111 SetRefractionTable
 50 BlitTexIce                 79 InternalConstructor(Fire) 112 SetWaveLight
 51 CalculateWater             80 InternalConstructor(Ice)  113 StaticClass(Fire)
 52 Clear(Fire)                81 InternalConstructor(Wave) 114 StaticClass(Fractal)
 53 Clear(Ice)                 82 InternalConstructor(Wet)  115 StaticClass(Ice)
 54 Clear(Water)               83 Lock(Ice)                 116 StaticClass(Water)
 55 Clear(Wave)                84 Lock(Wet)                 117 StaticClass(Wave)
 56 Clear(Wet)                 85 MousePosition(Fire)       118 StaticClass(Wet)
 57 Click(Fire)                86 MousePosition(Ice)        119 TempDrawSpark
 58 Click(Ice)                 87 MousePosition(Water)      120 Tick(Ice)
 59 Click(Water)               88 MoveIcePosition           121 TouchTexture(Fire)
 60 CloseSpark                 89 MoveSpark                 122 TouchTexture(Fractal)
 61 ConstantTimeTick(Fire)     90 MoveSparkAngle            123 TouchTexture(Water)
 62 ConstantTimeTick(Ice)      91 MoveSparkTwo              124 WaterPaint
 63 ConstantTimeTick(Wave)     92 MoveSparkXY               125 WaterRedrawDrops
 64 ConstantTimeTick(Wet)      93 PostDrawSparks
 65 DeleteDrops                94 PostEditChange(Fractal)
 66 DeleteSparks               95 PostLoad(Fire)
 67 Destroy(Ice)               96 PostLoad(Fractal)
 68 Destroy(Water)             97 PostLoad(Ice)
 69 Destroy(Wet)               98 PostLoad(Water)
 70 DrawFlashRamp              99 PostLoad(Wave)
 71 DrawSparkLine             100 PostLoad(Wet)
 72 FirePaint                 101 Prime(Fractal)
 73 Init(Fire)
 74 Init(Fractal)
```

The 65 unnamed product bodies are optimized Fire/Water filters and
dispatchers, Ice blit loops, deleting destructors, and container/palette/archive
support reachable from the families above. Catch/unwind bodies remain owned by
their exported/product callers and add no independent output.

## Direct evidence versus inference

Direct proof includes export VAs, switch cases, field reads/writes, arithmetic,
pixel addressing, tick calls, save/restore ordering, palette replacement, and
caller closure. Names for unknown bytes such as the client field at `+0x54`,
and semantic labels attached only by the decompiler, remain inferred. The
checklists preserve the direct state transition without inventing a name.

## Shared fractal scheduling and priming

`UFractalTexture::Prime` directly raises `PrimeCount` to at least 48. An
already-primed object returns. A null client or zero `Client+0x54` takes the
base/delegated Prime path; only a nonzero field is temporarily written to zero
for the virtual constant-time tick loop and then restored as literal `1`
([F:4864-5088](../res/Ghidra_Fire.c#L4864)). This is the source-backed behavior;
no semantic field name or arbitrary-old-value restoration is inferred.

- [x] Fire construction/first identity registration performs the exact
      minimum-48 pre-visible Prime loop; the old 32-step static snapshot was
      removed ([fire.rs](../crates/openhp1-texture/src/fire.rs),
      [loader.rs](../crates/openhp1-scene/src/loader.rs)). Synthetic counts
      `0, 1, 47, 48, 49` cover the Fire loop.
- [ ] Shared UWater, Wave, and Wet still need the same scheduler contract. Their
      open work must preserve the direct null/zero/nonzero client branches
      above rather than inventing a named field abstraction.

## FireTexture behavior closure

The FireTexture runtime is implemented for the proved shipped-live cases, but
the Fire acceptance boundary and `BASE-009B` remain open: exhaustive independent
acceptance for all 44 redraw cases is not complete, and `BASE-009B` also owns
the shared UWater and Wet behavior below. Output-bearing Fire evidence is
`AddSpark` through movement/flash helpers
([F:1113-3577](../res/Ghidra_Fire.c#L1113)), `UFireTexture::ConstantTimeTick`
at VA `0x105082c0` ([F:5543](../res/Ghidra_Fire.c#L5543)), persisted spark
filtering/serialization ([F:5715](../res/Ghidra_Fire.c#L5715)), and the optimized
filter wrappers beginning at [F:10839](../res/Ghidra_Fire.c#L10839) with
implementation bodies at [F:11843](../res/Ghidra_Fire.c#L11843).

- [x] Decode persistent Fire fields, exact eight-byte spark records, clamped
      active prefix/backing storage, the process-global 512-byte RNG transition,
      all `0x00..0x2b` redraw implementations, one exact wrapped scalar filter,
      `PostDrawSparks`, minimum-48 priming, and the 1,024 initialized render
      entries. The object reserves 1,028 bytes at `+0x104..+0x507`, but native
      PostLoad initializes only the first `0x400` bytes; four trailing bytes are
      not additional sum-table entries ([fire.rs](../crates/openhp1-texture/src/fire.rs),
      [texture.rs](../crates/openhp1-texture/src/texture.rs)).
- [x] Keep one animation per `SceneObjectId`, first-registration order for the
      shared RNG stream, shared masked/unmasked subscribers, pre-visible CPU
      dependency pixels, and changed GPU uploads
      ([loader.rs](../crates/openhp1-scene/src/loader.rs)).
- [x] Independent full-state direct-redraw fixtures cover shipped cases `0x00`,
      `0x01`, `0x03`, `0x1b` and its same-tick `0x2b`, plus authored-corpus
      `0x0c`. They compare all backing slots, all 64 pixels, the full RNG table
      and index, and helper-global state. Additional focused fixtures cover the
      corrected spawn/internal cases, append/swap-delete order, line endpoint,
      Manhattan and star boundaries, filters, render table, Prime, and scheduler.
- [ ] Add independently derived full-state acceptance for every one of the 44
      cases. The implementation switch is complete, but this exhaustive proof
      is intentionally not checked off.
- [ ] Recover the exact initial Core process-global `appRand` history and the
      native visible-use update cadence. The production MSVCRT/time seed is a
      closer startup approximation; injected 512-byte state is the exact test
      boundary.
- [ ] Live compare retail, Classic, and Modern. Proved BSP uses are `0x00` at
      `Lev_Tut1` `owlstand1` (2 surfaces), `0x01/0x03` at `Lev5_Final`
      `Furnace` (32) and `Lev5_fluffy` `ancflame1` (4), and `0x1b -> 0x2b` at
      `Lev3_DungeonB` `lumos1` (80). `0x0c` appears in stale `Win_L`/`Jelly1`
      imports with no owning class/function/state/default-property consumer, so
      it is extra authored-corpus coverage, not a shipped-live gate.

Editor painting/click/mouse exports are not stock-game rendering contracts.

## IceTexture closure

The existing `BASE-009A` behavior inventory is complete:

- movement: [F:3658](../res/Ghidra_Fire.c#L3658);
- blits: [F:3752](../res/Ghidra_Fire.c#L3752) and
  [F:4147](../res/Ghidra_Fire.c#L4147);
- lifecycle/ticks: [F:7146](../res/Ghidra_Fire.c#L7146);
- lock: VA `0x1050e560`.

No additional Ice output feature was found. Existing focused and live
acceptance remains authoritative.

## Shared UWater and Wave evidence

Direct evidence:

- `CalculateWater`: VA `0x105059f0`, [F:3583](../res/Ghidra_Fire.c#L3583).
- Drop dispatcher/cases: VA `0x10506440`,
  [F:4251](../res/Ghidra_Fire.c#L4251).
- Add/delete: VAs `0x10506f10/0x10507220`,
  [F:4588](../res/Ghidra_Fire.c#L4588).
- Water parity dispatcher: [F:10895-10915](../res/Ghidra_Fire.c#L10895).
- Corrupted optimized kernel decompilation: [F:12658-13326](../res/Ghidra_Fire.c#L12658).

The prior citation ending at line 13619 was invalid because `Ghidra_Fire.c`
ends at line 13326. Both optimized parity kernels are self-modifying/pipelined;
the decompiled C has corrupted register aliases and impossible stores. Exact
implementation remains blocked on either a mechanical instruction-order scalar
port from x86 disassembly or native-harness synthetic goldens. Algebraically
similar output is not sufficient to claim byte parity.

- [ ] Preserve the single in-place `width*height/2` SourceFields byte buffer,
      alternating parity-0/parity-1 kernels, 1,536-byte table, 256 eight-byte
      drops, process-global Fire RNG, exact 22 redraw cases
      (`0x00..0x13`, internal `0x40/0x41`), minimum-48 priming, and
      once-per-nonzero-update behavior when `MaxFrameRate==0`.
- [ ] For `BASE-009B`, recover both optimized parity kernels with a mechanical
      instruction-order scalar port or native-harness synthetic goldens.
- [ ] Implement Wave's exact 1,024-byte lighting table and generated palette.
- [ ] Acceptance: full tables/palette, wrapped 8x8 or 16x8 parity kernels,
      injected RNG/drop cases, clear bits, and exact 1/2/48-step checksums.

The only Wave export is `Detail.WaterDE2`. Twelve otherwise-unused
`Liquids` WetTextures reference it as `DetailTexture`, but the complete package
scan found no map/class importing those Wet owners. Wave/Wet have no honest
shipped live gate; use synthetic retail differentials without committed assets.

## Missing WetTexture feature row

Wet is not merely the shared Water output. It has a distinct source-image
refraction pass and must be owned explicitly by `BASE-009B`; `BASE-009C` owns
Wave-only behavior layered over the same core.

Direct evidence is `ApplyWetTexture` VA `0x10505a40`
([F:3601](../res/Ghidra_Fire.c#L3601)), Wet lifecycle/tick
([F:6689](../res/Ghidra_Fire.c#L6689)), and `SetRefractionTable` VA
`0x10509d10` ([F:7073](../res/Ghidra_Fire.c#L7073)). Direct x87 disassembly
recovers:

```text
table[i] = clamp(ftol((i - 511) * WaveAmp / 512), -128, 127)
i = 0..1023

D(x,y) = Source((x + signed(displacement(x,y))) & (width - 1), y)
```

Native Wet also nearest-neighbor expands a smaller source texture, adopts the
source palette when source identity changes, and uses the shared Water
drop/parity/RNG core.

OpenHP1's current Water animation is materially divergent: eight drop kinds,
full-resolution floating-point simulation, a fixed 30 Hz accumulator, an
unrelated LCG, and gradient-derived clamped displacement. Both Classic and
Modern therefore remain divergent for Wet.

- [ ] Build every one of the 1,024 refraction entries at `WaveAmp=0,128,255`.
- [ ] Cover signed offsets `-128,-1,0,1,127` with horizontal power-of-two wrap.
- [ ] Preserve smaller-source nearest-neighbor expansion and palette adoption.
- [ ] Exercise all 20 public drop cases plus internal `0x40/0x41`, direct
      `TouchTexture` writes to both checkerboard parities within SourceFields,
      and an injected shared-RNG sequence.
- [ ] Verify exact 1/2/48-step parity checksums and
      once-per-nonzero-update behavior for `MaxFrameRate==0`.
- [ ] Use synthetic retail/Classic/Modern comparison; do not invent a shipped
      scene for an unreachable owner.

## Ecto reachability boundary

`HPBase.spellEcto` was cut, has no shipped user, and is never rendered in the
game. It is not a live representative for procedural textures, decals, or
environment mapping. Keep only this negative-reachability result. Generic
synthetic fixtures may test an engine feature, but must not be described as an
ecto gameplay path.

## Remaining census follow-up

- [ ] Record the total number of all `FireTexture` exports in the reproducible
      package-class census. This does not reopen the native behavior/body
      closure or the already proved live Fire imports.
