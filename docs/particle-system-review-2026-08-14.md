# Particle system review, 2026-08-14

This review compares OpenHP1's `ParticleFX` path with the legally obtained
original game. It records confirmed native semantics separately from dormant
engine surface area and visual questions. It does not reproduce proprietary
source or asset data.

## Evidence boundary

Primary sources, in order:

- `res/System/Engine.u`, package version 76, SHA-256
  `b3661a1d2afb1f730ba5bb3cbd2a6716efaf55fd0d8cefa753b69026a8bc5a85`;
- `res/System/HPParticle.u`, SHA-256
  `ea7e5f22c23c3983338f7e974fe5922a4733eaaddb930b02e4d913f10b32ad17`,
  plus other shipped System packages and maps where noted;
- `res/System/Engine.dll`, SHA-256
  `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`.

`Engine.u` identifies `ParticleFX` as class export 494 and its `ScriptText` as
export 3743. Package tables were inspected with
`target/debug/examples/package_inspect`; embedded declarations were located
with `strings -a -n 4`; compiled defaults were decoded through the existing
runtime package/default-property path; and the x86 native implementations were
checked with `objdump -d`. The local SurrealEngine clone was searched only
after the shipped evidence. It has no HP1 `AParticleFX`/`UParticle`
implementation: its U227 emitter methods are unrelated and largely explicitly
unimplemented, so it supplied no particle formulas.

OpenHP1's reviewed path is `ScriptRuntime::particle_emitters` in
`crates/openhp1-runtime/src/world/actor.rs`, `ParticleEmitter` in
`crates/openhp1-runtime/src/world/action.rs`, and
`LoadedScene::{sync_particle_emitters,tick_particles}` plus their helpers in
`crates/openhp1-scene/src/loader.rs`.

## Original property surface

The shipped class declares these effective groups:

- emission: `ParticlesPerSec`, source width/height/depth, `Period`, `Decay`,
  angular spreads, `bSteadyState`, `bPrime`, `Distribution`, `Pattern`,
  `ParticlesAlive`, `ParticlesMax`, and `bEmit`;
- movement: `Speed`, `Lifetime`, `bUpdate`, `bVelocityRelative`,
  `bSystemRelative`, chaos and delay, elasticity, directional attraction,
  damping, wind response and `bWindPerParticle`, zone-gravity response, and an
  additional authored gravity vector;
- appearance: start/end color and alpha, width/length/end scale, spin and drip
  time, `ParentBlend`, alpha/color/size delay, alpha/size grow periods, five
  random texture slots, `ColorPalette`, and the render primitive.

The enum values are `Line=0`, `Billboard=1`, `Liquid=2`, `Shard=3`, and
`TriTube=4`; distribution is `Random=0`, `Uniform=1`, and `OwnerMesh=2`.
OpenHP1 already projects and implements the commonly authored emission,
movement, inherited-parent blending, pattern, owner-mesh, billboard, and
liquid paths. Existing native-evidence notes cover unlimited particle counts,
zero lifetime, gravity, chaos, damping, collision, wind force, and owner
velocity; those were not contradicted by this pass.

## Confirmed fixes from this review

### Native alpha evolution

The base compiled defaults set `AlphaStart=(1,0)` and leave `AlphaEnd` at
zero. Most finite HPParticle subclasses inherit that fade. Representative
compiled defaults include `Fire01` (lifetime 0.9 plus random 0.5),
`Levitate_wand` (0.25), `SmokeExplo_01` (start alpha 0.35, lifetime 1 plus
random 1), and `WizCard_Explo`; `FireEP_purple` additionally authors
`AlphaDelay=1.2` with lifetime 1.5. A defaults inventory found 153 of 156
renderable HPParticle subclasses ending at alpha zero.

`UParticle::Update` at virtual address `0x103bfc30`, alpha block
`0x103c0360..0x103c03d3`, evolves the sampled per-particle alpha when
`bUpdate` is true:

1. Before `lifetime * AlphaGrowPeriod`, add
   `delta * AlphaStart / (lifetime * AlphaGrowPeriod)`, capped at
   `AlphaStart`.
2. Otherwise, after `AlphaDelay`, add
   `delta * (AlphaEnd - AlphaStart) / (lifetime - AlphaDelay)`.
3. Clamp a result below the native near-zero threshold to zero. The native
   setup makes the fade slope zero when lifetime does not exceed the delay.

OpenHP1 previously projected `AlphaStart` and `AlphaEnd` but discarded both in
the scene, leaving particle color opaque. Commit `04aa1da` now samples and
updates alpha through the renderer's existing per-vertex color modulation,
including `AlphaDelay` and `AlphaGrowPeriod`. Commit `404f7cb` applies the
native `0.001` cutoff established by the constant at `0x10473814`.

### `bWindPerParticle`, not `bSystemRelative`, selects wind sampling

`AParticleFX::UpdateParticles` at `0x103c1f02..0x103c1f4b` samples wind once
at the emitter and passes that vector to particle updates. `UParticle::Update`
at `0x103bfd2a` tests the emitter bit for `bWindPerParticle`; only true
particles resample wind at their own world positions. Positional
`bSystemRelative` is independent.

This is authored game behavior, not unused metadata. `res/System/Hub5.u`
`FluteSleepFX` is class export 316 (class payload at file offset `0x4d43`) and
has a compiled true `bWindPerParticle`; it inherits from
`FluteNoteFX -> ParticleFX`, with effective damping and wind response. Commit
`b067365` projects the flag and uses it at the shared particle integrator.

### Patterned uniform residue samples `Period`

The `DIST_Uniform` pattern branch at `0x103c22cd..0x103c23a8` samples
`Period.Base + Period.Rand * FRand`, chooses that sample's segment, and uses
that segment length times `(point_count - 1) * DrawScale * Period.Rand` before
the one division by sampled `ParticlesPerSec`.

OpenHP1's residue helper previously chose a fixed period midpoint, while
particle placement separately drew a random period sample. The rate division
was already once and agreed with native. The discrepancy changes density only
when neighboring pattern segment lengths differ. `TemplateSparkle01`,
`GoodDrawPoint`, `DrawBadPoint`, `GoldSparkle01`, and `BronzeSparkle01` prove
shipped patterned-uniform use. Commit `4f8fb34` now samples the period for
segment selection and covers unequal segment lengths with a focused test.

### Native emission placement

In `AParticleFX::EmitParticles` (implementation starts at `0x103c2170`),
`0x103c2ddd..0x103c2ee8` samples each source coordinate uniformly over half
the sampled source dimension in each direction, then transforms that local
vector through the emitter actor coordinates.

OpenHP1 previously sampled the same centered box but added it directly to the
emission center, leaving a rotated, nonsymmetric source volume world-aligned.

The same native emitter uses an independent random temporal fraction for
`DIST_Random`, and `(index + 1) / emitted_count` for `DIST_Uniform`; it then
rewinds from the current emitter position toward the previous position by the
remaining fraction (`0x103c274c..0x103c278f` and
`0x103c2f35..0x103c2f7c`). OpenHP1 instead uses
`(index + 0.5) / emitted_count` for every distribution, making random emission
evenly stratified and leaving uniform emission half a substep behind the native
endpoint. Commit `530801a` rotates the sampled source box through the existing
Unreal transform and selects the native temporal fraction for random and
uniform distribution. Its focused scene regression covers both behaviors.

## Explicitly unresolved or unproven

- `PPRIM_Liquid` is implemented, but its exact original vertex shape was not
  recovered in this pass. Exact `Line`, `Shard`, and `TriTube` geometry remains
  unresolved in `Render.dll`; no shipped initialized use of those three modes
  was established. Do not invent geometry from enum names.
- Random selection among `Textures[5]` and `ColorPalette` cycling are correctly
  diagnosed as unsupported. A scan of initialized maps and shipped ParticleFX
  subclass defaults found no effective multi-texture or palette case, so they
  are engine coverage gaps rather than required game fixes.
- `Decay`, `bSteadyState`, and scripted native particle accessors
  (`AddParticle`, `GetParticleParams`, `SetParticleParams`, and
  `RecomputeDeltas`) are declared by the base engine. Searches of shipped
  System script text found no authored assignments/calls beyond declarations;
  no patch is warranted from this evidence.
- Authored Wind actors use `WindFluctuation=51` in eleven Quidditch maps, while
  OpenHP1 only consumes the current transient `Fluc` vector and does not evolve
  it. This is a genuine broader Wind gap, but this pass did not prove an active
  particle with nonzero damping and wind response overlaps those fluctuating
  winds. Keep it separate from `ParticleFX` until the native `AWind` evolution
  and effective consumer path are both established.
- Builds and synthetic tests prove formulas, not appearance. The retained
  changes still need particle-heavy differential observation against retail,
  especially fire/smoke alpha, Fluffy flute notes, moving broom/spell trails,
  and patterned lesson effects.

## Conclusion

The confirmed discrepancies are fixed in separate logical commits:

- `04aa1da`: native alpha evolution;
- `b067365`: `bWindPerParticle` sampling;
- `4f8fb34`: patterned-uniform `Period` sampling;
- `530801a`: rotated source boxes and native within-tick placement;
- `404f7cb`: native near-zero alpha cutoff.

No additional particle change is justified by the inspected original-game
evidence. Primitive invention, dormant texture/palette support, and Wind actor
fluctuation remain outside this pass pending effective shipped use and exact
native semantics. Retail visual comparison remains the acceptance boundary for
appearance.
