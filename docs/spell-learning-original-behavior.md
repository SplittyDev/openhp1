# Original HP1 spell-learning trace and dialogue behavior

This note records the behavior compiled into the legally owned local HP1
packages and native Windows binaries. It focuses on the two remaining
spell-learning discrepancies: the beginning of the player's red trace
disappearing before the attempt ends, and lesson dialogue becoming much quieter
than nearby ordinary dialogue.

The primary evidence is:

- `res/System/HPBase.u`: `SpellLearnTrigger` source text, compiled state code,
  and compiled class defaults;
- `res/System/HPParticle.u`: `TemplateSparkle01`, `DrawBadPoint`, and
  `GoodDrawPoint` source text and compiled class defaults;
- `res/System/Engine.u`: `Actor` and `ParticleFX` declarations and compiled
  defaults;
- `res/System/Engine.dll`: `AParticleFX::EmitParticles`,
  `AParticleFX::InitExecution`, `UParticle::Update`, and
  `AActor::execPlaySound`;
- `res/System/Galaxy.dll`: `UGalaxyAudioSubsystem::PlaySoundW`, `Update`, and
  `SetVolumes`;
- `res/System/0/Default.ini` and `res/System/1/Default.ini`: the shipped Galaxy
  configuration.

Embedded `ScriptText` was inspected locally with `strings -a -n 1`. Claims
about active defaults and calls were checked against the serialized class
defaults and decoded bytecode. Native behavior was checked in the shipped x86
binaries with `objdump`. The source below is paraphrased; extracted game source
and generated dumps must not be committed.

## The live red trace is one persistent moving emitter

`SpellLearnTrigger.Draw` creates one `DrawFX` actor at the wand. Every tick it
moves that same emitter to the new wand location. While AltFire is held, it
enables emission by restoring `DrawFX.default.ParticlesPerSec`; while the
button is up, it sets the base rate to zero and resets `LastEmitLocation` to
the current location.

The state deliberately overrides the emitter's particle lifetime to zero. An
older block that would have made particles live for `DrawTime` and start fading
at 90 percent of that duration is commented out. The compiled `Draw` state
contains the active zero-lifetime assignment and the width/length assignments,
so this is not stale source text.

`UParticle::Update` in `Engine.dll` implements zero lifetime as persistent. At
`0x103bfc3e..0x103bfc69`, it performs the age-expiry comparison only when the
particle lifetime is positive. A lifetime of zero bypasses expiry.

Consequently, the lesson's red particles are intended to remain from the
first emitted point through the end of the user's attempt. `SpellLearnTrigger`
destroys `DrawFX` only after entering `Judge`, when it replaces the live trace
with the red/white judging replay. The original lesson does not gradually erase
the beginning of the live trace.

## `ParticlesAlive=200` is commented data, not the trace default

Both `GoodDrawPoint` and `TemplateSparkle01` contain what looks like an old
placed-emitter property dump in their embedded source text. That entire dump,
including `ParticlesAlive=200`, is enclosed in a block comment. It is not an
active `defaultproperties` value.

The compiled inheritance used by the red trace is:

```text
TemplateSparkle01 -> DrawBadPoint -> ParticleFX
```

The serialized defaults for all three classes contain no `ParticlesAlive` or
`ParticlesMax` override. `AParticleFX`'s native constructor at
`0x103c0e90..0x103c0f0c` also does not initialize either field to a positive
limit. It initializes its native particle list and runtime counters, leaving
the zeroed/default values intact.

`AParticleFX::EmitParticles` confirms the meaning of those zero values:

- At `0x103c23ae..0x103c23f9`, `ParticlesMax` is enforced only when it is
  positive.
- At `0x103c250e..0x103c257d`, `ParticlesAlive` is enforced only when it is
  positive. The positive path asks the particle list for its count and removes
  the oldest entries until the new particles fit. A zero or negative value
  skips that removal path.

Therefore the effective spell-trace settings are unlimited total emission,
no live-set eviction, and no age expiry. Any fixed CPU or GPU capacity imposed
by the replacement engine is an implementation detail that must grow without
discarding either end of this particular trace.

## Emission follows the entire wand path

The trace classes use uniform emission. The uniform branch of
`AParticleFX::EmitParticles` computes emission from the distance travelled
between the previous and current emitter positions and carries a fractional
residue between updates. New particles are distributed along the traversed
segment rather than all appearing only at the newest endpoint.

This matters for capacity planning: emission count depends on path length and
the authored rate, not just elapsed wall time. It does not change the lifetime
or live-set conclusion above. Even a long or jittery path must retain all of
the zero-lifetime trace particles until `DrawFX` is destroyed.

## Attempts and retries create fresh effects

The authored state transition after both success and failure destroys the
wand, template, and live draw actors. A failed first lesson level returns to
`Template`. That state calls `DrawTemplate` again before entering `Draw`, and
`Draw` spawns fresh `WandFX` and `DrawFX` actors. The judging state separately
spawns `BadDrawPoint` and `TemplateSparkle01` for its replay.

Thus neither the white template nor the red live trace is intended to be
reused across attempts. Every retry starts with newly spawned effects and
fresh particle lists.

## Lesson speech is an ordinary positional `PlaySound`

All `SpellLearnTrigger` narration helpers, including the introduction, retry,
judgement, and points lines, call `PlaySound(dlgSound, SLOT_Talk)` on the
trigger itself. The volume, radius, and pitch arguments are omitted. Decoded
`SayIntro` and `SayPoints` bytecode confirms a two-argument `PlaySound` call;
the other helpers have the same compiled shape.

The inherited `Actor` defaults supply `TransientSoundVolume=1` and pitch 1.0.
Neither `Trigger` nor `SpellLearnTrigger` overrides those values. The authored
spell-learning camera is only 20 Unreal units from the trigger, so the source
is intentionally very close to the listener.

Nearby ordinary dialogue is commonly authored differently: several HPBase
conversation paths call `PlaySound` with volume 3.2 and a large explicit radius
such as 2,000 or 20,000. This difference does not imply that ordinary dialogue
should be 3.2 times louder, because the original Galaxy backend saturates the
per-voice gain at unity.

## Galaxy gives `SLOT_Talk` no special attenuation

`UGalaxyAudioSubsystem::PlaySoundW` at `0x10607730` stores the raw authored
volume, radius, and pitch in the playing-sound record. It computes priority
from volume and distance for channel selection, but does not transform Talk
volume.

The regular sound path in `UGalaxyAudioSubsystem::Update` at
`0x10608856..0x10608a78` computes the per-voice level as:

```text
clamp(volume * clamp(1 - distance / radius, 0, 1), 0, 1)
```

It then scales that value to the 16-bit Galaxy range `0..32767`. The
`32767.0` constant is stored at `0x1063a524`. Master `SoundVolume` is applied
separately by `SetVolumes`; both shipped localized configurations use
`SoundVolume=200` and `EffectsChannels=16`.

The only slot-specific update branch in this Galaxy binary identifies
`SLOT_Ambient`. There is no corresponding `SLOT_Talk` boost, duck, spatial
mode, or attenuation branch. SurrealEngine's modern compatibility heuristic
that compresses volumes above one, and its Deus Ex-only Talk boost, are useful
reference behavior but are not present in HP1's shipped Galaxy mixer.

With the observed lesson geometry, a volume-1 source at 20 units and a default
radius around 1,500--1,600 reaches about 98.7 percent of full per-voice gain,
roughly 0.12 dB below unity. An ordinary volume-3.2 line at close range reaches
the same unity ceiling. The original mix therefore predicts almost no audible
level step merely from entering spell-learning mode.

## Implementation consequences

The original behavior rules out two tempting fixes:

- Do not impose the commented `ParticlesAlive=200` value on the lesson trace.
  The compiled trace has no live-particle cap and no particle expiry.
- Do not add an HP1-specific Talk-slot boost. The original Galaxy backend has
  no such rule.

For the disappearing trace, inspect every storage boundary after runtime
emission: the authoritative particle list, scene synchronization, render
instance packing, and GPU buffer resizing. The trace must preserve insertion
order and grow past its initial estimate when both effective lifetime and
`ParticlesAlive` are zero.

The decoded ordinary and lesson Hermione clips use the same MPEG layer-II
format, 22,050 Hz sample rate, mono channel layout, and 64 kb/s bit rate. Their
measured peaks are effectively full scale and their mean levels overlap; the
assets do not explain the mode-dependent drop.

The drop occurred in Kira routing. The lesson camera sits 20 units in front of
the trigger, so the trigger sound is directly behind the listener. A
full-strength Kira spatial track reduces each ear according to whether the
source faces that ear; a centered rear source therefore loses level in both
channels. Galaxy instead keeps front and rear centered sources at the computed
voice gain and uses lateral direction for left/right panning. OpenHP1 now
applies Galaxy's distance-and-volume clamp itself and drives Kira's ordinary
panning control, avoiding the unintended rear-source attenuation.
