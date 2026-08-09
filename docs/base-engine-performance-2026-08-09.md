# Base-engine performance analysis: 2026-08-09

## Scope

This investigation covers the runtime, scene, animation, physics, scripting,
audio-facing state, and game-side update paths. Renderer work is deliberately
out of scope. The analysis started at `42c542c` and used the real
`res/Maps/Lev_Tut1.unr` corpus through the release `runtime_scan` workload.

Candidates were kept only after a matched release A/B test and identical
deterministic behavior counters. The counters cover timer callbacks, state
resumes, actions, animations, sounds, music, spawns, movement, rotation,
visibility, destruction, and script logs. These checks do not replace an
authored live replay for visual animation and attachment behavior.

## Result

Three independent changes removed repeated skeletal-animation work:

| Commit | Change | Matched result |
| --- | --- | --- |
| `d8adb81` | Skip skeletal pose sampling for meshes that have no attachment vertices or weapon bone | 5 s median: 3.02 to 2.82 s wall (6.6%); 2.87 to 2.69 s user CPU (6.3%) |
| `287aa60` | Sample indexed animation vertices once, then expand faces; transform, normal-transform, and light each shared vertex once when tweening permits it | 5 s median: 2.87 to 2.24 s wall (22.0%); 2.69 to 2.06 s user CPU (23.4%) |
| `f4e1aba` | Carry the already-computed skeletal bone palette into scene synchronization instead of resampling the same pose | 30 s median: 9.95 to 9.79 s wall (1.6%); 9.68 to 9.34 s user CPU (3.5%) |

All three comparisons produced identical behavior counters. The final
30-second run reported 43 timer callbacks, 13,305 state resumes, 141,638
actions, 1,726 applied animations, 81 spawns, 78,782 `SetLocation` actions,
56,957 `SetRotation` actions, and 20 destroyed actors in both builds.

The profile changed in the expected places:

- skipping irrelevant attachment work reduced sampled
  `SkeletalAnimation::sample_pose` stacks from about 515 to 280 samples;
- unique animated-vertex processing reduced `ActorVertexLighting::color`
  from about 1,045 to 318 samples;
- reusing the sampled bone palette reduced the remaining pose samples from
  about 428 to 285 in matched steady-state captures.

The sample counts are diagnostic rather than percentages: separate five-second
`sample` captures can vary with CPU frequency and sampling alignment. The
release timings above are the acceptance measurements.

## Why these paths were slow

The skeletal decoder naturally produces one position per source vertex, but
the scene animation path immediately expanded that result into complete
triangles. Shared vertices were therefore copied, transformed, normal-
transformed, and vertex-lit once for every triangle corner that referenced
them. The runtime then separately sampled skeletal poses for attachment and
bone queries, even when the mesh could not provide an attachment or when the
same pose had already been sampled for its visible vertices.

The fixes keep the existing decoder, transforms, lighting function, tweening,
root motion, face order, and render buffers. They only retain and reuse values
that the same frame had already computed.

## Rejected experiments

- Hoisting the unlit animation color out of the vertex loop was slower in the
  matched run: 2.95 s wall / 2.80 s user CPU versus 2.87 / 2.69 s. It was
  reverted.
- Indexed sampling by itself was borderline: about 2% lower wall time and
  effectively unchanged user CPU. It was not committed alone. Reusing the
  indexed positions at the actual transform, normal, and lighting consumers
  produced the material second result above.
- Sparse physics and lifespan actor sets were not implemented. The steady
  profile attributed little time to those broad traversals, while correct
  invalidation would have to cover property writes, spawn, destruction,
  restore, and native paths. The complexity and behavior risk are not
  justified by the current evidence.

## Remaining profile

After the retained changes, work is spread across skeletal pose sampling,
vertex-normal construction, BSP collision sweeps, movement, script execution,
and allocation/copy routines. None was a comparable isolated hotspot in this
workload.

The next plausible narrow candidate is to reuse the visible skeletal sample
for the subset of meshes that really do compute weapon attachments. It should
not be attempted until an authored attachment-heavy capture shows it is still
material; tween transitions, static poses, restore, and root motion all share
that state. Likewise, changing collision broad phases, physics stepping, tick
ordering, or script dispatch to reduce work would require original-engine
evidence and much stronger profiling because those changes can alter gameplay.

This headless workload deliberately removes rendering and presentation from
the measurement. It proves substantial avoidable base-engine work existed,
but it does not claim that the game's reported 14 ms frame will fall by the
same percentage. A release Classic replay with the same save, camera path,
and uncapped settings is still needed to measure the end-to-end frame-time
change and visually confirm skeletal animation, tweening, and attachments.

## Verification protocol

- Release A/B runs used an isolated `OPENHP1_SETTINGS_DIR` and the same map,
  duration, executable, and output counters.
- Focused `openhp1-mesh` and `openhp1-scene` tests ran after each retained
  optimization; the final focused run executed 53 tests.
- Strict Clippy passed for both changed crates after allowing three documented
  pre-existing scene lints (`field-reassign-with-default`, `obfuscated-if-else`,
  and `type-complexity`).
- `cargo check --workspace` passed. Workspace nextest ran 376 tests
  successfully; six GPU-dependent tests were skipped by their existing gates.
