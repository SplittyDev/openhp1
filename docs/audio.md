# Audio

`openhp1-audio` owns typed decoding and playback of package-backed `Sound` and
`Music` exports. The package crate remains responsible only for the shared
container and bounds-checked object reader; runtime actions carry decoded audio
without exposing package offsets to the game loop.

Kira owns device output, decoding, mixing, playback-rate changes, and the
spatial audio backend. Game and runtime code retain the original game's audio
policy rather than duplicating it in the renderer.

Both export classes serialize tagged properties followed by a format name, a
lazy-array offset in newer packages, a compact data size, and the embedded
audio bytes. HP1 `Sound` exports contain WAV or MPEG Layer II data; all 91
locally scanned `Music` packages also contain MPEG Layer II data.

Original game packages remain read-only and are never copied into public tests.

The game audio adapter consumes `PlaySound` runtime actions. Background music
follows the original `PlayerPawn.Song`, `SongSection`, and `Transition`
properties; the host only turns those authored state changes into playback.
Sound volume, pitch, radius, actor attachment, spatialization, and actor/slot
replacement follow the values supplied by the original runtime. Global music
and sound volume come from the audio subsystem selected by the original
`Default.ini`. Transition fading is not implemented yet.
