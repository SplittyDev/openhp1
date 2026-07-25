# Open Harry Potter 1

> An open-source re-implementation of the first Harry Potter game.

OpenHP1 requires the original game files to run.

The first development milestone can already parse and display untextured BSP
world geometry from the original maps:

```sh
cargo run -p openhp1-viewer -- res/Maps/Quid_RavenA.unr
```

Drag over the viewport to look around. Use `WASD` to move, `Q`/`E` to move
down/up, and hold Shift to move faster.

See [`docs/`](docs/) for the reverse-engineered package, map, texture, and
rendering notes. Original game data belongs in the gitignored `res/` directory
and must not be committed.
