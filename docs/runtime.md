# UnrealScript runtime behavior

This document records durable runtime semantics that are easy to lose when
extending native calls, actor state, or animation actions.

## Actor identity and state

Actors use stable package/export identities. Class defaults followed by actor
tagged-property overrides initialize persistent instance state. Remote actor
contexts must resolve registered actor handles so field reads, writes, and
calls affect the target actor rather than a temporary copy.

Runtime actions update both persistent actor state and the corresponding scene
state. In particular, later animation ticks must not undo `SetLocation` or
other transform changes.

## State execution

Persistent state frames retain their decoded instruction pointer and local
values across latent `Sleep` and `FinishAnim` actions. `GotoState`,
`GotoLabel`, and `Stop` operate on that retained frame rather than restarting
the state body.

Label lookup uses the final top-level `LabelTable` in canonical decoded
bytecode. Serialized state metadata offsets are not canonical decoded-byte
offsets.

## Animation actions

- `PlayAnim` and `LoopAnim` use the scene's existing animation path.
- Animation completion occurs at `AnimLast`, before the sampler wraps toward
  frame zero.
- Repeated `LoopAnim` calls preserve the current phase.
- `FinishAnim` ends the current loop.
- Tween-time arguments blend from the displayed pose.

Unsupported actions should remain nonfatal actor diagnostics until their
subsystem exists; they must not silently claim successful behavior.
