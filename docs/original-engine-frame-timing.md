# Original engine frame, physics, and presentation timing

## Question and result

This note asks how the shipped HP1 PC engine schedules game logic, physics,
camera calculation, rendering, and presentation. The short answer is:

- Retail HP1 does **not** have a global fixed 60 Hz physics step.
- A standalone game does **not** receive a 60 Hz cap from
  `UGameEngine::GetMaxTickRate`; that function returns zero in this state.
- Each engine frame supplies a variable delta to `ULevel::Tick`. The level
  clamps it to `0.005 .. 0.1` seconds, applies `TimeDilation`, and ticks actors.
- An actor's physics mode is dispatched once per actor tick with that delta.
  Collision retry/time slicing is implemented inside individual physics modes
  and is adaptive; it is not a universal `0.02` second subdivision.
- The Windows client repaints from the same engine tick, and the D3D renderer's
  fullscreen `Flip` waits for vertical synchronization by default. That
  presentation wait, rather than an engine-side standalone tick cap, normally
  paces simulation and rendering together.
- `UGameEngine::Draw` calculates the camera from current state. No previous
  camera sample or render interpolation alpha was found.

Therefore the original is a variable-step, render-coupled design. A manual
60 Hz no-vsync cap can be a useful compatibility policy, but it is not the
retail scheduling mechanism. Decoupling a 60 Hz simulation from faster
interpolated rendering would be a modern extension, not recovered retail
behavior.

## Scope and primary artifacts

The primary evidence is the legally obtained installation under `res/`:

| Artifact | SHA-256 |
| --- | --- |
| `res/System/Engine.dll` | `7756a2a3df7198d72f4706952196bee8adb3b79edfe7c8b3a5e4d2e3593d8ebc` |
| `res/System/D3DDrv.dll` | `7683b11647dafe3926eff7d0d055abbe3d728648a19f5f8a613fd03efd151599` |
| `res/System/WinDrv.dll` | `cca53d5eac40ffea2ee2e041e249624c05929dd478475b9346e27ce75fb21c57` |
| `res/System/HP.exe` | `c026e7579db229263abf350d488d5beae6e9d1b4242b1897046df0194a80de3d` |
| `res/Ghidra_Engine.c` | `6b761322d45a1869d01d1993997588de58de6df453258a175d6ae93a4e353b0b` |

The Ghidra C export has useful control-flow and type recovery, but some of its
generated internal function labels are displaced from the current DLL. For
example, its `UGameEngine::Tick` wrapper points at `FUN_103a05e0`, while the
current PE's export thunk reaches `0x103a0900`. Addresses below are therefore
from disassembly of the hashed DLL; Ghidra C line numbers are supporting
landmarks rather than address authority.

## Confirmed original-engine evidence

### One variable level tick, bounded but not fixed

`ULevel::Tick` is export ordinal 2010, thunk `0x103010a5`, body
`0x103b6db0`. At `0x103b7062 .. 0x103b70a2`, it clamps the incoming delta to
at least `0.005f` and at most `0.1f`, then multiplies it by the active
`LevelInfo.TimeDilation`. The same logic appears in
`res/Ghidra_Engine.c:265917-265925`.

This has two important consequences:

- Below 200 real frames per second, the level uses the measured frame delta
  (subject to time dilation), not `1/60`.
- A genuinely unbounded loop above 200 Hz advances the level by at least 5 ms
  per iteration and can therefore run game time too fast. A frame stall longer
  than 100 ms advances game time by no more than 100 ms in that tick.

`UGameEngine::Tick` is export ordinal 2009, thunk `0x10303a12`, body
`0x103a0900`. Its active-level calls at `0x103a0b00 .. 0x103a0b54` pass the
host delta into `ULevel::Tick`. No fixed-step accumulator is visible on this
path.

### `GetMaxTickRate` does not cap standalone play

`UGameEngine::GetMaxTickRate` is export ordinal 1267, thunk `0x103017ad`, body
`0x103a06d0` (`res/Ghidra_Engine.c:23569-23640`). It returns a bounded server
rate in network-server states, or a connection-derived rate for a network
client. The server rate is clamped to `10 .. 120`; the shipped TCP driver
configures `NetServerMaxTickRate=20` and `LanServerMaxTickRate=35` at
`res/System/1/Default.ini:163-175`. The demo driver separately specifies 60 at
lines 185-197.

Ordinary standalone execution falls through to the zero float loaded at
`0x103a086f`. Base `UEngine::GetMaxTickRate`, thunk `0x10304287`, body
`0x10393170` (`res/Ghidra_Engine.c:129915-129922`), likewise returns zero
outside editor mode; its editor case returns 30.

This sharpens the older wording in
`docs/broom-physics-original-behavior.md`: the function can be an engine rate
limit in network/editor states, but it does **not** establish a standalone
retail frame cap. The later finding in
`docs/lev3-dungeon-small-gridmover-repeat-push.md` that standalone returns zero
is the precise interpretation.

`MinDesiredFrameRate=30` in `res/System/1/Default.ini:70-97` is not evidence of
a 30 Hz cap. It is a client detail/performance target and is not returned by
the standalone `GetMaxTickRate` path above.

### Physics is dispatched once; subdivision belongs to each mode

`AActor::Tick`, thunk `0x10304205`, body `0x103b3840`, calls the actor's
virtual `performPhysics` once with the level tick delta at
`0x103b4331 .. 0x103b434c`.

`AActor::performPhysics`, body `0x103e52c0`, dispatches its active physics mode
once with that delta. `APawn::performPhysics`, thunk `0x103028f1`, body
`0x103e5520` (`res/Ghidra_Engine.c:67916-67966`), does the same for pawn modes
and then performs the pawn's rotation update. There is no outer loop that
repeatedly invokes `performPhysics` in `0.02` second slices.

There is consequently no retail “pawn exception” to a universal 20 ms actor
step: the universal step itself does not exist. OpenHP1's current logic at
`crates/openhp1-runtime/src/world/physics.rs:13,266-270`—whole delta for pawns,
but repeated `0.02` slices for other actors—is not the structure of the shipped
engine. The pawn branch happens to preserve one outer dispatch, but faithful
pawn behavior still depends on implementing each native mode's internal
collision iteration.

The native inner behavior is mode-specific:

- `APawn::physWalking`, thunk `0x10304480`, body `0x103e6b60`
  (`res/Ghidra_Engine.c:135243-135314`), permits at most eight collision
  iterations. Under its movement/player conditions it chooses at most 50 ms,
  often half of the remaining time; otherwise it may consume the full
  remainder. This is adaptive, not a fixed 20 ms step.
- `AActor::physFalling`, thunk `0x10302f6d`, body `0x103eea20`
  (`res/Ghidra_Engine.c:84255-84288`), also permits at most eight iterations
  and uses adaptive slices capped at 100 ms when the remainder is large.
- `AActor::physMovingBrush`, body `0x104061f0`, receives the whole actor delta
  and manages its own remaining-time, collision, and completion behavior. The
  caller at `AActor::performPhysics` does not pre-split it. This is the relevant
  retail behavior for movers that can stick when outer `performPhysics`
  semantics are repeated artificially.
- `APawn::physFlying`, body `0x103f13a0`, begins from `Velocity * DeltaTime`
  and performs its own collision slide/`TwoWallAdjust` handling; it has no
  generic 20 ms wrapper.

Searching the hashed `Engine.dll` for the little-endian IEEE-754 bytes of
`0.02f` (`0a d7 a3 3c`) finds no match. This supports, but is not needed for,
the positive control-flow evidence above.

### Tick, draw, and presentation remain coupled

After level processing, `UGameEngine::Tick` calls the Windows client tick at
`0x103a1962 .. 0x103a1967`. `UWindowsClient::Tick` is export thunk
`0x11101073`, body `0x11102e00`; it selects the active viewport and calls its
repaint function at `0x11102f3a .. 0x11102f44`. `UWindowsViewport::Repaint` is
export thunk `0x111010f5`, body `0x11108a50`, and invokes the engine's `Draw`
virtual at `0x11108a72 .. 0x11108a8b`.

`UGameEngine::Draw` is export thunk `0x10303dfa`, body `0x1039fa40`
(`res/Ghidra_Engine.c:117589-117793`). At
`0x1039fab1 .. 0x1039fb07`, it copies the pawn's current transform, calls
`PlayerCalcView`, and sends that result to rendering. The function has neither
a render interpolation alpha nor a previous camera sample. This confirms a
current-state camera, not an independently interpolated presentation camera.

Fullscreen D3D presentation occurs in
`UD3DRenderDevice::Unlock`, export thunk `0x100010cd`, body `0x100038b0`. At
`0x10003946 .. 0x10003961`, it calls the DirectDraw surface's `Flip` with flags
`1 | (UseVSync ? 8 : 0)`: `DDFLIP_WAIT` is always present and
`DDFLIP_NOVSYNC` is added only when the config field named `UseVSync` is true.
The name is counterintuitive, but the passed flags are decisive.

`UD3DRenderDevice::StaticConstructor`, export thunk `0x1000101e`, body
`0x100019b0`, registers `UseVSync` at object offset `0x9cc` and gives it no
nonzero class default. No shipped INI under `res/System` sets `UseVSync`.
Thus its normal zero value produces a synchronized, waiting flip. The shipped
D3D section also enables page flipping and triple buffering at
`res/System/1/Default.ini:255-272`.

`RefreshRate=60Hz` at `res/System/1/Default.ini:226-237` belongs only to the
Glide renderer. It does not prove that the D3D path forces a 60 Hz display
mode. The D3D evidence proves synchronized presentation, not a hard-coded
refresh rate.

## Licensed reference-engine evidence, not retail evidence

The local SurrealEngine clone is useful only as a comparison after the shipped
artifacts:

- `/Users/splitty/Developer/SurrealEngine/SurrealEngine/UObject/UActor.cpp:432-471`
  implements a generic 20 ms `TickPhysics` loop for all actors. It has no pawn
  exception. This directly differs from the HP1 `Engine.dll` call graph and
  must not override the retail evidence. It is a plausible source for the
  generic `0.02` design currently present in OpenHP1.
- `/Users/splitty/Developer/SurrealEngine/SurrealEngine/Engine.cpp:124-200`
  calculates elapsed time, ticks levels, calculates the view, and renders in
  one variable-step loop. Its clock clamp at lines 990-1000 is analogous in
  architecture, but its constants and implementation are not proof about HP1.

## Implications for OpenHP1

### Strict retail scheduling

The closest recovered model is one variable simulation tick per rendered
frame, a level delta clamped to `5 .. 100` ms, mode-local collision iteration,
and synchronized presentation. On a stable 60 Hz display this naturally tends
toward one approximately 16.7 ms game tick and one draw per refresh, without
`GetMaxTickRate` returning 60.

The original DirectDraw waiting flip does not imply that a modern wgpu FIFO
queue will have identical latency or missed-vblank behavior. The observed
modern fall to approximately 30 frames per second is therefore not evidence
that retail intentionally ran game logic at 30 Hz.

### Smooth rendering beyond the retail model

An uncapped present loop cannot safely drive the recovered level tick directly:
the retail 5 ms floor makes sufficiently fast loops advance game time too
quickly, and OpenHP1's generic outer 20 ms splitting is not the retail physics
solution.

A fixed/accumulated 60 Hz game simulation plus an independently rendered,
interpolated presentation can provide smoother camera motion on higher-refresh
displays. That design should be labelled an intentional modern extension. To
avoid relative jitter, interpolation would need coherent previous/current
presentation transforms for the camera and moving scene objects, rather than
interpolating only the camera. The retail engine itself supplies no such alpha
or history.

For a parity-first implementation, the native correction is not “substep every
non-pawn actor at 0.02.” It is one `performPhysics` dispatch per actor tick plus
the collision retry/time-slicing rules of each physics mode.

## Unresolved points and hypotheses

- The exact host clock calculation that produces the delta passed into
  `UGameEngine::Tick` has not been established from `HP.exe`. The DLL evidence
  begins at the received delta and establishes its downstream treatment.
- The shipped D3D flags establish a vblank-waiting fullscreen flip, page
  flipping, and configured triple buffering. They do not by themselves prove
  the exact queue depth, latency, display refresh selected on every machine, or
  how a missed vblank behaved on each historical DirectDraw driver.
- It is a strong architectural inference that stable synchronized presentation
  is why ordinary retail play commonly approached the display refresh while
  preserving physics. This is not evidence of a hidden fixed 60 Hz simulator.

## Reproduction commands

Run from the repository root:

```sh
shasum -a 256 res/Ghidra_Engine.c res/System/Engine.dll \
  res/System/D3DDrv.dll res/System/WinDrv.dll res/System/HP.exe

/opt/homebrew/opt/llvm/bin/llvm-readobj --coff-exports res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump -d --no-show-raw-insn \
  --start-address=0x103a06d0 --stop-address=0x103a0890 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump -d --no-show-raw-insn \
  --start-address=0x103b6db0 --stop-address=0x103b7225 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump -d --no-show-raw-insn \
  --start-address=0x103e52c0 --stop-address=0x103e56f0 res/System/Engine.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump -d --no-show-raw-insn \
  --start-address=0x1039fa40 --stop-address=0x103a0040 res/System/Engine.dll

/opt/homebrew/opt/llvm/bin/llvm-readobj --coff-exports res/System/WinDrv.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump -d --no-show-raw-insn \
  --start-address=0x11102e00 --stop-address=0x11103000 res/System/WinDrv.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump -d --no-show-raw-insn \
  --start-address=0x11108a50 --stop-address=0x11108ab0 res/System/WinDrv.dll

/opt/homebrew/opt/llvm/bin/llvm-readobj --coff-exports res/System/D3DDrv.dll
/opt/homebrew/opt/llvm/bin/llvm-objdump -d --no-show-raw-insn \
  --start-address=0x100038b0 --stop-address=0x10003980 res/System/D3DDrv.dll

nl -ba res/Ghidra_Engine.c | sed -n \
  '23569,23640p;67916,67966p;84255,84288p;117589,117793p;129915,129922p;135243,135314p;265917,265925p'
nl -ba res/System/1/Default.ini | sed -n '70,97p;163,197p;226,237p;255,272p'
rg -n -i 'UseVSync|RefreshRate|MinDesiredFrameRate|NetServerMaxTickRate|LanServerMaxTickRate' \
  res/System --glob '*.ini'

perl -0777 -ne 'print "0.02f found at ", pos($_)-4, "\n" while /\x0a\xd7\xa3\x3c/g' \
  res/System/Engine.dll
```
