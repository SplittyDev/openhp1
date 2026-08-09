# Modern renderer performance research (2026-08-09)

## Scope and conclusion

This note investigates the current wgpu 29 Modern renderer and volumetric
lighting path on Apple silicon. The constraint is strict: improve frame time
without reducing resolution, samples, precision, lighting coverage, or the
visible result.

The first implementation pass should target work that is provably redundant,
then reduce render-pass boundaries, then tune scheduling. The best candidates
are:

1. cache unchanged point-light shadow cube faces;
2. reduce redundant work in the froxel portal loop without removing any
   contributing portal or depth slice;
3. merge the three consecutive additive volumetric draws into one HDR render
   pass;
4. use wgpu multiview to draw six point-shadow faces per pass on supported
   adapters;
5. benchmark froxel workgroup shapes without changing global invocations or
   shader arithmetic.

These are hypotheses until measured on the real game. Apple explicitly
recommends prioritizing passes on the GPU critical path and using Occupancy,
Limiter, and Bandwidth counters to identify the actual constraint
([Apple Performance timeline](https://developer.apple.com/documentation/xcode/analyzing-apple-gpu-performance-using-a-visual-timeline)).

## Implemented result

The first candidate now keeps point-shadow slots until their exact source key
or conservatively intersecting caster bounds change. On the Apple M2 Ultra
headless Modern benchmark at 1024x768, `Lev_Tut1` improved from a five-run
median of 9.939 ms/frame to 8.806 ms/frame (11.4%) and from 210 to 198 steady
draw/pass submissions. Both paths produced checksum `477194a049a3c069`.
These are development-profile queued-GPU throughput measurements, not a claim
about every authored camera view.

The additive volumetric pass merge also completed. On a fixed `Lev_Tut1`
portal view it reduced steady submissions from 203 to 201 and improved the
five-run median from 10.100 to 9.804 ms/frame (2.9%), with identical checksum
`c4d0899930b48d3c`. Shadow generation and froxel compute remain separate;
only the three consecutive additive HDR draws share the render pass.

A follow-up release-viewer measurement showed that the original benchmark had
missed the dominant live-update cost: Classic was about 23 ms/frame, Modern
with every optional effect disabled was 35--40 ms/frame, volumetrics raised it
to about 41 ms/frame, and all effects reached 50--60 ms/frame. The benchmark
now has a 1792x1536 Retina workload and an `--updates` mode that exercises the
same `Renderer::update_scene` path used by animated viewer frames.

That mode exposed repeated full-image scans for unchanged light-sprite colors,
plus volumetric scene updates and frame preparation even when volumetrics were
disabled. Caching each unique source-texture color and refreshing it only from
reported texture changes reduced the full-effects development benchmark from
255.708 to a three-run median of 79.897 ms/frame (68.8%). Skipping the unused
volumetric update/prepare path reduced the baseline diagnostic from 253.066 to
a three-run median of 21.821 ms/frame (91.4%). Both retained their original
checksums. These deliberately update an unchanged full scene every frame to
isolate renderer update overhead; they are not release gameplay frame times.

The same follow-up removed per-pixel work that was provably unable to affect
the image: out-of-range real-time lights no longer sample their visibility
mask, and the composite no longer reads AO or bloom textures while those
effects are disabled. At 1792x1536, the five-run baseline median improved from
7.427 to 6.457 ms/frame (13.1%) with checksum `fbc483c96ac2f238` on both paths.

The local-volumetric update now also clusters fixture emitters once and shares
that result between light-volume energy scaling and point-shadow selection.
The same full-effects forced-update workload improved from 79.897 to a
three-run median of 70.217 ms/frame (12.1%), retaining checksum
`a38ca3a7a3464ee3` and 198 draw/pass submissions.

Directional shadow slices now persist until their exact light-view projection
or caster geometry changes. On the Retina portal-camera forced-update workload,
this removed three steady shadow passes and improved the three-run median from
70.723 to 68.992 ms/frame (2.4%), with checksum `4b3c4d710847bf8f` unchanged.

The local-volumetric light inputs now also persist across unrelated vertex
updates. A Time Profiler capture showed point fixture clustering and instance
packing repeating even when every light, corona, and source texture was
unchanged. Comparing only the fields that can affect those outputs reduced the
same forced-update workload from 73.781 ms/frame to a three-run median of
46.173 ms/frame (37.4%), with checksum `4b3c4d710847bf8f` and 198 submissions
unchanged. Geometry invalidation remains independent, so moving shadow casters
still invalidate the affected point shadows.

## Current cost model

The Modern frame is not one expensive shader. It is a chain of persistent HDR,
AO, volumetric, bloom, composite, and optional AA resources and passes:

- The BSP scene first renders into `Rgba16Float`; Modern then runs volumetrics,
  AO, three quarter-resolution bloom passes, the final tone-map composite, and
  optional FXAA or three-pass SMAA
  ([`ModernRenderer::render`](../crates/openhp1-render/src/renderer/modern.rs#L390)).
- AO uses a full-resolution `R32Float` depth pyramid, `R8Unorm` visibility and
  edge targets, one or up to five depth-preparation passes depending on the method,
  the AO pass, and two denoise passes
  ([`AoRenderer::render`](../crates/openhp1-render/src/renderer/modern/ao.rs#L250)).
- Directional volumetrics can render as many as four 1024x1024 shadow maps and
  make as many as 128 visible portals available to the froxel shader
  ([directional limits](../crates/openhp1-render/src/renderer/modern/volumetric/shadow.rs#L16)).
- Local volumetrics can select 20 point shadows. Each is a six-face 128x128
  cube, and every selected face currently gets a separate render pass every
  frame: a worst case of **120 shadow passes**
  ([point-shadow limits and render loop](../crates/openhp1-render/src/renderer/modern/volumetric/point_shadow.rs#L15)).
- The froxel volume is `ceil(width / 8) * ceil(height / 8) * 64` texels. One
  compute invocation owns an entire 64-slice ray and loops over every visible
  portal at every slice
  ([froxel dimensions](../crates/openhp1-render/src/renderer/modern/volumetric/froxel.rs#L9),
  [nested loops](../crates/openhp1-render/src/shaders/modern/froxel_volumetric.wgsl#L151)).

That last term is large before considering the work inside `portal_light`:

| Output size | Froxels written | Worst-case portal tests at 128 portals |
| --- | ---: | ---: |
| 1920x1080 | 2,073,600 | 265,420,800 |
| 2560x1440 | 3,686,400 | 471,859,200 |
| 3840x2160 | 8,294,400 | 1,061,683,200 |

Each surviving portal test can include aperture sampling and a comparison
sample from a 1024x1024 shadow map
([`portal_light`](../crates/openhp1-render/src/shaders/modern/froxel_volumetric.wgsl#L58)).
The actual portal count and early-return rate therefore belong in every
benchmark; the worst case is not a claim that shipped maps reach it.

The current path already makes several sound choices worth preserving: bloom
is quarter-resolution, AO uses single-channel targets, HDR and froxel storage
are already 16-bit-per-component textures, full-screen passes use one triangle,
and pipelines, bind groups, samplers, and targets persist across frames. A new
renderer architecture or dependency would not address the identified hot work.

## Measurement and visual-equivalence gate

### GPU measurement

Add optional pass-boundary timestamps, not a permanent readback stall. wgpu's
`TIMESTAMP_QUERY` is supported on Metal, works through
`RenderPassDescriptor::timestamp_writes` and
`ComputePassDescriptor::timestamp_writes`, and requires resolved ticks to be
multiplied by `Queue::get_timestamp_period()`
([wgpu `TIMESTAMP_QUERY`](https://docs.rs/wgpu/29.0.4/wgpu/struct.Features.html#associatedconstant.TIMESTAMP_QUERY)).
The adapter feature must be checked before requesting it; profiling must fall
back cleanly on unsupported devices. Arbitrary timestamps *inside* a pass are
not available on Apple GPUs, so pass begin/end timestamps are the portable
granularity
([wgpu timestamp feature limits](https://docs.rs/wgpu/29.0.4/wgpu/struct.FeaturesWGPU.html#associatedconstant.TIMESTAMP_QUERY_INSIDE_PASSES)).

For the Mac Studio baseline, also capture representative frames in Xcode's
Metal debugger. It reports pass duration and overlap, Occupancy, Limiter, and
Bandwidth counters, plus shader instruction costs
([Metal debugger](https://developer.apple.com/documentation/xcode/metal-debugger),
[Apple GPU timeline](https://developer.apple.com/documentation/xcode/analyzing-apple-gpu-performance-using-a-visual-timeline)).
The Metal Performance HUD can provide encoder timing during a live run by
setting `MTL_HUD_ENCODER_TIMING_ENABLED=1`
([Metal Performance HUD](https://developer.apple.com/documentation/xcode/monitoring-your-metal-apps-graphics-performance/)).

Use three real-game captures rather than one average scene:

- a portal-heavy view for directional shadows and froxels;
- a local-light-heavy view that approaches the selected point-shadow budget;
- a high-resolution view with AO, bloom, and the configured AA path enabled.

Warm pipeline compilation and resource creation before sampling. Record median
and p95 CPU frame time, GPU frame time, per-pass GPU time, draw/pass counts,
visible portal count, selected point-shadow count, resolution, renderer
settings, and GPU/OS identity. Apple notes that pass setup has latency and that
too many small passes are themselves a performance problem
([Apple Performance timeline](https://developer.apple.com/documentation/xcode/analyzing-apple-gpu-performance-using-a-visual-timeline)).

### Output equivalence

Every optimization needs fixed-camera, fixed-resolution, fixed-settings, and
fixed-animation-time captures of Composite plus all volumetric debug views.
Compare the final image and relevant intermediate attachments (point shadows,
directional shadows, integrated froxel volume, AO, and HDR scene target). For a
change that only skips identical work or changes scheduling, require identical
attachments. For a shader algebra change, use a tight image-difference
threshold followed by an authored moving-camera replay.

Bitwise identity is not automatically portable after shader algebra is
rearranged: WGSL permits floating-point reassociation/fusion within its accuracy
rules
([WGSL floating-point evaluation](https://www.w3.org/TR/WGSL/#floating-point-evaluation)).
That is a reason to prefer caching, pass merging, and scheduling changes first,
not a reason to accept a visible difference.

## Ranked candidates

Impact is an informed estimate from the current cost model, not a measured
result.

| Rank | Candidate | Expected impact | Output-equivalence risk | Deciding measurement |
| ---: | --- | --- | --- | --- |
| 1 | Cache unchanged point-shadow faces | Very high when local volumetrics are active | Low if invalidation is complete; high if it is heuristic | Point-shadow GPU time, pass count, and captured depth layers |
| 2 | Conservatively cull froxel portal work and remove duplicate slice-distance evaluation | Very high on portal-heavy views | Low for conservative culling; low-medium for precomputed float data | Froxel compute time, portal tests, occupancy/limiter, froxel texture diff |
| 3 | Merge additive volumetric HDR passes | High on Apple TBDR, especially at high resolution | Low | GPU read/write bandwidth and volumetric render-pass time |
| 4 | Render six point-shadow faces with multiview | High on cache misses or dynamic scenes | Low-medium; feature/backend path must be covered | Shadow pass count, vertex time, CPU encoding time, depth-layer diff |
| 5 | Benchmark froxel workgroup shapes | Medium but hardware-dependent | Very low | Froxel compute time and occupancy on Apple plus one non-Apple adapter |
| 6 | Fuse final AO denoise into Modern composite only with explicit R8 equivalence | Medium if AO is hot | Medium because the current R8 target quantizes before composite | AO/composite time and exact final-AO/final-image diff |
| 7 | Pack or stage many small uniform writes | Low-medium CPU win before caching; probably low after it | None | CPU allocation/encoding profile and queue-write count |
| 8 | Reuse static draw commands with render bundles | Low unless CPU command encoding is hot | None | CPU renderer time with unchanged GPU pass time |

### 1. Cache unchanged point-shadow faces

Today `prepare` rewrites six `FaceUniform` buffers per selected point source and
`render` clears and redraws all six faces per source on every frame
([point-shadow prepare/render](../crates/openhp1-render/src/renderer/modern/volumetric/point_shadow.rs#L239)).
The face matrix depends on the selected source and face, not the camera; the
camera only controls source selection. The shadow depth also depends on the
shared shadow-caster geometry.

Keep the cube-array texture and reuse a slot when both of these are unchanged:

- the complete selected source state used by `face_uniform`;
- a generation or exact identity for the shadow-caster vertex data.

Only clear and redraw the six dirty faces. Never use a timer, movement
threshold, map name, or assumed-static actor class. Scene updates that change
source position/radius or any shadow-caster vertex must invalidate the affected
slot. wgpu's `StoreOp::Store` preserves the attachment result for later use;
`Discard` would explicitly make it uninitialized
([wgpu `StoreOp`](https://docs.rs/wgpu/29.0.4/wgpu/enum.StoreOp.html)).
This makes reuse an output-preserving removal of repeated work, provided the
invalidation key is complete.

This should be implemented before multiview or upload infrastructure: a cached
face costs no render pass at all.

### 2. Remove redundant froxel portal work

The kernel currently evaluates `slice_distance(z)` and
`slice_distance(z + 1)` on every iteration, even though one iteration's end is
the next iteration's start. Carry the previous `segment_end` forward so each
boundary is evaluated once. This leaves the same 65 boundary evaluations and
the same integration order, while avoiding up to 63 duplicate `pow` calls per
screen tile. Confirm in the Metal shader profiler because a compiler may
already eliminate some repetition.

The larger opportunity is a **conservative, order-preserving portal list per
screen tile**. The current CPU path globally camera-culls portals, but every
remaining 8x8 froxel column still tests every portal at all 64 depths. Build a
list containing every extruded portal prism that could intersect rays through
that tile, preserve the original portal order, and iterate only that list. A
portal may be removed only when the intersection test proves it cannot
contribute anywhere in the tile/depth interval. Frostbite's production froxel
design aligned volume tiles with tiled-light lists and reused post-culling
lists; this is the authoritative precedent, not a mandate to copy its broader
renderer
([EA/Frostbite overview](https://www.ea.com/news/physically-based-unified-volumetric-rendering-in-frostbite),
[SIGGRAPH 2015 Frostbite course material](https://www.advances.realtimerendering.com/s2015/Frostbite%20PB%20and%20unified%20volumetrics.pptx)).

Do not start by changing `TILE_SIZE`, `DEPTH_SLICES`, the 1500-unit distance,
portal order, shadow filtering, or noise. Those change sampling or floating
point accumulation and therefore need a separate visual-quality decision.

Precomputing portal-invariant edges, normalized direction, determinant, and
center prism into the portal buffer may be worthwhile after culling. It is
lower priority because moving those floating-point operations from WGSL to CPU
or changing their evaluation order may change low bits; it needs intermediate
texture comparison and an authored replay.

### 3. Merge additive volumetric draws into one HDR pass

After shadow maps and the froxel compute dispatch complete, the current code
opens three render passes on the same HDR target in this order:

1. froxel composite (`Load` + `Store`)
   ([froxel composite](../crates/openhp1-render/src/renderer/modern/volumetric/froxel.rs#L238));
2. window projection / shafts / dust motes (`Load` + `Store`)
   ([shaft pass](../crates/openhp1-render/src/renderer/modern/volumetric/shadow.rs#L539));
3. local light volumes (`Load` + `Store`)
   ([local volume pass](../crates/openhp1-render/src/renderer/modern/volumetric.rs#L306)).

All three use additive blending and sample scene depth or shadow resources, not
the updated HDR target. Encode their draws in the same order into one render
pass with one initial `Load` and one final `Store`. Preserve all debug-view
branches.

Apple explicitly calls this case out: consecutive passes sharing the same
render target can be merged to avoid round trips from tile memory to system
memory, saving significant bandwidth
([Bring your game to Mac, Part 3](https://developer.apple.com/videos/play/wwdc2023/10125/)).
Apple silicon GPUs are tile-based deferred renderers
([Apple TBDR documentation](https://developer.apple.com/documentation/metal/tailor-your-apps-for-apple-gpus-and-tile-based-deferred-rendering)),
and wgpu notes that attachment stores/loads matter especially on tile hardware
([`StoreOp`](https://docs.rs/wgpu/29.0.4/wgpu/enum.StoreOp.html),
[`LoadOp`](https://docs.rs/wgpu/29.0.4/wgpu/enum.LoadOp.html)).

This refactor should not merge passes with real read-after-write dependencies.
AO mip generation, bloom blur, SMAA, and the final composite consume previous
pass outputs and must retain those boundaries unless their algorithms change.

### 4. Render point-shadow cube faces with multiview

When a point shadow is dirty, six passes draw the same geometry with six view
matrices. wgpu's `MULTIVIEW` feature enables multiview render passes and
`@builtin(view_index)` on Metal, Vulkan, and D3D12
([wgpu `MULTIVIEW`](https://docs.rs/wgpu/29.0.4/wgpu/struct.FeaturesWGPU.html#associatedconstant.MULTIVIEW)).
Use a six-layer texture view and six matrices so one pass/draw targets all six
faces. Keep the current path as the fallback when the adapter lacks the feature.

Apple documents rendering one draw to multiple array or cube texture slices
([Apple layered rendering](https://developer.apple.com/documentation/metal/rendering-to-multiple-texture-slices-in-a-draw-command)).
Apple's feature tables report layered rendering and up to eight vertex-amplified
views on M1-family Apple7 hardware, enough for a six-face cube
([Metal feature set tables](https://developer.apple.com/metal/feature-sets/)).

Do this after caching. Multiview reduces pass setup and geometry submission; it
does not remove shadow rasterization on frames where the cube really changed.

### 5. Benchmark froxel workgroup shapes

The current `@workgroup_size(8, 8, 1)` has 64 independent invocations and no
workgroup memory or barriers. Shapes such as 8x4, 16x4, 8x8, and 16x8 can keep
the same global IDs and shader math while changing scheduling. Apple recommends
making threadgroup size a multiple of the pipeline's `threadExecutionWidth`,
but the best size also depends on register use and occupancy and therefore must
be measured
([Apple threadgroup sizing](https://developer.apple.com/documentation/metal/calculating-threadgroup-and-grid-sizes),
[`threadExecutionWidth`](https://developer.apple.com/documentation/metal/mtlcomputepipelinestate/threadexecutionwidth)).

Because each froxel invocation has long serial loops and many temporaries, a
larger workgroup can reduce occupancy instead of improving it. Keep only a
shape that improves representative GPU medians and does not regress another
supported GPU class. No runtime tuner is needed.

### 6. AO final-denoise fusion: only after proof

AO currently writes the final denoised value to an `R8Unorm` target, then the
Modern composite loads that one byte per pixel
([AO final pass](../crates/openhp1-render/src/renderer/modern/ao.rs#L321),
[composite AO load](../crates/openhp1-render/src/shaders/modern/composite.wgsl#L25)).
If profiling shows this pass is material, the composite shader could directly
perform the final denoise from the first-pass filtered AO and edges, eliminating
one full-screen pass and one store/read pair.

This is not automatically exact: the current path quantizes through `R8Unorm`
before composite, and WebGPU permits implementation-defined choices when
converting floating-point shader output to non-integer attachment formats
([WebGPU output merging](https://gpuweb.github.io/gpuweb/#output-merging)).
Accept the fusion only if it explicitly reproduces the current quantization on
all supported backends and the captured AO/final output passes the equivalence
gate. Otherwise keep the simple existing pass.

### 7. Reduce upload overhead only if CPU profiling finds it

The point-shadow path can issue 120 tiny `Queue::write_buffer` calls before its
120 passes, in addition to camera, portal, volume, AO, and post-process writes.
Caching removes most point-face writes first. If CPU allocation or queue-write
encoding remains hot, pack face uniforms into one aligned buffer/write or use
wgpu's existing `StagingBelt`.

wgpu documents that native queue writes use short-lived staging allocations;
`write_buffer_with` does not help small struct data by itself, while
`StagingBelt` is specifically intended to share and reuse storage for many
small writes
([wgpu `Queue`](https://docs.rs/wgpu/29.0.4/wgpu/struct.Queue.html#method.write_buffer_with),
[`StagingBelt`](https://docs.rs/wgpu/29.0.4/wgpu/util/struct.StagingBelt.html)).
Do not add a custom ring allocator unless the existing utility measurably falls
short.

### 8. Render bundles only for a measured CPU bottleneck

Static BSP/shadow draw state can be recorded in a wgpu render bundle and replayed.
wgpu describes bundles as reusable and often more efficient than manually
reissuing their commands
([`RenderBundleEncoder`](https://docs.rs/wgpu/29.0.4/wgpu/struct.RenderBundleEncoder.html)).
They reduce CPU encoding, not shader, raster, or bandwidth work. Consider them
only if Metal System Trace shows the CPU failing to feed the GPU after shadow
caching and pass reduction.

## Techniques rejected under the no-look-change constraint

- **No lower froxel resolution, fewer depth slices, shorter ray march, fewer
  local-light steps, reduced shadow resolution, dynamic resolution, or
  MetalFX.** All change sampling or spatial detail.
- **No temporal reprojection or checkerboard updates.** They can be fast and
  visually plausible, but introduce history, ghosting/disocclusion behavior,
  and frame-to-frame differences.
- **No `f16` arithmetic or narrower replacement formats.** Apple documents
  real register/bandwidth benefits from lower precision, but it changes
  quantization; the path already stores HDR and froxels as `Rgba16Float`
  ([Apple GPU optimization guidance](https://developer.apple.com/videos/play/wwdc2020/10603/)).
- **No lossy texture compression or filtering substitutions.** They cannot
  guarantee the current image.
- **No manual resource barriers.** WebGPU defines usage scopes and the
  implementation manages transitions
  ([WebGPU resource usages](https://gpuweb.github.io/gpuweb/#resource-usages));
  the current frame already uses one encoder, so manual transitions are not a
  first-order opportunity.
- **No bindless/argument-buffer rewrite, async renderer, ECS, or job system.**
  The current hot candidates are repeated GPU work and pass boundaries, and the
  renderer already persists its bind groups and pipelines.
- **No pipeline-cache project for Metal frame time.** wgpu 29 documents its
  pipeline-cache feature as implemented for Vulkan, not Metal
  ([wgpu `PIPELINE_CACHE`](https://docs.rs/wgpu/29.0.4/wgpu/struct.FeaturesWGPU.html#associatedconstant.PIPELINE_CACHE)).

## Recommended measured sequence

1. Capture baseline CPU/GPU/counter data and fixed-time images.
2. Cache point-shadow faces with complete invalidation; stop if it does not
   move the point-shadow timing.
3. Merge the additive volumetric HDR passes; confirm draw order and exact
   output.
4. Remove duplicate slice-boundary evaluation, then add conservative per-tile
   portal lists only if froxel compute remains hot.
5. Add the multiview point-shadow path only if dirty/dynamic shadow frames are
   still material.
6. Benchmark a small fixed set of workgroup shapes and keep one winner.
7. Profile AO, upload allocation, and CPU encoding before considering the
   lower-ranked candidates.

Each accepted optimization should be a separate commit with its own before /
after timing, behavior counters, fixed-time image comparison, and a short live
Modern-renderer replay. Stop when the next candidate is no longer material;
speculative shader micro-optimization is not a performance pass.
