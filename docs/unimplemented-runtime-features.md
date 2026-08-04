# Runtime feature implementation ledger

This ledger records the UnrealScript and adjacent runtime gaps identified from
the 41 shipped maps, the behavior required to close each gap, and their current
implementation status. Every listed item is now implemented for its audited
shipped-level use. The original failure and implementation-seam notes remain as
acceptance criteria and future maintenance context, not as claims about current
support.

Passing corpus scans and synthetic tests establish audited runtime coverage;
they do not by themselves prove pixel-perfect rendering, original-game replay
equivalence, or support for unaudited UE1 content.

## Scope and evidence

The corpus is the 41 `.unr` files in `res/Maps` from the local legally obtained
game installation. The inventory combines two kinds of evidence:

- **Static use** means a level actor's class, its base-class chain, or nested
  `Function`/`State` exports contain the feature. This conservative authored
  reachability is authoritative for code behind triggers, timers, states, or
  input that a neutral replay may not enter, but does not prove the branch runs.
- **Replay use** means a release `runtime_scan` replay reached OpenHP1's
  unsupported/partial path. Replay evidence proves impact on that neutral path,
  but absence from a replay does not prove a feature is unused.

OpenHP1's decoder and dispatcher are the authority for implemented support:
[`opcode.rs`](../crates/openhp1-runtime/src/opcode.rs),
[`frame/execute.rs`](../crates/openhp1-runtime/src/frame/execute.rs),
[`world/native.rs`](../crates/openhp1-runtime/src/world/native.rs), and
[`world/execution/dispatch.rs`](../crates/openhp1-runtime/src/world/execution/dispatch.rs).
Canonical token names and reference behavior come from the local SurrealEngine
clone, primarily
[`UClass.h`](../../SurrealEngine/SurrealEngine/UObject/UClass.h#L227),
[`Bytecode.cpp`](../../SurrealEngine/SurrealEngine/VM/Bytecode.cpp#L25), and
[`ExpressionEvaluator.cpp`](../../SurrealEngine/SurrealEngine/VM/ExpressionEvaluator.cpp#L1).
SurrealEngine is a licensed implementation reference, not proof of exact HP1
behavior; original-game differential replay remains the final authority.

Level lists below omit the `.unr` suffix. They identify level reachability, not
estimates of how often a player reaches the path. A feature with only replay
evidence says so explicitly.

To keep repeated level lists readable:

- **All maps** means all 41 files under `res/Maps`.
- **All non-Entry maps** means all maps except `Entry` (40 maps).
- **All gameplay maps** means all maps except `Entry` and `startup` (39 maps).

## Inventory

The initial 2026-08-03 audit attempted every map for 120 simulated seconds with
the release `runtime_scan`. It exposed the gaps below and a 14-map HPHud setup
blocker. The final post-implementation audit on 2026-08-04 rebuilt the scanner
from commit `808d4a4`, then completed all 41 maps for the same 120 simulated
seconds. Every map exited successfully, and all per-map logs were free of
`deferred`, `failed`, `unknown`, `Error:`, unsupported opcode/native,
unimplemented, and ConsoleCommand-failure diagnostics.

The replay command for each map was:

```sh
env RUSTC_WRAPPER= cargo run --release -p openhp1-scene \
  --example runtime_scan -- res/Maps/<level>.unr 120
```

The static pass conservatively collected each serialized level actor's class,
walked its full base-class chain, and scanned nested function/state bytecode for
the audited opcode and native indexes. It excluded the bytecode payloads of
functions declared `native`, because those declarations describe engine entry
points rather than script that can execute. It did not count unrelated classes
that merely share the same package. The filtered dispatcher diff produced 26
missing numeric native indexes; every one has a section below. Named-native
reachability is the conservative intersection of a reachable native declaration
and a reachable call name, with the declaring class recorded for implementation.
The final reachable-script and reachable-named scans were byte-identical to the
initial inventory, so implementation did not reveal additional statically
reachable constructs.

The short table highlights the original replay-observed and non-dispatcher
gaps. The per-feature sections, rather than this summary table, are the complete
52-item ledger.

| Feature | Original gap | Replay-observed levels |
| --- | --- | ---: |
| `EatString` (`0x0e`) | Missing VM opcode | 0; statically reachable |
| `MetaCast` (`0x13`) | Missing VM opcode | 0; statically reachable |
| `Sin` (`0x0bb`) | Missing numeric native | 18 |
| `PlayerCanSeeMe` (`0x214`) | Missing numeric native | 3 |
| `Multiply_VectorVector` (`0x128`) | Missing scalar native | 2 |
| `BonePos` (`0x101`) | Missing HP1 native | 2 |
| `RadiusActors` (`0x136`) | Missing iterator native | 0; statically reachable |
| `VisibleCollidingActors` (`0x138`) | Missing iterator native | 0; statically reachable |
| `NameToString` (`0x57`) | Partial conversion | 1 |
| `PatrolPoint.PreBeginPlay` does not terminate | VM/runtime semantic gap | 1 |
| Nested `HPSounds` import does not resolve | Object-resolution gap | 1 |
| HPHud setup assertion | HUD subclass/conversion/native-call blockers | 14 |
| Serialized `DamageType` names | Physics value-decoding gap | 40 |
| Final-function failure deferral | Partial VM error semantics | 19 |
| Non-billboard particle `RenderPrimitive` | Missing particle render mode | 8 initialized levels |
| Particle collision `Elasticity` | Missing particle collision response | 2 initialized levels |
| Particle `WindModifier` | Missing zone-wind response | 4 initialized levels |
| Particle `bVelocityRelative` | Missing emitter-velocity inheritance | 1 initialized level |

### `EatString` — opcode `0x0e`

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** None on the neutral path.
- **Required behavior:** Evaluate the nested expression for its side effects,
  discard its value, and return no value. The serialized layout is the opcode
  followed by one expression
  ([`Bytecode.cpp`](../../SurrealEngine/SurrealEngine/VM/Bytecode.cpp#L198));
  SurrealEngine's evaluator confirms the discard behavior
  ([`ExpressionEvaluator.cpp`](../../SurrealEngine/SurrealEngine/VM/ExpressionEvaluator.cpp#L116)).
- **Implementation seam:** Add an execution arm that uses the existing nested
  expression evaluator. Do not skip the bytes or suppress the child's side
  effects.

### `MetaCast` — opcode `0x13`

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** None on the neutral path.
- **Required behavior:** Evaluate an object expected to be a class object and
  return it only when it is the serialized target metaclass or derives from
  that metaclass; otherwise return `None`. The token stores a class reference
  followed by the value expression
  ([`Bytecode.cpp`](../../SurrealEngine/SurrealEngine/VM/Bytecode.cpp#L236));
  reference evaluation walks the class `BaseStruct` chain
  ([`ExpressionEvaluator.cpp`](../../SurrealEngine/SurrealEngine/VM/ExpressionEvaluator.cpp#L210)).
- **Implementation seam:** Reuse the runtime's central class-resolution and
  `class_is_a` paths. This is a cast of class objects, not the ordinary
  instance `DynamicCast` opcode.

### `Sin` — native `0x0bb` (187)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay-observed levels:** `Lev2_Fire2`, `Lev2_HogFront`, `Lev2_Inc_A`,
  `Lev2_Inc_B`, `Lev2_fire1`, `Lev3_Dungeon`, `Lev3_DungeonB`, `Lev3_Intro`,
  `Lev3_Lumos`, `Lev3_Troll`, `Lev4_Sneak`, `Lev4_Sneak2`, `Lev5_Chess`,
  `Lev5_Final`, `Lev5_fluffy`, `Lev_Tut1b`, `Lev_Tut3`, `Lev_Tut3b`.
- **Observed callers:** `savepoint` and `Star` ticks.
- **Original failure:** Unrecognized scalar indexes end in
  `native 0x0bb is not implemented`
  ([`world/native/scalar.rs`](../crates/openhp1-runtime/src/world/native/scalar.rs#L162)).
- **Required behavior:** Return the sine of one float in radians. SurrealEngine
  registers index 187 and delegates to `std::sin`
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L163),
  [`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L1446)).
- **Implementation seam:** Add one `ScalarNative` mapping and match arm beside
  the existing `Tan` and `Sqrt` operations.

The remaining scalar entries below were found by diffing every numeric native
called from reachable, non-native script bytecode against every current world,
iterator, gesture, and scalar dispatcher arm. Unless noted otherwise, each
falls through to `native ... is not implemented`. Mutating operators must keep
their first argument as an lvalue and return the exact pre/post-operation value;
implementing only the arithmetic result is not sufficient.

### `LessLess_IntInt` — native `0x094` (148)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Return signed integer `A << B`, matching UE1's shift
  count and overflow behavior. SurrealEngine registers it at index 148 and
  implements the direct shift in
  [`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L1027).
- **Implementation seam:** Add a scalar integer arm, but first pin invalid or
  oversized shift counts with a synthetic bytecode test rather than relying on
  Rust's debug/release shift behavior.

### `GreaterGreater_IntInt` — native `0x095` (149)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Arithmetic right-shift the signed integer `A` by `B`.
  See [`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L914).
- **Implementation seam:** Share validated shift-count handling with
  `LessLess_IntInt` while retaining sign extension.

### `MultiplyEqual_IntFloat` — native `0x09f` (159)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Multiply the integer lvalue by the float, convert the
  result back to integer using HP1's truncation rules, store it, and return it.
  See [`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L1141).
- **Implementation seam:** Requires lvalue write-through plus an explicitly
  tested float-to-int conversion for NaN, infinity, and overflow.

### `DivideEqual_IntFloat` — native `0x0a0` (160)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable level:** `Lev5_Final`.
- **Required behavior:** Divide the integer lvalue by a float, convert back to
  integer, store it, and return it. SurrealEngine's direct reference is
  [`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L600).
- **Implementation seam:** Share conversion and lvalue behavior with
  `MultiplyEqual_IntFloat`; establish HP1's zero-divisor result by differential
  replay before choosing an error or fallback value.

### `SubtractEqual_IntInt` — native `0x0a2` (162)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Subtract the right integer from the left lvalue, store
  the result, and return it
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L1511)).
- **Implementation seam:** Use the same lvalue and overflow path as
  `AddEqual_IntInt`.

### `Cos` — native `0x0bc` (188)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable level:** `Lev5_Final`.
- **Required behavior:** Return the cosine of one float in radians
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L547)).
- **Implementation seam:** Add beside `Sin`, `Tan`, and `Sqrt` in
  `ScalarNative`.

### `GreaterGreaterGreater_IntInt` — native `0x0c4` (196)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Logically right-shift `A` as an unsigned 32-bit value
  by `B`, then return the result in the script integer type
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L909)).
- **Implementation seam:** Share shift-count validation with the two signed
  shift operators while preventing sign extension.

### `Cross_VectorVector` — native `0x0dc` (220)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Return the vector cross product
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L552)).
- **Implementation seam:** Add to `ScalarNative` using `glam::Vec3::cross` and
  the existing Unreal/render coordinate convention only at its established
  boundary.

### `PlayerCanSeeMe` — native `0x214` (532)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable levels:** `Lev2_Fire2`, `Lev2_HogFront`,
  `Lev2_HogFront_2`, `Lev2_HogFront_3`, `Lev2_Inc_A`, `Lev2_Inc_B`,
  `Lev2_RemChase`, `Lev2_fire1`, `Lev3_Dungeon`, `Lev3_DungeonB`, `Lev3_Intro`,
  `Lev3_Lumos`, `Lev3_PreDungeon`, `Lev3_PreTroll`, `Lev3_Quid2`, `Lev3_Troll`,
  `Lev4_Sneak`, `Lev4_Sneak2`, `Lev5_Chess`, `Lev5_Final`, `Lev5_FlyKeys`,
  `Lev5_Snare`, `Lev5_fluffy`, `Lev_Tut1`, `Lev_Tut1b`, `Lev_Tut2`, `Lev_Tut3`,
  `Lev_Tut3b`, `Snapes_Office`.
- **Replay-observed levels:** `Lev2_Fire2`, `Lev2_fire1`, `Lev3_Troll`.
- **Observed caller:** `FireCrab.Tick` through a global function.
- **Original failure:** Falls through the world-native dispatcher to the scalar
  native error.
- **Required behavior:** Return true when a pawn in `Level.PawnList`, other than
  the actor itself, is close enough, has the actor inside its view cone unless
  using behind view, and has line of sight from its eye position. SurrealEngine's
  reference implementation is
  [`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L2547),
  registered at index 532 in
  [`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L66).
- **Implementation seam:** Reuse the runtime pawn list, view rotation, and BSP
  trace path already used by `CanSee`; do not add a second visibility system.
  Differentially verify HP1's exact distance and cone tests.

### `Multiply_VectorVector` — native `0x128` (296)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All gameplay maps.
- **Replay-observed levels:** `Lev3_Intro`, `Lev_Tut3b`.
- **Observed caller:** `gen_male_5.SetInitialState`.
- **Required behavior:** Return the component-wise product of two vectors.
  SurrealEngine registers index 296 and uses its vector component multiplication
  operator
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L137),
  [`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L1223)).
- **Implementation seam:** Add the index to `ScalarNative` and one vector match
  arm next to the existing vector add/subtract operations.

### `BonePos` — HP1 native `0x101` (257)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable levels:** `Lev5_fluffy`, `Lev5_Snare`.
- **Replay-observed levels:** `Lev5_fluffy`, `Lev5_Snare`.
- **Observed callers:** `Fluffy.PostBeginPlay`, `DevilsSnareNew.Timer`, and
  `SausageRollKidOnAStick.Tick`.
- **Required behavior:** Return the requested skeletal bone's current position
  in the coordinate space HP1 exposes to UnrealScript, including the actor's
  current animation pose and transform.
- **Reference limit:** SurrealEngine confirms the HP1 registration and signature
  ([`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L126),
  [`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L1061)) but
  its implementation returns zero and logs unimplemented
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L2798)).
  It is not semantic evidence for the returned coordinate space.
- **Implementation seam:** Extend the existing skeletal pose sampling used for
  weapon attachments and root motion, select the bone through the runtime's
  retained bone-name list, and transform its sampled origin through the actor
  transform. Compare at least one animated and one bind-pose result with HP1
  before fixing the public coordinate convention.

### `RadiusActors` — iterator native `0x136` (310)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** None on the neutral path.
- **Original failure:** The iterator dispatcher accepts only `AllActors`,
  `TraceActors`, and `VisibleActors`; every other index returns
  `iterator function is not implemented`
  ([`world/execution/dispatch.rs`](../crates/openhp1-runtime/src/world/execution/dispatch.rs#L481)).
- **Required behavior:** Yield actors derived from the requested base class
  whose locations are within `Radius` of the optional `Loc`, defaulting to the
  receiver's location. SurrealEngine registers the signature in
  [`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L585) and
  filters the level actor list in
  [`Iterator.cpp`](../../SurrealEngine/SurrealEngine/VM/Iterator.cpp#L234).
- **Implementation seam:** Reuse the existing class test, actor location, and
  `IteratorValue` output path used by the supported actor iterators.

### `VisibleCollidingActors` — iterator native `0x138` (312)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All maps.
- **Replay observation:** None on the neutral path.
- **Required behavior:** Query collision actors within the optional radius and
  location, defaulting to the receiver's collision radius and location; yield
  only the requested class and, for HP1's pre-220 signature, skip hidden actors.
  Despite the name, the reference iterator performs no line-of-sight trace.
  See SurrealEngine's registration
  ([`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L724)) and
  filtering loop
  ([`Iterator.cpp`](../../SurrealEngine/SurrealEngine/VM/Iterator.cpp#L369)).
- **Implementation seam:** Reuse the runtime collision-actor cache and existing
  iterator output path. Match HP1's four-argument/pre-220 defaults rather than
  the later optional `bIgnoreHidden` overload.

### `Warp` — native `0x13a` (314)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable level:** `Lev5_Final`.
- **Required behavior:** Transform location, velocity, and rotation from the
  paired warp zone's coordinate space into this warp zone's space, mutating all
  three parameters. SurrealEngine registers the HP-era index/signature in
  [`NWarpZoneInfo.cpp`](../../SurrealEngine/SurrealEngine/Native/NWarpZoneInfo.cpp#L6)
  and applies the inverse warp transform in
  [`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L4415).
- **Implementation seam:** Add warp-zone transform state to the runtime object
  model and support three native output parameters. Keep coordinate conversion
  in the existing central Unreal transform module.

### `UnWarp` — native `0x13b` (315)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable level:** `Lev5_Final`.
- **Required behavior:** Apply the inverse of `Warp` to the location, velocity,
  and rotation lvalues. See the paired registration and implementation in
  [`NWarpZoneInfo.cpp`](../../SurrealEngine/SurrealEngine/Native/NWarpZoneInfo.cpp#L6).
- **Implementation seam:** Implement together with `Warp` and test round-trip
  location, velocity, and rotator behavior.

### `RotRand` — native `0x140` (320)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable level:** `Lev5_Final`.
- **Required behavior:** Return a uniformly randomized Unreal rotator with
  randomized pitch and yaw and optional roll
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L1423)).
- **Implementation seam:** Use the runtime's deterministic RNG source so
  replay remains reproducible; preserve the full 16-bit Unreal angle range.

### `LineOfSightTo` — native `0x202` (514)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable level:** `Lev5_Final`.
- **Required behavior:** Return whether the pawn has line of sight to another
  actor, including the UE1 alternate head/body visibility probes rather than a
  single center-point ray. SurrealEngine routes the native through
  [`NPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPawn.cpp#L148).
- **Implementation seam:** Reuse the BSP/actor trace path and any visibility
  helpers built for `CanSee` and `PlayerCanSeeMe`; do not create a second LOS
  system.

### `FindPathTo` — native `0x206` (518)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Clear cached paths by default, locate the closest
  navigation point to the requested world position, and return the next
  navigation point along the selected route. The optional flags control
  single-path search and cache clearing. See
  [`NPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPawn.cpp#L118) and
  [`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L3602).
- **Implementation seam:** Extend the navigation cache already used by HP1
  native `FindPath` (`0x229`); do not conflate the two signatures.

### `actorReachable` — native `0x208` (520)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable levels:** All maps except `Entry`, `startup`,
  `Lev2_RemChase`, and `Lev5_Chess` (37 maps).
- **Required behavior:** Test whether the pawn can physically reach the target
  actor using UE1 reachability rules, including navigation-point treatment;
  this is not a line-of-sight query. SurrealEngine delegates to
  `ActorReachable(..., true)` in
  [`NPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPawn.cpp#L276).
- **Implementation seam:** Reuse BSP movement sweeps and navigation reachspecs
  from the existing pawn pathfinder.

### `FindStairRotation` — native `0x20c` (524)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Return the pawn pitch/rotation adjustment used while
  traversing stairs for the supplied delta time.
- **Reference limit:** SurrealEngine confirms the signature and index but also
  returns zero as unimplemented
  ([`NPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPawn.cpp#L142)).
  Recover HP1's slope sampling, interpolation, and return units from original
  behavior before implementation.
- **Implementation seam:** Build on the existing floor-normal and walking
  collision state rather than adding a visual-only stair heuristic.

### `PickAnyTarget` — native `0x216` (534)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Select an eligible pawn target for the supplied fire
  direction and projectile start while updating the `bestAim` and `bestDist`
  output lvalues. SurrealEngine's native wrapper delegates to the pawn target
  selector in
  [`NPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPawn.cpp#L167).
- **Implementation seam:** Extend the existing `PickTarget` scoring/list walk
  and add native output-argument copying.

### `UpdateURL` — native `0x222` (546)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Add or replace an option in the current level URL. The
  later overload also accepts a separate value and can persist it as a default;
  HP1's exact serialized signature must govern. SurrealEngine implements both
  versioned forms in
  [`NPlayerPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPlayerPawn.cpp#L105).
- **Implementation seam:** Reuse the game host's travel URL/configuration model.
  Do not let this runtime native mutate package data.

### `FastTrace` — native `0x224` (548)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable levels:** `Lev4_Sneak`, `Lev4_Sneak2`.
- **Required behavior:** Return whether an unobstructed world trace exists from
  the optional start (defaulting to the receiver's location) to the requested
  end. SurrealEngine's wrapper is
  [`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L244).
- **Implementation seam:** Reuse the BSP trace path with the original native's
  fast/world-only filter semantics.

### `ModifySound` — HP1 native `0x237` (567)

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable levels:** `Lev2_Quid1`, `Lev2_RemChase`, `Lev3_Quid2`,
  `Lev5_FlyKeys`, `Lev_Tut2`, `Quid_HuffleA`, `Quid_HuffleB`, `Quid_HuffleC`,
  `Quid_RavenA`, `Quid_RavenB`, `Quid_RavenC`, `Quid_SlythA`, `Quid_SlythB`,
  `Quid_SlythC`.
- **Implemented behavior:** Decode the shipped declaration as
  `ModifySound(parameter, value, optional sound, optional slot)`. Match the
  receiver's live sound channel by slot and, when supplied, sound object;
  update volume, radius, or pitch for parameter values 0, 1, or 2. Return
  `false` without emitting an action when no live matching channel exists.
- **Required behavior:** Modify an already playing HP1 sound's authored
  playback parameters. The shipped metadata confirms the name/index, but the
  local SurrealEngine clone does not register or implement this HP1 native.
- **Original investigation:** Record the exact parameter properties and output flags
  from the shipped `Function` export, then differentially replay changes to
  volume, radius, pitch, slot, and/or actor association. Do not infer the
  signature from `PlaySound` merely because the affected properties overlap.
- **Implementation seam:** Extend `openhp1-audio`'s existing sound handle/action
  path once the target-selection and parameter semantics are established.
- **Verification:** The real serialized byte/float/object/byte call shape has a
  runtime regression. All 14 statically reachable maps completed the
  120-second replay after the decoder correction in `148f603`.

### `AutonomousPhysics` — HP1 native `0xf83` (3971)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Advance the receiving actor's physics for the supplied
  delta time, using the same physics-mode update as its ordinary autonomous
  tick. SurrealEngine's native calls `TickPhysics(DeltaSeconds)`
  ([`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L183)).
- **Implementation seam:** Call the runtime's shared actor physics tick while
  guarding against double advancement in the same host tick.

### `Pawn.CheckValidSkinPackage` — named native

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Validate that a requested skin package and mesh name
  form an allowed/compatible player skin selection, returning a boolean.
- **Reference limit:** SurrealEngine confirms the parameters but itself returns
  false and logs unimplemented
  ([`NPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPawn.cpp#L85)).
  Recover HP1's package naming and mesh compatibility checks from original
  behavior and shipped skin metadata.
- **Implementation seam:** Resolve through the read-only package store; do not
  treat validation as permission to load arbitrary filesystem paths.

### `PlayerPawn.ClientTravel` — named native

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Request client travel to the supplied URL, honoring the
  travel type and whether inventory should transfer. SurrealEngine forwards the
  request to the engine travel coordinator
  ([`NPlayerPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPlayerPawn.cpp#L38)).
- **Implementation seam:** Emit a host-level travel request through the current
  game/runtime action boundary; the script runtime must not open the next map
  directly.

### `PlayerPawn.GetPlayerNetworkAddress` — named native

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Required behavior:** Return the current player's network address string.
- **Reference limit:** SurrealEngine also leaves this unimplemented and returns
  an empty string
  ([`NPlayerPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPlayerPawn.cpp#L84)).
  For OpenHP1's local-only host, an explicitly specified empty/local result may
  be correct, but it must be an intentional networking policy rather than a
  failed native call.
- **Implementation seam:** Query the game host/network layer when one exists.

### `Pawn.StopWaiting` — named native

- **Status:** Implemented for audited shipped-level use.
- **Static-reachable levels:** All maps except `Entry`, `startup`,
  `Lev2_RemChase`, and `Lev5_Chess` (37 maps).
- **Required behavior:** End the pawn's current waiting/sleep interval by
  setting its remaining sleep time to zero
  ([`NPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPawn.cpp#L240)).
- **Implementation seam:** Clear the existing latent sleep/wait action on the
  receiving pawn without discarding unrelated persistent state-frame locals.

### `NameToString` — conversion opcode `0x57`

- **Status:** Implemented for audited shipped-level use.
- **Replay-observed level:** `Lev4_Sneak`.
- **Observed path:** `QuidPlayer.SetInitialState` calls `GotoState` (`0x071`);
  evaluation of an argument reaches `NameToString` with a package name index and
  fails with `expected supported conversion input value, found name`.
- **Original implementation:** The conversion only accepts `Value::NameText`
  ([`frame/operations.rs`](../crates/openhp1-runtime/src/frame/operations.rs#L397)),
  while package-backed name lookup already exists in `runtime_name`
  ([`world/native/support.rs`](../crates/openhp1-runtime/src/world/native/support.rs#L159)).
- **Required behavior:** Resolve a numeric `Name` against the bytecode's source
  package name table and return its text. Keep the pure `NameText` conversion.
- **Implementation seam:** Route numeric name conversion through the hosted
  frame/runtime boundary, as `ObjectToString` already does for object identity;
  do not stringify the integer index.

### `PatrolPoint.PreBeginPlay` execution does not terminate

- **Status:** Implemented for audited shipped-level use.
- **Replay-observed level:** `Lev3_Troll`.
- **Observed path:** Two `PatrolPoint0.PreBeginPlay` calls exceeded 100,000
  instructions. The prebound-call deferral then let initialization continue.
- **Original implementation:** Frames default to the 100,000-instruction safety limit
  ([`frame.rs`](../crates/openhp1-runtime/src/frame.rs#L197)); every decoded
  opcode counts toward it
  ([`frame/execute.rs`](../crates/openhp1-runtime/src/frame/execute.rs#L934)).
- **Original investigation:** Trace the final repeating instruction range and actor
  fields, then compare the actual `PatrolPoint.PreBeginPlay` branch and call
  order with HP1. Likely categories include an incorrect comparison, iterator,
  property default, or context result. Do **not** raise or remove the limit;
  ordinary `PreBeginPlay` must terminate.

### Nested `HPSounds` import resolution

- **Status:** Implemented for audited shipped-level use.
- **Replay-observed level:** `Lev3_Troll`.
- **Observed path:** `CutScene0.Tick` attempted to resolve Sound path
  `Hub5_sfx.Hub3_sfx.Vold_Pillar_Thump_06` in package `HPSounds`; no matching
  export was found, so the sound call was deferred.
- **Original implementation:** Imported object resolution reconstructs every outer group and
  requires an exact class/object/group match
  ([`resolver.rs`](../crates/openhp1-package/src/resolver.rs#L147)).
- **Original investigation:** Inspect the exact import outer chain in the calling
  package, the candidate `HPSounds` exports, and HP1's resolution result. The
  repeated `Hub5_sfx.Hub3_sfx` groups may be authored indirection, a resolver
  reconstruction error, or malformed content tolerated by the original engine.
  Do not fix it by blindly dropping a group.

### HPHud initialization replay blocker

- **Status:** Implemented for audited shipped-level use.
- **Blocked before timed replay:** `Lev2_Quid1`, `Lev2_RemChase`, `Lev3_Quid2`,
  `Lev5_FlyKeys`, `Lev_Tut2`, `Quid_HuffleA`, `Quid_HuffleB`, `Quid_HuffleC`,
  `Quid_RavenA`, `Quid_RavenB`, `Quid_RavenC`, `Quid_SlythA`, `Quid_SlythB`,
  `Quid_SlythC`.
- **Observed path:** `initialize_player_hud` returned no `SpawnActor` action for
  class `HPHud`, violating a hard assertion in `runtime_scan`
  ([`runtime_scan.rs`](../crates/openhp1-scene/examples/runtime_scan.rs#L154)).
  The runtime intentionally returns no action when `myHUD` already exists or
  `HUDType` is absent
  ([`world/actor/player.rs`](../crates/openhp1-runtime/src/world/actor/player.rs#L17)).
- **Resolution:** The scanner/runtime path now accepts the configured HUD
  subclass instead of requiring an exact `HPHud` class. Once initialization
  advanced, the shipped `BroomHarry.PlayerInput` bytecode exposed a serialized
  `ByteToInt` applied to a boolean field; the shared conversion now maps
  `false`/`true` to `0`/`1` (`01b9fa7`). The next reached call exposed the
  reversed `ModifySound` argument decoder, corrected in `148f603`.
- **Original investigation:** Record the selected player class, `myHUD`, and
  `HUDType` for each blocked map, then compare the game host's lazy `PreRender`
  setup. If the state is valid, relax the scanner assertion; if not, repair the
  shared player/HUD initialization path. Until then, these maps have only
  initialization coverage, not a 120-second neutral replay.
- **Verification:** All 14 formerly blocked maps now complete the full
  120-second neutral replay without a deferred or error diagnostic.

### Serialized `DamageType` and `ReducedDamageType` names

- **Status:** Implemented for audited shipped-level use.
- **Replay-observed levels:** All non-Entry maps.
- **Observed path:** Zone physics read the shipped `DamageType=None` default and
  emitted `Physics: actor property DamageType is Name("None")`. The same shared
  reader is used for pawn `ReducedDamageType` when evaluating pain-zone damage
  compatibility.
- **Required behavior:** Accept serialized `NameProperty` values as names,
  including the `None` sentinel, alongside script-produced text and numeric
  name values. Preserve the authored damage-type comparison used to decide
  whether a pawn may traverse a pain zone.
- **Original failure:** Serialized defaults are stored as
  `StoredValue::Name(String)`, while `optional_actor_name` accepted only
  `Value::NameText`, package-indexed `Value::Name`, and `Value::None`.
- **Implementation seam:** Decode every accepted representation in the shared
  `optional_actor_name` helper used by zone and pawn movement; do not special
  case `DamageType` at individual callers.
- **Verification:** A synthetic zone-physics regression uses the actual
  `StoredValue::Name("None")` representation. After `808d4a4`, all 40 affected
  maps complete the 120-second replay without the deferred physics diagnostic.

### Non-billboard particle `RenderPrimitive`

- **Status:** Implemented for audited shipped-level use.
- **Initialized-level use:** `Lev2_Fire2`, `Lev2_HogFront`, `Lev2_HogFront_2`,
  `Lev2_HogFront_3`, `Lev2_Inc_A`, `Lev2_Inc_B`, `Lev2_fire1`, `Lev_Tut2`.
- **Original implementation:** Every non-1 value produces `particle render primitive is not a
  billboard`
  ([`world/action.rs`](../crates/openhp1-runtime/src/world/action.rs#L87)).
- **Required behavior:** Decode the authored `RenderPrimitive` enum value and
  render the corresponding particle primitive rather than forcing a camera
  billboard.
- **Original uncertainty:** The local SurrealEngine clone does not implement HP1's
  `ParticleFX`. Resolve the enum names and geometry rules from the shipped
  `ParticleFX` metadata and original `AParticleFX`/`UParticle` behavior before
  choosing a scene representation.

### Particle collision `Elasticity`

- **Status:** Implemented for audited shipped-level use.
- **Initialized-level use:** `Lev2_Fire2`, `Lev2_fire1`.
- **Original implementation:** Any nonzero value produces `particle collision elasticity is
  unsupported`
  ([`world/action.rs`](../crates/openhp1-runtime/src/world/action.rs#L99)).
- **Required behavior:** Detect the particle's authored world collision and
  reflect or damp its velocity using `Elasticity` according to HP1's update
  order.
- **Implementation seam:** Reuse the existing BSP collision query and particle
  integrator. First recover whether HP1 sweeps the particle, uses a point trace,
  and how it applies elasticity to normal and tangential velocity; SurrealEngine
  has no HP1 particle reference for these details.

### Particle `WindModifier`

- **Status:** Implemented for audited shipped-level use.
- **Initialized-level use:** `Lev3_Dungeon`, `Lev3_DungeonB`, `Lev3_Quid2`,
  `Lev5_fluffy`.
- **Original implementation:** Any nonzero value produces `particle wind response is
  unsupported`
  ([`world/action.rs`](../crates/openhp1-runtime/src/world/action.rs#L102)).
- **Required behavior:** Apply the active zone's authored wind/velocity to the
  particle using `WindModifier` at the correct point in integration.
- **Implementation seam:** Extend the emitter's existing zone-physics lookup;
  do not make the scene resolve zones. Recover the exact formula and whether the
  zone is sampled at the emitter or particle from original HP1 native behavior.

### Particle `bVelocityRelative`

- **Status:** Implemented for audited shipped-level use.
- **Initialized-level use:** `Lev4_Sneak`.
- **Original implementation:** A true value produces `particle owner-velocity inheritance is
  unsupported`
  ([`world/action.rs`](../crates/openhp1-runtime/src/world/action.rs#L96)).
- **Required behavior:** Incorporate the owning/emitting actor's movement into
  particle velocity according to HP1's `bVelocityRelative` semantics.
- **Original uncertainty:** Determine from original HP1 whether this is an initial spawn
  impulse, a per-tick relative frame, or both. `bSystemRelative` already owns
  position attachment and must remain a separate behavior.

## Source-audited partial implementations

These paths return a plausible value or perform only part of the engine-side
work, so an "unsupported" error scan alone cannot find them. Their level lists
come from the same corpus pass as the hard failures above.

### `MakeNoise` — native `0x200` (512)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** The call succeeds silently, so the neutral replay
  cannot distinguish it from a complete implementation.
- **Original implementation:** [`world/native.rs`](../crates/openhp1-runtime/src/world/native.rs#L472)
  explicitly defers pawn noise slots and `HearNoise` dispatch.
- **Required behavior:** Resolve the actor's pawn `Instigator`, suppress recent
  nearby duplicate noises, update one of the pawn's two timestamped
  location/loudness slots, walk `Level.PawnList`, apply `CanHearNoise`, and call
  `HearNoise(loudness, source)` for qualifying pawns. SurrealEngine implements
  this sequence in
  [`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L2497).
- **Implementation seam:** Keep the state in the existing actor instance and
  route the event through the normal event dispatcher. Do not add a parallel AI
  notification system.

### `IsAnimating` root-bone overload — native `0x11a` (282)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** The ignored optional argument produces no diagnostic.
- **Original implementation:** [`world/native.rs`](../crates/openhp1-runtime/src/world/native.rs#L320)
- **Required behavior:** The no-argument form reports whether actor animation is
  active. The HP1 overload can ask about a root-bone animation channel, which
  requires channel-aware animation state before the argument can affect the
  result.
- **Original uncertainty:** SurrealEngine also ignores the HP1 `RootBone` argument and
  logs it as unimplemented
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1928)).
  Recover exact per-channel semantics from original HP1 behavior before
  implementing this overload.

### `SaveConfig` — native `0x218` (536)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** The no-op produces no diagnostic.
- **Original implementation:** [`world/native.rs`](../crates/openhp1-runtime/src/world/native.rs#L1081)
- **Required behavior:** Persist instance properties marked `config` or
  `globalconfig` to the appropriate INI configuration. SurrealEngine routes the
  native through `UObject::SaveConfig`
  ([`NObject.cpp`](../../SurrealEngine/SurrealEngine/Native/NObject.cpp#L1430))
  and serializes the marked properties
  ([`UObject.cpp`](../../SurrealEngine/SurrealEngine/UObject/UObject.cpp#L244)).
- **Implementation seam:** Extend the existing configuration/package path only
  when OpenHP1 has an explicit writable settings policy. The runtime and package
  loaders otherwise remain read-only.

### `ConsoleCommand` named-native overloads

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** The conservative named-call intersection covers all
  non-Entry maps. It does not prove that every call resolves to every overload.
- **Replay observation:** The empty result produces no diagnostic.
- **Original implementation:** [`world/execution.rs`](../crates/openhp1-runtime/src/world/execution.rs#L517)
- **Required behavior:** Execute a console command in the calling actor's
  context and return the command's string result. See SurrealEngine's player
  and actor forms in
  [`NPlayerPawn.cpp`](../../SurrealEngine/SurrealEngine/Native/NPlayerPawn.cpp#L45)
  and [`NActor.cpp`](../../SurrealEngine/SurrealEngine/Native/NActor.cpp#L198).
- **Implementation seam:** Route through the eventual game-console command
  surface. Do not create a runtime-only command registry merely to remove this
  stub.

### `TraceTexture` — native `0x11d` (285)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** The null result produces no diagnostic.
- **Original implementation:** [`world/native/support.rs`](../crates/openhp1-runtime/src/world/native/support.rs#L92)
- **Required behavior:** Trace the requested segment and return the hit BSP
  surface's texture, honoring the native's flags and optional decal behavior.
- **Implementation seam:** Retain material/texture identity on the existing BSP
  collision result, then resolve that identity through the runtime object-handle
  path. Do not duplicate collision traversal.

### `SetLocation` placement semantics — native `0x10b` (267)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** The replay reached `SetLocation`, but success does not
  reveal whether any request needed the missing rejection/notification path.
- **Original implementation:** [`world/native.rs`](../crates/openhp1-runtime/src/world/native.rs#L901)
- **Required behavior:** Reject a blocked placement, return `false`, otherwise
  update collision/light registration and dispatch the resulting `Touch` and
  `UnTouch` notifications. SurrealEngine's reference path is
  [`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1366).
- **Original uncertainty:** SurrealEngine's `CheckLocation` includes a nearby-placement
  search with its own approximation. Compare HP1 at blocked and overlapping
  destinations before treating it as exact engine behavior.

### `SetRotation` placement semantics — native `0x12b` (299)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** The replay reached `SetRotation`, but success does not
  reveal whether any request needed blocked-rotation rejection.
- **Original implementation:** [`world/native.rs`](../crates/openhp1-runtime/src/world/native.rs#L959)
- **Required behavior:** Apply the requested rotation only when the actor still
  fits, and rotate/move based actors with their base.
- **Original uncertainty:** SurrealEngine also leaves the collision rejection as a TODO
  while implementing based-actor rotation
  ([`UActor.cpp`](../../SurrealEngine/SurrealEngine/UObject/UActor.cpp#L1411)).
  Differential HP1 replay is required.

### `Spawn` world placement — native `0x116` (278)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All maps.
- **Replay observation:** Spawn succeeds or fails without identifying whether
  the missing BSP placement/search path would change the result.
- **Original implementation:** [`world/movement.rs`](../crates/openhp1-runtime/src/world/movement.rs#L271)
- **Required behavior:** For collision-enabled actors, reject an invalid world
  location before allocating the actor identity; preserve UE1's nearby valid
  placement behavior if HP1 uses it.
- **Implementation seam:** Add BSP placement to the existing
  `spawn_location_is_clear` path and reuse collision geometry already owned by
  the runtime.

### `SetBase` standing-count bookkeeping — native `0x12a` (298)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** `StandingCount` is not consumed by the scan, so the
  missing bookkeeping produces no diagnostic.
- **Original implementation:** [`world/movement.rs`](../crates/openhp1-runtime/src/world/movement.rs#L560)
- **Required behavior:** Update the old and new bases' direct-standing actor
  counts as the base relationship changes.
- **Implementation seam:** Derive this from the existing compact based-actor
  index instead of maintaining a second relationship graph.

### Looping `SetTimer` catch-up — native `0x118` (280)

- **Status:** Implemented for audited shipped-level use.
- **Static reachability:** All non-Entry maps.
- **Replay observation:** The replay records timer callbacks but does not reveal
  callbacks coalesced inside a frame.
- **Original implementation:** [`world/actor/tick.rs`](../crates/openhp1-runtime/src/world/actor/tick.rs#L873)
- **Required behavior:** Dispatch the number of elapsed callbacks if sub-frame
  timer fidelity is observable, while retaining the remainder for the next
  tick.
- **Implementation seam:** Extend the existing timer advancement result to a
  count; do not introduce a scheduler.

### Final-function failure deferral

- **Status:** Implemented for audited shipped-level use.
- **Replay-relied-on levels:** `Lev2_Fire2`, `Lev2_HogFront`, `Lev2_Inc_A`,
  `Lev2_Inc_B`, `Lev2_fire1`, `Lev3_Dungeon`, `Lev3_DungeonB`, `Lev3_Intro`,
  `Lev3_Lumos`, `Lev3_Troll`, `Lev4_Sneak`, `Lev4_Sneak2`, `Lev5_Chess`,
  `Lev5_Final`, `Lev5_Snare`, `Lev5_fluffy`, `Lev_Tut1b`, `Lev_Tut3`,
  `Lev_Tut3b`.
- **Original implementation:** [`world/execution/dispatch.rs`](../crates/openhp1-runtime/src/world/execution/dispatch.rs#L49)
- **Required behavior:** Once the underlying VM/native gaps are implemented,
  propagate real script failures instead of converting them into successful
  calls. Remove the deferral only after the corpus executes without relying on
  it.

## Decoder discrepancy: byte `0x60`

OpenHP1 currently maps `0x39..=0x60` to conversion opcodes
([`opcode.rs`](../crates/openhp1-runtime/src/opcode.rs#L106)). The reference
format uses `0x60` as `ExtendedNative60`: it consumes a low byte, forms native
index `0x00xx`, and then reads parameters through `EndFunctionParms`
([`UClass.h`](../../SurrealEngine/SurrealEngine/UObject/UClass.h#L326),
[`Bytecode.cpp`](../../SurrealEngine/SurrealEngine/VM/Bytecode.cpp#L36)).

This is a decoder bug, not an unknown conversion. If the corpus contains byte
`0x60`, fix bytecode decoding and runtime opcode dispatch together; treating it
as `ConversionOpcode::Unsupported` consumes the following bytes with the wrong
layout.

The full shipped script corpus contains no byte `0x60`, and the level-class scan
therefore found no reachable use. It is recorded here because it directly
affects the accuracy of future opcode scans, not as a currently used level gap.

## Audited non-gaps and zero-use diagnostics

- `SetCollision` is statically reachable in every map, but its three flags are
  stored and the cached collision actor is refreshed. The old source comment
  about waiting for BSP movement is not evidence of a current gap.
- `Texture`, `bUnlit`, and `bMeshEnviroMap` assignment paths are reachable in
  all non-Entry maps, but the shipped writes observed by the existing
  provenance scan restore the same effective defaults. There is no proven
  level transition to implement. `MultiSkins` is only reachable through generic
  network skin code, likewise without a proven shipped transition. See
  [`runtime-capability-provenance.md`](runtime-capability-provenance.md#runtime-render-properties).
- Particle random texture selection and `ColorPalette` are detected by
  `ParticleEmitter::capability_diagnostics`, but no initialized shipped level
  authored an effective value requiring them. Dynamically authored
  `ParentBlend` is implemented from the original native class-default blending
  path; see
  [`spell-learning-original-behavior.md`](spell-learning-original-behavior.md#parentblend-means-superclass-defaults-not-actor-ownership).
- `Decal.DetachDecal`, the other reserved/unsupported VM tokens, and
  `PHYS_Spider` had no statically reachable or replay-observed shipped-level use
  in this pass. They remain broader engine coverage gaps, not entries in this
  level ledger.

## Updating this ledger

For future scans:

1. Run the static map-reachability scan across all 41 files and record every
   unsupported opcode, numeric native, named native, and known partial path.
2. Run `runtime_scan` in release mode for each map and merge replay diagnostics
   without replacing static evidence.
3. Add newly observed levels to the existing feature section; do not duplicate
   sections for the same opcode or native index.
4. After implementing a feature, move its durable semantics to `runtime.md` and
   remove it from this gap ledger only after the static and replay scans are
   clean for that feature.
