# Original HP1 casting flow

This note records the casting behavior compiled into the legally owned local
HP1 packages. It intentionally paraphrases the embedded UnrealScript instead
of copying proprietary source into the repository.

The primary evidence is:

- `res/System/DefUser.ini`
- `res/System/HarryPotter.u`: `Harry`, `skHarry`
- `res/System/HPBase.u`: `baseHarry`, `Target`, `baseWand`, `baseSpell`,
  `spellnone`, `spelldud`, `spellFlip`, `SpellLearnTrigger`
- `res/System/HPParticle.u`: `TargetGlow`
- `res/System/HProps.u`: `FloatingSpellBook`
- `res/System/Engine.u`: compiled defaults for `Pawn`

The embedded `ScriptText` was inspected locally with `strings -a -n 1`, then
behavioral claims were checked against the compiled function bytecode and
class defaults through the package and script decoders. This distinction
matters: several statements remain in `ScriptText` but have no corresponding
compiled calls. Do not commit extracted source or generated dumps.

## Input enters aiming before target acquisition

`DefUser.ini` maps `LeftMouse` to the `AltFire` alias. The alias both updates
the held `bAltFire` button and invokes the `AltFire` exec, so the press enters
aiming and the state later observes release as `bAltFire == 0`.
Its `MouseY` binding feeds `aMouseY` with positive speed. HP1's UE1 rotator
convention maps positive pitch toward positive Z.

`Harry.AltFire` rejects the press when Harry is frozen or falling, the HUD is
in cutscene mode, a boss encounter has disabled casting, Harry has no weapon,
the press was already handled, or he is carrying something. It does **not**
require a spell target. An accepted press:

1. marks the weapon as pointing;
2. sets `bJustAltFired`;
3. enters `Harry.PlayerAiming`.

Although the embedded text still shows an initial `PlaySound` and
`weapon.altfire(1)`, neither call exists in compiled `Harry.AltFire` bytecode.
`baseWand.AltFire` exists as a function, but the ordinary compiled Harry input
path does not invoke it.

## `PlayerAiming` owns the hold/release sequence

On entry, compiled `PlayerAiming.BeginState` calls `DebugState`, resets
`fTimeToStop`, and calls `StartSoundFX`. Compiled `StartSoundFX` plays
`spell_build_nl2` and the looping `spell_loop_nl`. The embedded text's
`MovementMode(true)` and `HAR_raise_arm` calls are absent from the compiled
`BeginState` bytecode.

The state body always calls `Harry.makeTarget`, loops Harry's `wave`
animation, and marks the wand as casting. `makeTarget` spawns an
`HPBase.Target` 50 Unreal units ahead of Harry and gives it a reference back
to Harry. This happens whether or not any castable world actor exists.

While the button remains held, the `Target` actor ticks and moves the aiming
particles. On release, the state stops the build/loop sound, plays Harry's
`cast` animation at rate 2 with a 0.1 tween, waits until its frame reaches
0.95, and returns to `PlayerWalking`.

Both `skHarry` and `skSmallHarry` attach an animation notify to `cast` at
normalized time 0.1. That notify invokes `Harry.Cast`. There is no
target-dependent animation branch in `PlayerAiming`: the packaged script uses
the same `cast` sequence for success and failure.

## Target ray and auto-aim

`HPBase.Target.setTarget` runs every tick. In ordinary play its ray:

- starts at Harry's location, not at the camera;
- uses Harry's yaw plus a separate target pitch;
- reaches 512 units, or 1024 when `bExtendedTargetting` is enabled;
- clamps pitch to `-8000...12000` rotator units;
- changes pitch from `SmoothMouseY / 8` once the absolute input exceeds 256.

Normal targeting keeps the target's yaw offset at zero, so horizontal mouse
input turns Harry and therefore the ray. The separate yaw offset is used by
locked boss targeting.

The acquisition order is:

1. Iterate `TraceActors` along the ray and stop at the first actor whose
   `bProjTarget` or `bBlockActors` is true.
2. If that finds nothing, call `baseHarry.ExtendTarget`.
3. If auto-aim also finds nothing, perform a normal `Trace` so the free reticle
   can rest on world geometry; if that misses too, use the ray endpoint.

`ExtendTarget` considers visible `bProjTarget` actors other than the player
and `BaseCam`. A candidate must be strictly within 4000 rotator units in both
yaw and pitch and no farther than 512 units. The nearest qualifying candidate
wins. Four thousand rotator units are about 22 degrees, so the authored
auto-aim cone is deliberately broad.

The target test itself is `bProjTarget`; neither `setTarget` nor
`ExtendTarget` checks Harry's learned-spell list. A blocking actor occludes
auto-aim through the direct trace path. A selected projectile target receives
its `Targeted` event, becomes `Target.victim`, and is passed to
`Target.LockOn`.

`LockOn` sizes the reticle from the victim's world collision box and
`SizeModifier`, then asks the wand to choose the victim's
`eVulnerableToSpell`. It also obtains that chosen spell's gesture. Moving the
mouse outside the cone makes the next tick's `victim` selection start over;
the script does not set Harry's general `bLockedOnTarget` flag for ordinary
auto-aim. That flag is reserved for boss-style targeting. Losing the victim
calls the unlock path, which shuts down an active gesture effect; moving far
enough away therefore releases the snap and removes the target gesture.

## The reticle and spell gesture are different effects

`Target.startup` always creates two `HPParticle.TargetGlow` children. One is
red and one cyan; the `Target` emitter supplies the third component of the
floating aiming feedback. Each tick moves all three emitters to the traced,
auto-aimed, or free endpoint.

The spell-shaped gesture is separate. `Target.DrawSpellFX` is reached from
`LockOn`, after a `bProjTarget` victim has been selected and its vulnerability
has chosen a spell. Most targets display the gesture while aiming. A
`baseChar` with `bGestureOnTargeting == false` defers that gesture until the
target actor leaves its state during release.

No-target aiming does not call `LockOn` and therefore must not create a spell
gesture. This distinction is important because `spellnone` and `spelldud`
carry misleading compiled display defaults: both are named “Flipendo” and
reference `FlipPattern`. Those defaults do not make a no-target Flipendo
gesture authored behavior.

Destroying `Target` destroys both glow children and calls `StopEffect`.
`StopEffect` shuts down an active gesture, plays `spell_off_target3`, and
stops `spell_targetloop`.

## Release with and without a victim

`Harry.Cast`, invoked by the animation notify, uses the `victim` maintained by
the `Target` tick. A once-present final call to `ExtendTarget` is commented
out in the packaged source and is not part of the compiled release behavior.

With a victim, `Harry.Cast` passes that actor to `baseWand.CastSpell`. Without
a victim, it passes the reticle actor itself. The latter is intentional: it
provides a direction and target point for the failed cast.

The compiled `baseWand` default has `bAutoSelectSpell == true`, and
`baseWand.BeginPlay` sets `bUseNoSpell == true`. `CastSpell` therefore selects
the supplied actor's `eVulnerableToSpell` immediately before firing:

- a real vulnerable target selects its matching spell;
- the no-target reticle inherits `SPELL_None`, which makes `ChooseSpell`
  select `spellnone`.

The relevant direct mappings are `SPELL_Flipendo` to `spellFlip` and
`SPELL_Alohomora` to `spellAloho`; the switch also covers the other shipped
spell types. During the special `underAttack` path, `Harry.Cast` instead
chooses the attacker's vulnerability and uses `baseWand.forcespell`, bypassing
normal auto-selection for that shot.

Compiled `spellnone` defaults use the `spell_dud` cast sound, a short
`SmokeExplo_03` flying effect, and a 0.3-second lifespan. This is the authored
failure cast. It is not `spellFlip`, even though the class's legacy UI name
and gesture fields still say Flipendo.

After choosing the spell, `CastSpell` fires it, assigns its homing target and
original caster, and plays an incantation when applicable. `Harry.Cast` then
destroys the reticle.

## Spell learning is not a cast-time permission check

Compiled `Harry.PostBeginPlay` adds only `spelldud`; this makes `spelldud` the
initial `curSpell`. The embedded text still contains temporary calls adding
`spellFlip` and `spellAloho` and selecting `spellFlip`, but those three calls
are absent from the compiled bytecode. In either case, the current selection
does not control a normal targeted cast: `CastSpell` auto-selects from the
current target immediately before firing.

`SpellLearnTrigger.Trigger` adds its configured spell at the start of the
lesson and selects it; the source notes that the lesson cannot be failed.
`FloatingSpellBook.Touch` also adds its configured spell. The only other
packaged use of `spellList` is spell-list UI and manual selection.

Neither `baseWand.ChooseSpell` nor target lock-on checks whether the chosen
class appears in `spellList`. In the shipped logic, progression is enforced
primarily by level state and which actors are exposed as `bProjTarget` with a
particular vulnerability, not by a new cast-input guard. Reimplementing a
learned-spell check in the input path would diverge from the original scripts.

## OpenHP1 compatibility consequence

`Engine.Pawn` has compiled defaults `bCollideActors == true`,
`bBlockActors == true`, and `bProjTarget == true`; Harry inherits them.
OpenHP1 had reversed the authored `Start` and `End` arguments before running
`TraceActors`. The backwards sweep approached Harry at the terminal end of the
ray and falsely selected him as the victim. That false lock-on selected
`spellnone`, read its legacy `FlipPattern`, and drew a Flipendo-shaped gesture
at Harry.

UE1 traces from `Start` toward `End`. OpenHP1's shared actor sweep already
omits an actor overlapping the trace origin, so preserving the authored
direction also preserves the original result: Harry is not returned by the
ray that begins at his location. This belongs at the shared
`TraceActors`/collision seam, not in `PlayerAiming` or a Harry-specific casting
guard.

The auto-aim fallback has a separate authored self-filter. Compiled
`ExtendTarget` rejects a visible candidate when it equals `Self`. OpenHP1's VM
represents `Self` internally with a sentinel object handle, but implicit native
calls had compared that sentinel literally instead of resolving it to the
current actor. The filter consequently admitted Harry whenever his own
location fell inside the auto-aim cone. Resolving `Self` at the common actor
call boundary restores the compiled comparison and other actor calls that pass
`Self`; no casting-specific exclusion is needed.

Independently, every accepted aim must spawn and update `Target` plus both
`TargetGlow` actors even when no valid target exists. Missing free-aim
particles are therefore a particle spawning, ticking, tracing, or projection
defect—not intended spell-progression behavior.

## Package evidence index

| Package | Compiled object | Relevant evidence |
| --- | --- | --- |
| `HarryPotter.u` | `Harry` class export 43 | `AltFire` 8, `PostBeginPlay` 175, `makeTarget` 672, `PlayerAiming` state 915, `Cast` 992 |
| `HarryPotter.u` | `skHarry` class export 902 | `cast` animation notify calls `Cast` at 0.1 |
| `HPBase.u` | `baseHarry` class export 0 | `ExtendTarget` function 2913 |
| `HPBase.u` | `Target` class export 321 | `seeking` state 2268, `setTarget` 2310, `DrawSpellFX` 2385, `StopEffect` 1937, `Destroyed` 2406 |
| `HPBase.u` | `baseWand` class export 25 | `ChooseSpell` 2017, `CastSpell` 2021; compiled `bAutoSelectSpell=true` |
| `HPBase.u` | spell classes | `spellFlip` 225, `spellnone` 1290, `spelldud` 1313 |
| `HPParticle.u` | `TargetGlow` | red/cyan glow setup and lock/unlock particle sizing |
| `Engine.u` | `Pawn` | compiled collision and projectile-target defaults inherited by Harry |
