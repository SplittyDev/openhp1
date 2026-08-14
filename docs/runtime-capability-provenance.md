# Runtime capability provenance

This note records the evidence behind the runtime render-property and
`ParticleFX` capability diagnostics. It is deliberately specific about which
behavior comes from the shipped game and which details are OpenHP1
implementation choices.

## Evidence and confidence

The evidence was checked in the following order:

1. The legally obtained HP1 packages and executable under `res/System`.
2. The local SurrealEngine clone.
3. Online references, only if the first two sources were insufficient.

The shipped files and SurrealEngine were sufficient for every capability
except the HP1-specific `Opacity` render formula. The original scripts prove
the authored values and intended fade, but neither SurrealEngine nor available
UE1 documentation exposes that game-specific field. Its implementation choice
is called out below. In this note:

- **Game script, high confidence** means a declaration, comment, inheritance
  relationship, or serialized class default in a shipped package.
- **Game native, high confidence** means behavior recovered from the shipped
  `Engine.dll` implementation.
- **Implementation choice** means a deliberate OpenHP1 detail that is not
  expected to reproduce the original engine bit for bit.
- **Compatibility assumption** means the original content proves that a
  behavior exists, but none of the available engine references reveals its
  exact native formula.

The inspected `res/System/Engine.dll` has SHA-256
`7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc`. It is a
32-bit x86 PE dated 2001-10-29. The relevant exported methods are
`AParticleFX::UpdateParticles(float)` (implementation RVA `0x0c1d60`) and
`UParticle::Update(FVector const&, float, FVector const&, ULevel*, float,
AParticleFX*)` (implementation RVA `0x0bfc30`).

`res/System/Engine.u` is package version 76. Its `ParticleFX` class is export
494, and its `ParticleFX.ScriptText` `TextBuffer` is export 3743. The script
describes:

- `GravityModifier` as the fraction of zone gravity to apply, where zero means
  no response and one means the full zone gravity.
- `Gravity` as an additional force authored for a particular effect.
- `Chaos` as a velocity perturbation magnitude.
- `ChaosDelay` as the time between velocity perturbations.

The base `ParticleFX` defaults set `bUpdate`, `RenderPrimitive`, and `bEmit`, but
do not override these float properties. Their inherited default is therefore
zero.

## Diagnostic scope

The latest inspected log, `logs/openhp1-game-1785435283873.log`, contains 29
zone-gravity diagnostics and 16 chaos diagnostics. These are repeated
observations of eight effect classes rather than 45 distinct missing
capabilities:

| Effect class | Zone gravity | Chaos |
| --- | ---: | ---: |
| `BadDrawPoint` | 7 | 0 |
| `Levitate_wand` | 7 | 7 |
| `RewardFail` | 7 | 7 |
| `SmokeExplo_03` | 6 | 0 |
| `Spawn_flash_1` | 0 | 1 |
| `WizCardSpin` | 0 | 1 |
| `WizCard_Explo` | 1 | 0 |
| `WizCard_Explo2` | 1 | 0 |

The same log contains 15 render-property diagnostics:

| Property | Count | Effective in this trace |
| --- | ---: | --- |
| `Style` | 3 | `Ghost0` changes; two wand writes do not |
| `AmbientGlow` | 2 | Both wands change |
| `DrawScale` | 2 | Neither wand changes |
| `Mesh` | 2 | Both wands change |
| `Texture` | 2 | Neither wand changes |
| `bUnlit` | 2 | Neither wand changes |
| `bMeshEnviroMap` | 2 | Neither wand changes |

That tutorial trace is not sufficient to decide the engine-wide render scope.
The follow-up corpus pass decoded all 29 shipped System `.u` packages, all 41
maps, 1,247 script `TextBuffer` exports, and 46,041 serialized Level actors
without a decode failure. It compared assignments with inherited class
defaults and map overrides. A separate neutral-input runtime pass ran 27 maps
for 120 seconds; the remaining 14 reached scene/runtime setup but stopped at
the unrelated missing-`HPHud` assertion. Static script and actor coverage is
therefore the authoritative inventory; neutral runtime execution is supporting
evidence, not proof that a delayed or triggered branch is unused.

## Runtime render properties

### Authored writes

**Game script and serialized defaults, high confidence:** the following values
were recovered from `HPBase.u`, `Engine.u`, `HarryPotter.u`, `Tut1.u`, and
`Lev_Tut1.unr`:

| Actor/property | Before | Assigned | Effective |
| --- | --- | --- | --- |
| `baseWand.Mesh` | `WandMesh` | `None` | Yes |
| `baseWand.DrawScale` | `1.0` | `1.0` | No |
| `baseWand.Style` | normal | normal | No |
| `baseWand.Texture` | `Engine.S_Weapon` | same object | No |
| `baseWand.bUnlit` | false | false | No |
| `baseWand.bMeshEnviroMap` | false | false | No |
| `baseWand.AmbientGlow` | 255 | 0 | Yes |
| `Ghost0.Style` | normal | translucent | Yes |

`Inventory.BecomePickup` assigns the unset `PickupViewMesh`, and
`Inventory.BecomeItem` assigns the unset `PlayerViewMesh`; both therefore clear
the wand actor's standalone `Mesh`. The latter also clears `AmbientGlow`.
`Weapon.SetDisplayProperties` writes the four class-default display values, so
those writes are genuine script execution but do not change effective state.

`Harry.PostBeginPlay` and `tut1Peeves.PostBeginPlay` each spawn and equip one
`baseWand`. `Ghost.patrol.startup` assigns the translucent style after its
authored 120-second idle period. The runtime must compare the previous and new
stored values before reporting or rebuilding scene state; a property assignment
alone is not evidence of a missing visual capability.

The corpus-wide result adds the following shipped requirements:

| Property | Original-content evidence | Required projection |
| --- | --- | --- |
| `Mesh` | 25 assignment lines, including Hagrid/Norbert, Quirrell/Voldemort, gargoyle damage stages, potion bottles, spell firecrackers, confetti, shards, and `None` transitions | Null and arbitrary non-null mesh rebinding |
| `DrawScale` | 31 lines with direct, compound, random, and per-tick changes | Mesh transform and sprite dimensions |
| `Style` | Real normal/translucent transitions for Ghost and Peeves/QuidCam paths | Surface blend mode |
| `AmbientGlow` | Inventory and Weapon toggle 255 and 0 | Actor vertex lighting |
| `ScaleGlow` | Broom hoops pulse and fade; 164 authored hoops occur in `Lev_Tut2` | Actor vertex lighting |
| `Skin` | Broom stages select FireTextures; decoration, Pawn, and TV paths also assign it | Actor texture selection |
| `SkelAnim` | Eight configured `TriggerChangeActorMesh` actors assign it, five together with `Mesh` | Skeletal sequence source and bone metadata |
| `Opacity` | Invisible Harry and Peeves animate values between 0.3 and 1.0 | Actor fade multiplier |
| `LightBrightness` | 59 TriggerLights exist across nine maps; a neutral run observed 50 effective writes in eight maps | Dynamic actor lighting and BSP lightmaps |

The neutral runtime pass independently observed:

- 53 wand `Mesh` clears and 53 `AmbientGlow` clears across 40 maps;
- three `BroomHoopStage` scale changes from 1.7 to 1.333 in `Lev_Tut2`;
- the `Snitch_Halo` scale change from 1 to 2 in `Lev5_FlyKeys`; and
- the tutorial Ghost style change exactly at 120 seconds.

`Texture` has one changed direct script path on `ectoMark`, but every spawn of
that class is commented out and no shipped map contains it. Current live
`Texture` writes restore unchanged defaults. `MultiSkins` is limited to generic
network skin selection with no proven shipped level transition. These remain
changed-value diagnostics rather than speculative runtime implementations.

There are zero effective runtime changes to `bUnlit` or
`bMeshEnviroMap`. Six classes and six `Lev2_fire1` movers do author static
`bUnlit` values, which the existing load path already preserves. The only
static environment-mapped actor is `HPBase.spellEcto`; it has
`bMeshEnviroMap=true` and an explicit `Texture=Jgreen`. No class or map actor
authors a ZoneInfo or LevelInfo `EnvironmentMap`.

### UE1 rendering boundary

**SurrealEngine reference, high confidence:** a carried weapon's third-person
geometry is independent of its standalone `Actor.Mesh`.
`../SurrealEngine/SurrealEngine/Render/VisibleMesh.cpp` selects
`Weapon.ThirdPersonMesh` and `ThirdPersonScale` while drawing the owning pawn,
then uses the weapon actor for its material and lighting properties. Clearing
`Actor.Mesh` must therefore hide the standalone actor without removing the
carried wand attachment.

The same reference reads current `Mesh`, `DrawScale`, `Style`, `Skin`, and
`MultiSkins` while drawing; sprite dimensions are texture size multiplied by
`DrawScale`, and mesh transforms include the same scale. It maps translucent
style to the translucent pass, uses `bUnlit` to bypass dynamic lighting, and
adds camera-relative reflection UVs for `bMeshEnviroMap`. Actor `Texture`
precedes the ZoneInfo and LevelInfo environment maps. The shipped
`spellEcto` case exercises only that first branch.

SurrealEngine's actor vertex-light path reads the light actor's current
`LightBrightness`. Its lightmap builder also combines each listed light using
the current brightness. This supports updating both movable actor colors and
world lightmaps for TriggerLight fades; treating the assignment as an actor
material property would be incomplete.

### HP1-specific opacity

**Game script and native renderer, high confidence:** Invisible Harry propagates a live `Opacity`
value to his weapon while fading out and back in. Tutorial Peeves actors use
the same 0.3-to-1.0 range together with translucent style. The property is a
float in the shipped metadata.

The shipped `Render.dll` mesh path checks `Opacity < 1`, adds HP's alpha flag
and the translucent flag, multiplies vertex RGBA by `Opacity`, and replaces
vertex alpha with `Opacity`. The shipped D3D renderer maps that combination to
`SrcAlpha` / `OneMinusSrcAlpha` blending. OpenHP1 follows that path, clamps the
GPU value to 0 through 1, and rebuilds actor materials only when opacity crosses
1 so ordinary opaque, translucent, and modulated modes are restored exactly.
Manual comparison with Invisible Harry and Peeves remains required.

### OpenHP1 implementation boundary

The implementation retains the resolved actor display state that already feeds
scene assembly. Effective topology and material-object changes (`Mesh`,
`DrawType`, `Style`, `Skin`, and `SkelAnim`) re-run that same assembly path,
append replacement geometry, and collapse the old bounded range. This keeps
standalone `Mesh` independent from a carried weapon's `ThirdPersonMesh`,
preserves the current animation sequence and phase, and avoids a second asset
decoder.

Per-tick scalar state does not append geometry. `DrawScale` transforms the
current mesh or sprite range and updates collision-facing visual bounds;
`AmbientGlow` and `ScaleGlow` relight the current range; `Opacity` updates the
actor's blended materials. `LightBrightness` updates retained actor lights,
rebuilds only BSP lightmaps that reference the changed export, and queues only
those atlas rectangles for upload. Static `bUnlit` and environment-map
defaults remain in the ordinary material path.

Keep effective `Texture`, `MultiSkins`, `bUnlit`, and dynamic
`bMeshEnviroMap` changes diagnostic. Suppress same-value assignments by
comparing against the actor instance, inherited class default, or typed zero
default in that order.

## Zone-gravity response

### Native behavior

**Game native, high confidence:** `AParticleFX::UpdateParticles` obtains the
emitter's active zone, falling back to the level information when there is no
zone actor. It resolves acceleration once for the emitter update:

```text
effective_acceleration =
    emitter.Gravity + active_zone.ZoneGravity * emitter.GravityModifier
```

The result is passed to each particle update. Zone gravity is therefore sampled
at the emitter, not separately at every particle location. A negative modifier
reverses the zone-gravity contribution; it must not be clamped.

The native particle update incorporates this acceleration into its
damping/position integration before applying attraction and chaos. OpenHP1's
current particle integrator is not the same analytic damping integrator as the
original. Supporting this capability does not require replacing that
integrator, but the combined acceleration must enter at the same point as the
currently authored `Gravity`, before damping and position advancement.

### OpenHP1 implementation boundary

`ScriptRuntime::particle_emitters` reads both live properties, samples the
emitter's zone through the existing `ScriptRuntime::zone_physics` seam, and
projects their already combined acceleration. That lookup resolves the BSP
zone actor and falls back to `LevelInfo`; the scene remains unaware of package
objects and simply applies the projected acceleration at the existing gravity
step.

### Focused tests

- A synthetic collision model with no mapped zone selects the active
  `LevelInfo` fallback.
- Authored gravity `[1, 2, 3]`, zone gravity `[0, 0, -100]`, and modifier
  `-0.5` produce `[1, 2, 53]`, proving the modifier is not clamped.

## Chaos movement

### Native behavior

**Game native, high confidence:** every particle owns a chaos-delay timer. The
native update implements the following sequence after position integration and
attraction:

```text
timer = max(timer - delta_time, 0)

if Chaos != 0 and timer <= 0:
    direction = vector(
        2 * appFrand() - 1,
        2 * appFrand() - 1,
        2 * appFrand() - 1,
    )
    if length_squared(direction) >= 1e-8:
        direction = normalize(direction)
    velocity += direction * Chaos
    timer = ChaosDelay
```

Important consequences:

- `Chaos` is added as a velocity impulse. It is **not** multiplied by
  `delta_time`, despite the less precise units implied by the script comment.
- Three independent random values form a point in a cube, which is then
  normalized. The original does not use rejection sampling for a uniform
  sphere.
- The kick occurs after the particle's position has advanced, so it affects
  position on the next update.
- `ChaosDelay == 0` produces one kick on every particle update.
- A positive delay suppresses further kicks until the per-particle timer
  expires.

The inspected native constructor does not explicitly initialize the timer
field. Initializing it to zero in OpenHP1, so the first update is eligible for a
kick, is an **implementation choice** consistent with the native condition and
the engine's zeroed particle storage, rather than a separate behavioral formula
recovered from that constructor.

### OpenHP1 implementation boundary

`ParticleEmitter` carries the live `Chaos` and `ChaosDelay` properties into the
scene. Each particle owns a zero-initialized timer, and the native timer and
kick sequence runs after the existing position/attraction work.

Using OpenHP1's deterministic per-emitter random stream is an
**implementation choice**. The original uses the process-global `appFrand`
stream, whose exact sequence depends on unrelated engine activity. Reusing the
local stream preserves reproducible tests and effects while matching the
recovered formula and update order; it is not intended to reproduce the
original random directions bit for bit.

### Focused tests

- The same deterministic random state produces the same direction for short
  and long `delta_time` values, and `Chaos=3` changes velocity by exactly three
  units in both cases.
- `ChaosDelay == 0` kicks on every update.
- With `ChaosDelay == 0.5`, the first update kicks, subsequent updates do not
  kick while the timer is positive, and a kick occurs when it reaches zero.
- Two particles carry independent delay timers.
- The kick does not affect position until the update after it is generated.

## Authored HP1 effects

The following defaults were read from `res/System/HPParticle.u` (package
version 76). “Effective zero” means neither the class nor its parent overrides
the zero-valued base `ParticleFX` property.

| Class (export) | Inheritance/evidence | Relevant effective defaults |
| --- | --- | --- |
| `WizCard_Explo` (48) | Extends `ParticleFX`; local serialized defaults | `GravityModifier=0.2`, `Damping=3.5` |
| `Levitate_wand` (139) | Extends `ParticleFX`; local serialized defaults | `GravityModifier=0.003`, `Chaos=1`, `ChaosDelay=0` |
| `RewardFail` (185) | Extends `Reward01`; local values override its gravity/damping defaults | `GravityModifier=0.3`, `Chaos=1`, `ChaosDelay=0`, `Damping=6` |
| `SmokeExplo_03` (219) | Script text extends `SmokeExplo_01`; local serialized default | `GravityModifier=-0.09` |
| `Spawn_flash_1` (231) | Extends `ParticleFX`; local serialized defaults | `Chaos=3`, `ChaosDelay=0.5` |
| `WizCard_Explo2` (312) | Script text extends `WizCard_Explo`; no local override | Inherits `GravityModifier=0.2`, `Damping=3.5` |
| `WizCardSpin` (316) | Extends `ParticleFX`; local serialized defaults | `Chaos=1`, `ChaosDelay=0`, `Damping=2` |
| `BadDrawPoint` (576) | Extends `GoodDrawPoint`; local serialized default | `GravityModifier=0.1` |

The shipped script for these classes does not mutate the listed fields at
runtime, so the serialized/inherited defaults explain the values observed in
the tutorial-level diagnostics.

## Reference-engine check

SurrealEngine does not implement the HP1 `ParticleFX` classes or the
`AParticleFX`/`UParticle` update routines. Searches for `GravityModifier`,
`ChaosDelay`, `AParticleFX`, and `UParticle` found no applicable implementation.
Its similarly named U227 emitter support only registers or reports unimplemented
`XParticleEmitter` operations in
`../SurrealEngine/SurrealEngine/UObject/U227Emitter.cpp` and
`../SurrealEngine/SurrealEngine/Native/N227Emitter.cpp`. It was therefore not
used to infer either formula.

## Completion criteria

Implement actor display state, environment mapping, TriggerLight brightness,
zone gravity, and chaos as separate logical changes. Focused synthetic checks
must be followed by release scans of all shipped maps, because the tutorial
does not exercise the full render inventory. The scan confirms capability and
diagnostic coverage, not visual equivalence: environment reflections, actor
fades, light transitions, particle trajectories, and attachments still need
manual comparison with the original game.

The particle formulas and authored values in this note are not guesses. The
documented compatibility choices are OpenHP1's deterministic particle random
stream, explicit zero initialization of the chaos timer, append-and-collapse
render topology, and the HP1-specific opacity color multiplier.
