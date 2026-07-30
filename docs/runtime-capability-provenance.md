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

The shipped files were sufficient, so no online source or behavioral guess was
needed. In this note:

- **Game script, high confidence** means a declaration, comment, inheritance
  relationship, or serialized class default in a shipped package.
- **Game native, high confidence** means behavior recovered from the shipped
  `Engine.dll` implementation.
- **Implementation choice** means a deliberate OpenHP1 detail that is not
  expected to reproduce the original engine bit for bit.

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

The same log contains 15 render-property diagnostics. Fourteen belong to two
spawned `baseWand` actors; the remaining diagnostic is `Ghost0.Style`.

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

### UE1 rendering boundary

**SurrealEngine reference, high confidence:** a carried weapon's third-person
geometry is independent of its standalone `Actor.Mesh`.
`../SurrealEngine/SurrealEngine/Render/VisibleMesh.cpp` selects
`Weapon.ThirdPersonMesh` and `ThirdPersonScale` while drawing the owning pawn,
then uses the weapon actor for its material and lighting properties. Clearing
`Actor.Mesh` must therefore hide the standalone actor without removing the
carried wand attachment.

The same reference maps translucent actor style to the translucent mesh pass,
uses `bUnlit` to bypass dynamic lighting, and adds `bMeshEnviroMap` reflection
UVs. OpenHP1's existing translucent blend already matches the reference blend.
None of the logged `Texture`, `bUnlit`, or `bMeshEnviroMap` assignments changes
value, so the latest trace does not justify implementing those unexercised
features.

### OpenHP1 implementation boundary

Project only effective typed render-property writes into the scene:

- `Mesh=None` hides the standalone actor geometry while retaining a matching
  `ThirdPersonMesh` attachment template.
- `AmbientGlow` adjusts the actor's baked vertex-light contribution.
- translucent `Style` changes the actor surface mode and forces renderer batch
  rebuilding.

Keep changed, unsupported values diagnostic. In particular, a future non-null
mesh replacement or a true `bMeshEnviroMap` assignment must not be silently
treated as supported.

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

`ParticleEmitter` already carries `gravity_modifier` and `gravity` in
`crates/openhp1-runtime/src/world/action.rs`, and
`ScriptRuntime::particle_emitters` reads both live instance properties in
`crates/openhp1-runtime/src/world/actor.rs`. The scene currently applies only
the authored gravity in `LoadedScene::tick_particles` in
`crates/openhp1-scene/src/loader.rs`.

The runtime already has the correct central zone lookup in
`ScriptRuntime::zone_physics` in
`crates/openhp1-runtime/src/world/physics/events.rs`: it finds the BSP zone at
an actor location, resolves the zone actor, falls back to `LevelInfo`, and
reads `ZoneGravity`. Reuse or factor that seam when projecting a particle
emitter. Do not add a second zone/object-resolution path in the scene crate.
The projected emitter should carry either the resolved zone gravity or the
already combined acceleration; the scene must remain unaware of package
objects and BSP zone actors.

### Focused tests

- With authored gravity `[1, 2, 3]`, zone gravity `[0, 0, -950]`, and modifier
  `0.3`, the projected acceleration is `[1, 2, -282]`.
- Modifiers `0`, positive, and negative values preserve the formula without
  clamping.
- A synthetic zone actor supplies its `ZoneGravity`; an unmapped/default zone
  uses the active `LevelInfo` fallback.
- Applying the combined acceleration before the existing damping step is
  covered by a scene particle-update test.

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

`ParticleEmitter` currently carries `chaos` but not `ChaosDelay`, and the scene
particle state has no chaos timer. Add:

1. `chaos_delay` to the runtime-to-scene emitter projection, read from the live
   `ChaosDelay` instance property.
2. A per-particle `chaos_timer`, initialized to zero.
3. The native timer and kick sequence after the existing position/attraction
   work, using the particle system's existing random stream.

Using OpenHP1's deterministic per-emitter random stream is an
**implementation choice**. The original uses the process-global `appFrand`
stream, whose exact sequence depends on unrelated engine activity. Reusing the
local stream preserves reproducible tests and effects while matching the
recovered formula and update order; it is not intended to reproduce the
original random directions bit for bit.

### Focused tests

- A controlled random triple equivalent to `[1, 0.5, 0.5]` produces a
  positive-X unit direction, and a `Chaos` value of `3` changes velocity by
  exactly three units for both short and long `delta_time` values.
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

Implement zone gravity and chaos as separate logical changes. After focused
synthetic tests, a release `Lev_Tut1.unr` replay should no longer emit the 29
zone-gravity or 16 chaos diagnostics listed above. That confirms capability
coverage, not visual equivalence: particle trajectories and appearance still
need manual comparison with the original game.

No formula or authored value in this note is an educated guess. The only
compatibility choices are the deterministic OpenHP1 random stream and
explicitly zero-initializing the native timer state.
