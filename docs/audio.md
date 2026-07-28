# Audio

`openhp1-audio` owns typed decoding and playback of package-backed `Sound` and
`Music` exports. The package crate remains responsible only for the shared
container and bounds-checked object reader; runtime actions carry decoded audio
without exposing package offsets to the game loop.

Both export classes serialize tagged properties followed by a format name, a
lazy-array offset in newer packages, a compact data size, and the embedded
audio bytes. HP1 `Sound` exports contain WAV or MPEG Layer II data; all 91
locally scanned `Music` packages also contain MPEG Layer II data.

Original game packages remain read-only and are never copied into public tests.
