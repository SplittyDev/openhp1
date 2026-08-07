# XeGTAO spatial and temporal stability audit

## Conclusion

OpenHP1 does **not** vary XeGTAO sample directions between frames. Its two
noise values depend only on the current screen pixel's position in a repeating
64-by-64 Hilbert/R2 pattern. The AO pass runs at the renderer's full internal
viewport resolution, uses Intel's Ultra preset of nine slices and three steps
per side (54 depth samples per pixel), and applies two edge-aware 3-by-3
spatial passes for an effective 5-by-5 footprint.

The likely source of noise that appears to move is therefore residual
**spatial** sampling error: as the camera or geometry moves, a world-space
surface point crosses pixels and receives different fixed screen-space sample
rotations and offsets. OpenHP1 has FXAA and SMAA 1x, but no temporal AA or AO
history to accumulate that error. Adding a frame counter to the current noise
would make the pattern flicker; Intel says to use frame-varying noise only with
TAA and to keep its noise index at zero otherwise.

## Reference expectations and current behavior

| Property | Intel XeGTAO reference | OpenHP1 now | Assessment |
| --- | --- | --- | --- |
| Sample variation | A 64-by-64 Hilbert index drives a two-dimensional R2 sequence. With TAA, the reference adds `288 * (frameIndex % 64)`; without TAA, `NoiseIndex` is zero. ([sampling-noise notes](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#sampling-noise), [`SpatioTemporalNoise`](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/vaGTAO.hlsl#L65-L83), [TAA-gated frame input](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Effects/vaGTAO.cpp#L283-L290), [`GTAOConstants::NoiseIndex`](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.h#L147-L175)) | `spatial_noise(pixel)` uses the same Hilbert/R2 constants, but has no frame input. The Rust uniform also has no frame index. ([shader](../crates/openhp1-render/src/shaders/modern/ao_xegtao.wgsl#L28), [noise function](../crates/openhp1-render/src/shaders/modern/ao_xegtao.wgsl#L106), [uniform](../crates/openhp1-render/src/renderer/modern/ao.rs#L38)) | Correct for a renderer without TAA. Directions vary by screen pixel, not by frame. |
| Resolution | XeGTAO defaults to full resolution. Intel's half-resolution suggestion requires a bilateral upsample and trades quality for speed. ([resolution and sampling](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#resolution-and-sampling), [FAQ](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#faq)) | Raw AO, filtered AO, packed edges, and mip-zero view depth are allocated at `AoRenderer::size`, which is the full renderer viewport. ([resource creation](../crates/openhp1-render/src/renderer/modern/ao.rs#L332), [texture size](../crates/openhp1-render/src/renderer/modern/ao.rs#L343), [viewport uniform](../crates/openhp1-render/src/renderer/modern/ao.rs#L263)) | Full internal resolution, not half resolution. The independently enlarged OS window is not the AO resolution. |
| Horizon work | High is 3 slices x 3 steps per side = 18 depth samples. Ultra is 9 x 3 = 54. ([quality entry points](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/vaGTAO.hlsl#L94-L123), [resolution and sampling](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#resolution-and-sampling)) | `SLICE_COUNT` is 9, `STEPS_PER_SLICE` is 3, and every step samples positive and negative offsets. ([constants and slice loop](../crates/openhp1-render/src/shaders/modern/ao_xegtao.wgsl#L12), [step loop and paired loads](../crates/openhp1-render/src/shaders/modern/ao_xegtao.wgsl#L57)) | Matches Ultra exactly. The preceding High setting was a plausible source of more visible residual variance, but the current path is not undersampled relative to Intel's highest preset. |
| Depth pyramid | Five levels, using a weighted average biased from the most distant sample; Intel chose this as more stable in motion than rotated-grid subsampling. The default depth-MIP offset is `3.30`. ([depth filtering and stability](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#memory-bandwidth-bottleneck), [source defaults](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.h#L93-L104)) | Up to five `R32Float` levels, the corresponding weighted filter, and offset `3.30`. ([formats and limit](../crates/openhp1-render/src/renderer/modern/ao.rs#L7), [downsample shader](../crates/openhp1-render/src/shaders/modern/ao_depth_downsample.wgsl), [offset](../crates/openhp1-render/src/shaders/modern/ao_xegtao.wgsl#L11)) | Matches the reference stability-oriented path. |
| Spatial denoise | The current XeGTAO design uses depth-aware spatial denoising and relies on TAA for temporal accumulation when available. One, two, and three passes are Sharp, Medium, and Soft; chained 3-by-3 passes produce the documented 5-by-5 spatial result. ([denoising](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#denoising), [`XeGTAO_Denoise`](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L607-L729), [settings](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.h#L125-L132)) | Two passes ping-pong raw -> filtered -> raw. The first uses center weight `1.2 / 5`, the final uses `1.2`, with the same packed-edge symmetry, leak, cardinal, and diagonal weights. ([pass sequence](../crates/openhp1-render/src/renderer/modern/ao.rs#L307), [denoiser](../crates/openhp1-render/src/shaders/modern/ao_denoise.wgsl#L10)) | Matches the reference Medium spatial filter. There is no temporal filter. |
| Normals | Intel advises supplying screen-space normals. Its fallback reconstructs them from depth and notes that the depth buffer is only a height-field representation, especially problematic at thin objects and discontinuities. ([normal recommendation](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#misc), [height-field limitation](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#thin-occluder-conundrum)) | Normals are reconstructed from the full-resolution `R32Float` view depth and the four edge weights. ([normal call](../crates/openhp1-render/src/shaders/modern/ao_xegtao.wgsl#L23), [reconstruction](../crates/openhp1-render/src/shaders/modern/ao_main.wgsl#L57)) | Source-supported fallback, but a plausible secondary source of shimmer around silhouettes and thin geometry. |
| Temporal consumer | Intel's temporal-noise result is intended to be accumulated by TAA; the README warns that temporal variance must remain low enough for TAA to treat it as noise. ([denoising discussion](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#denoising)) | The only AA consumers are FXAA and SMAA 1x. ([AA variants](../crates/openhp1-render/src/renderer/modern/aa.rs#L35), [settings](../crates/openhp1-render/src/settings.rs#L76)) | No history buffer, reprojection, or accumulation exists. |

The original GTAO paper is not a reason to add rotations alone. Its temporal
scheme combines half-resolution rendering, one direction per pixel, a 4-by-4
bilateral spatial reconstruction, six temporal rotations, reprojection, and
exponential accumulation to reach 96 effective directions. The changing
directions and the accumulator are one design
([Jimenez et al., section 4.1, PDF pages 3-4](https://www.activision.com/cdn/research/PracticalRealtimeStrategiesTRfinal.pdf)).

## Likely cause of moving noise

The code contains no time-varying value in the XeGTAO shader or AO uniform, so
intentional per-frame sample rotation is ruled out. At a motionless camera with
unchanged depth, the raw and filtered AO should be identical from frame to
frame.

During camera motion, however, a surface moves through the pixel-indexed R2
sequence, so its slice rotations and step offsets change as it crosses pixels.
The preceding High setting had only 18 depth taps, making this screen-space
crawl the strongest explanation for its moving noise. The current Ultra
setting raises that to 54 taps, but its two spatial passes still cannot
guarantee world-space stability. Intel's preferred final image adds TAA
accumulation, which OpenHP1 does not have. Depth-derived normals and the
single-layer depth-buffer representation can add more motion error at
silhouettes, rails, foliage-like cutouts, and depth discontinuities. These are
stronger remaining explanations than AO resolution or an accidental
frame-varying seed, because both current choices match the reference.

## Smallest fixes, in order

1. **Do not add frame-varying noise without TAA.** Keep the equivalent of
   `NoiseIndex = 0`. This prevents true temporal flicker and follows Intel's
   integration contract.
2. **Validate the current Ultra sampling change before adding machinery.** It
   is the smallest source-defined spatial fix: nine slices, three steps per
   side, no new resource or pass. Keep it only if the same slow camera pan is
   visibly better than High and its threefold main-pass sample cost is
   acceptable.
3. **If broad-surface crawl remains, A/B Intel's three-pass Soft denoise.** It
   can reuse the existing raw/filtered ping-pong textures and adds one spatial
   pass, but may blur fine contact detail. Keep two passes if the difference is
   not clear at normal playback speed.
4. **If the remaining shimmer is concentrated at silhouettes or thin
   geometry, supply geometric view-space normals.** This is the reference's
   preferred input, but it needs a normal target and scene-shader plumbing, so
   it is not the first fix.
5. **Only add temporal noise together with actual TAA.** A complete temporal
   change needs jitter/history/reprojection and history reset on resize,
   camera cuts, and invalid motion. Then add Intel's
   `288 * (frameIndex % 64)` offset. A frame counter alone is a regression.

Do not lower AO resolution as a stability fix. Intel presents half resolution
as a performance tradeoff that also needs bilateral upsampling. Do not first
increase the effect radius either: Intel reports that larger radii amplify the
depth-buffer height-field mismatch around thin objects.

## Validation criteria

- **Static determinism:** with a fixed camera and no animated depth-writing
  geometry, capture at least 120 consecutive raw and final AO frames. Their
  pixels must be identical. Any change disproves the fixed-noise diagnosis and
  should be traced before tuning quality.
- **Motion comparison:** in the rebuilt release viewer or game, record the same
  slow translation and yaw path with AO Off, the preceding High setting, and
  current Ultra. Use at least one broad wall/floor contact and one thin or
  silhouette-heavy area. Compare matching frames, not unrelated screenshots.
- **Success threshold:** current Ultra should visibly reduce crawling on broad
  contact shadows versus High without increasing haloing or losing fine
  contact detail. Reject it if the improvement is not clear at normal playback
  speed.
- **Cost:** record AO GPU time (or, until timestamps exist, repeatable whole
  frame time) at the normal internal resolution. Main-pass sample work rose
  from 18 to 54 taps; set a budget before retaining Ultra as the default.
- **Normal-path diagnosis:** if broad surfaces stabilize but rails and
  silhouettes do not, compare depth-derived normals with a normal-buffer
  prototype before changing denoise weights or radius.
- **Temporal acceptance:** if TAA is later added, a still camera must converge,
  slow motion must not shimmer more than the fixed-noise baseline, and camera
  cuts/resizes must show neither ghosted AO nor stale history.

The existing broader port notes remain in [xegtao.md](xegtao.md); this document
is limited to the current implementation's stability behavior and next steps.
