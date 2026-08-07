# XeGTAO integration notes

## Scope

This note records the primary-source behavior needed to port Intel's XeGTAO
ambient-occlusion term to OpenHP1's wgpu/WGSL modern renderer. It deliberately
excludes bent normals and temporal reprojection. XeGTAO supports spatial-only
operation: Intel describes its current filter as depth-aware spatial denoising
that can run without TAA, while TAA is an optional source of temporal
accumulation rather than a required XeGTAO pass
([Intel overview, Denoising](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#denoising)).

XeGTAO remains a screen-space approximation. An occluder that leaves the depth
buffer cannot continue contributing, and the depth buffer represents only the
frontmost surface. Intel explicitly identifies this height-field limitation as
the source of thin-object and depth-discontinuity artifacts
([Intel overview, Thin occluder conundrum](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#thin-occluder-conundrum)).

## Reference pipeline

Intel's integration has three required stages after opaque depth is available:

1. Convert device depth to positive view-space depth and build a five-level
   depth pyramid.
2. Search horizon angles in several screen-space slices, integrate visibility,
   and write raw visibility plus four depth-derived edge weights.
3. Spatially denoise visibility using those edge weights, ping-ponging the AO
   image for additional passes.

These are the reference `PrefilterDepths`, `MainPass`, and `Denoise` passes
([Intel overview, Implementation and integration](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#implementation-and-integration-overview),
[Intel integration shader](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/vaGTAO.hlsl)).
The reference executes all three at full display resolution. Its quality entry
points use the following horizon-search work:

| Quality | Slices | Steps per side per slice | Depth samples |
| --- | ---: | ---: | ---: |
| Low | 1 | 2 | 4 |
| Medium | 2 | 2 | 8 |
| High | 3 | 3 | 18 |
| Ultra | 9 | 3 | 54 |

Each step samples both directions along its slice. Intel recommends High as the
normal full-resolution preset; Medium is its lower-cost preset
([Intel overview, Resolution and sampling](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#resolution-and-sampling),
[Intel integration shader entry points](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/vaGTAO.hlsl#L94-L123)).

For OpenHP1, start with High at full resolution. Do not add a half-resolution
path until measurement justifies the bilateral upsample it would require;
Intel lists that as an explicit lower-quality optimization
([Intel FAQ](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#faq)).

## Depth and normals

The reference linearizes DirectX-style device depth into a positive view-space
distance and reconstructs positions with positive `z`
([XeGTAO depth helpers](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L95-L109)).
OpenHP1's depth buffer is `Depth32Float` with the same 0-to-1 NDC depth range,
but its current post-process position helper returns negative view-space `z`.
The port should keep an AO-local positive-depth convention rather than mixing
signs inside the Intel equations. Device depth at the clear value must produce
visibility `1.0` and must not seed the depth pyramid as nearby geometry.

Intel prefers supplied view-space normals, but includes a depth-derived normal
path. It calculates slope-adjusted left/right/top/bottom depth edges, then
weights the four cross-product candidates to avoid choosing across a depth
discontinuity
([XeGTAO edge and normal reconstruction](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L110-L145)).
The source warns that depth-derived normals visibly degrade with a 16-bit depth
buffer and recommends 32-bit working depth for this case
([XeGTAO main pass](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L235-L254)).
OpenHP1 has no normal buffer and already renders `Depth32Float`, so deriving the
normal from depth is the smallest compatible first implementation. It can be
done in the main pass; the reference retains that path specifically for engines
where a reusable separate normal pass is not useful.

The depth prefilter uses a weighted average of each 2-by-2 block. Samples near
the most distant of the four depths receive weight, which biases against thin
foreground occluders and was chosen for better motion stability than rotated
grid subsampling
([Intel overview, Memory bandwidth bottleneck](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#memory-bandwidth-bottleneck),
[XeGTAO depth filter](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L511-L533)).
The reference builds five mip levels in one 8-by-8 compute workgroup, where each
invocation initially handles a 2-by-2 pixel block
([XeGTAO depth prefilter](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L545-L622)).

## Horizon search and sampling

For each pixel, XeGTAO projects the surface normal onto each slice, searches
both directions for the maximum horizon cosine, analytically integrates the
two visible arcs, averages the slices, and applies the final visibility power
([XeGTAO main pass](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L295-L508),
[Jimenez et al. GTAO paper](https://www.activision.com/cdn/research/Practical_Real_Time_Strategies_for_Accurate_Indirect_Occlusion_NEW%20VERSION_COLOR.pdf)).
Samples use `pow(s, 2)` to concentrate work near the center, where small
crevices need detail. Distant samples use progressively coarser linear-depth
mips, selected by
`max(0, log2(sample_offset_pixels) - depth_mip_sampling_offset)`
([Intel overview, Sample distribution](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#sample-distribution),
[XeGTAO sampling loop](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L370-L425)).
Depth sampling must be point-filtered within a mip; Intel warns that linear
filtering between neighboring depth texels creates unwanted interpolation.

The spatial pattern is a 64-by-64, six-level Hilbert index driving a
two-dimensional R2 sequence. Intel reports that evaluating the Hilbert curve in
the shader costs about 7% and offers a precomputed 64-by-64 `R16Uint` lookup as
the faster alternative
([Intel overview, Sampling noise](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#sampling-noise),
[Intel noise function](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/vaGTAO.hlsl#L65-L83)).
A tiny generated lookup texture is reasonable here, but computing the index in
WGSL first is also correct and avoids another resource until profiling shows it
matters.

Without TAA, `NoiseIndex` must remain `0`; do not rotate or offset the pattern
between frames. The reference only uses `frameIndex % 64` when TAA is active
([XeGTAO constant update](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.h#L147-L175),
[Intel integration](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Effects/vaGTAO.cpp#L283-L290)).
Fixed spatial noise plus denoising removes intentional frame-to-frame flicker,
but a world point still crosses the fixed screen-space pattern as the camera
moves. Some residual shimmer is therefore expected without temporal
accumulation.

## Spatial denoising

The main pass writes four slope-adjusted edge weights for left, right, top, and
bottom neighbors. The reference packs the four two-bit values into one
`R8Unorm` texel. The denoiser enforces approximately symmetric edges, gives
diagonal taps weights derived from their two connecting cardinal edges, then
normalizes a 3-by-3 weighted sum
([XeGTAO edge packing](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L110-L129),
[XeGTAO denoiser](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L607-L729)).

Use two spatial passes for OpenHP1's no-TAA baseline. Intel labels one, two,
and three passes `Sharp`, `Medium`, and `Soft`; two chained 3-by-3 passes give a
5-by-5 footprint and match the spatial-only use case better than the one-pass
setting intended to work with later TAA
([XeGTAO version notes and settings](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.h#L18-L23),
[XeGTAO integration loop](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Effects/vaGTAO.cpp#L377-L405)).
The reference center weight is `1.2` on the last pass and `1.2 / 5` on earlier
passes. Preserve the source's slight leak across pixels surrounded by three or
four strong edges; Intel uses it to reduce spatial and temporal aliasing.

## Constants and OpenHP1 baseline

The current reference source defines these auto-tuned constants
([XeGTAO defaults](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.h#L93-L104)):

| Constant | Value |
| --- | ---: |
| Radius multiplier | `1.457` |
| Falloff range | `0.615` |
| Sample distribution power | `2.0` |
| Thin-occluder compensation | `0.0` |
| Final visibility power | `2.2` |
| Depth-mip sampling offset | `3.30` |
| Packed visibility scale | `1.5` |
| Denoise center weight | `1.2` |

The README prose mentions `3.15` for the depth-mip offset, but the archived
v1.30 source defines `3.30`; use the source value. Keep these heuristics fixed
for the first port. The physical effect radius is the one setting that must be
tuned in OpenHP1's Unreal units: Intel's default `0.5` is not portable across
scene scales. To preserve the existing SSAO's approximate 96-unit effective
reach while retaining the `1.457` radius multiplier, start near `66` units and
validate it on real game geometry. Radius tuning should compare contact detail,
haloing at depth discontinuities, and stability while moving the camera.

Other source constants worth preserving are the `1.3`-pixel minimum sample
distance, the `0.99999` center-depth bias for FP32 depth, and minimum final
visibility `0.03`
([XeGTAO main pass setup](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L250-L328),
[XeGTAO visibility output](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Shaders/XeGTAO.hlsli#L469-L508)).

## Minimal wgpu resources

The AO-only port needs:

- one full-resolution `R32Float` linear-depth texture with five mip levels;
- one full-resolution `R8Unorm` raw AO texture and one `R8Unorm`
  ping-pong/final AO texture;
- one full-resolution `R8Unorm` packed-edge texture; and
- optionally, one generated 64-by-64 `R16Uint` Hilbert lookup texture.

The Intel sample uses `R16Float` working depth for bandwidth, but explicitly
uses `R32Float` when generating normals from depth; it uses `R8Uint` for the AO
term and `R8Unorm` for packed edges
([Intel resource creation](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Effects/vaGTAO.cpp#L223-L262)).
OpenHP1 uses `R8Unorm` for both render-target outputs so the shared fragment
pipeline and composite path can read scalar visibility directly. Using FP32
WGSL math and `R32Float` depth is the portable starting point.
Intel's fp16 path is optional and reported as hardware- and driver-dependent;
WGSL also requires the optional `shader-f16` device feature before `f16` can be
enabled
([Intel FAQ](https://github.com/GameTechDev/XeGTAO/tree/e7698f874e90f2516fca26c696ec3cd2c70e505a#faq),
[WGSL enable extensions](https://www.w3.org/TR/WGSL/#enable-extension)).

WGSL has no HLSL-style `#include`: a shader is one module and its only standard
directives are `diagnostic`, `enable`, and `requires`
([WGSL modules and directives](https://www.w3.org/TR/WGSL/#module),
[WGSL directive grammar](https://www.w3.org/TR/WGSL/#directives)).
Keep each complete compute entry-point module self-contained, or concatenate
shared WGSL source in Rust before `create_shader_module`; do not add a shader
preprocessor dependency solely for this port.

The reference dispatches the prefilter as `ceil(width / 16)` by
`ceil(height / 16)`, the main pass as `ceil(width / 8)` by `ceil(height / 8)`,
and the denoiser as `ceil(width / 16)` by `ceil(height / 8)` because each
denoise invocation writes two horizontal pixels
([Intel pass orchestration](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/Source/Rendering/Effects/vaGTAO.cpp#L292-L405)).
WGSL should clamp texture reads and guard stores for partial edge workgroups.
No invocation may return before a workgroup barrier in the depth-prefilter
pass.

## Validation

Automated checks should validate WGSL parsing, resource formats and bind-group
layouts, ceil-divided dispatch over odd viewport sizes, background depth
remaining fully visible, and a small synthetic depth arrangement producing
less visibility in a corner than on an isolated plane.

Visual validation must use the rebuilt release game or viewer on several real
maps. For each SSAO and XeGTAO choice, hold the camera still to inspect spatial
noise, then translate and rotate slowly around player-sized actors, thin rails,
wall/floor contacts, and the viewport edge. Compare AO-off captures as the
lighting baseline. Expected remaining limitations are off-screen information
loss, no contribution from translucent surfaces that do not write depth, and
some no-TAA shimmer; these are not fixed by increasing the radius.

## License and attribution

Intel XeGTAO is MIT licensed and the repository is archived/read-only as of
April 2024
([Intel XeGTAO repository](https://github.com/GameTechDev/XeGTAO),
[Intel license](https://github.com/GameTechDev/XeGTAO/blob/e7698f874e90f2516fca26c696ec3cd2c70e505a/LICENSE)).
A direct WGSL port may copy and adapt the implementation, but distributed
copies or substantial portions must retain Intel's copyright and MIT permission
notice. Put that notice in the ported shader or the repository's third-party
notices. The GTAO paper explains the underlying algorithm; Intel's source is
the implementation reference for the XeGTAO-specific heuristics and defaults.
