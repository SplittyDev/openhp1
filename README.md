# Open Harry Potter 1

> An open-source re-implementation of the first Harry Potter game.

OpenHP1 requires the original game files to run.

The first development milestone can parse and display textured BSP world
geometry and visible vertex- and skeletal-mesh actors from the original maps,
including masked, translucent, and modulated surfaces, reconstructed static
lightmaps, sky zones, animated water, and mesh animation playback. Experimental
runtime support dispatches actor lifecycle, tick, timer, and persistent state
code, including `GotoState`, labels, `Sleep`, `PlayAnim`, `LoopAnim`, and
`FinishAnim`:

```sh
cargo run -p openhp1-viewer -- res/Maps/Quid_RavenA.unr
```

Drag over the viewport to look around. Use `WASD` to move, `Q`/`E` to move
down/up, and hold Shift to move faster. The Actors panel searches the current
level's actors and shows their identity, decoded state, render ranges, and
diagnostics.

See [`docs/`](docs/) for the reverse-engineered package, map, texture, and
rendering notes. Original game data belongs in the gitignored `res/` directory
and must not be committed.
