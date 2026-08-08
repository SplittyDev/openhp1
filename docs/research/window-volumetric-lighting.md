# Window volumetric lighting in real-time engines

Research date: 2026-08-09

Implementation note: the Stage 2 froxel MVP described below was implemented
after live validation showed that Stage 1's corrected integration remained
visually dominated by the independently rasterized portal prisms.

## Conclusion

Production engines do not normally render each light shaft as translucent
geometry. They evaluate participating-media density and shadowed lighting in a
low-resolution, camera-aligned 3D grid (a froxel volume), integrate scattering
and extinction along each view ray, and sample that integrated volume when
compositing the scene. Spatial filtering, sub-voxel jitter, and temporal
reprojection make the deliberately low-resolution result smooth.

OpenHP1 currently ray-marches each portal triangle's extruded frustum directly
into the frame. That can produce a useful shaft, but its shape remains the
straight prism/frustum supplied by the window, overlapping portal draws add
independently, and the density field is sampled only once at the ray interval's
midpoint. The result therefore reads as translucent geometry more than a body
of air, and the midpoint changes with the camera ray, explaining much of the
view-dependent appearance.

The smallest correct next step is not to bend the shafts or add another edge
blur. It is to make the existing 32-step march perform real front-to-back
single-scattering integration: sample world-space density at every step,
accumulate in-scattered radiance under Beer-Lambert transmittance, and use a
normalized anisotropic phase function. That directly addresses the flat beam
body without introducing a new render target. A camera-aligned froxel volume is
the proper subsequent architecture if that prototype confirms the diagnosis.

## What production implementations do

### 1. Store the medium and lighting in a view-aligned volume

Bart Wronski's Ubisoft implementation separates the work into lighting and
shadowing per volume cell, density evaluation, integration through the volume,
and final application. Its 3D texture is aligned to the camera frustum, with an
exponential distribution of depth slices; the shipped configurations were
`160x90x64` or `160x90x128`. The final scene samples the integrated 3D texture
rather than re-running a ray march for every surface pixel. See slides 22-28
and 38-44 of [Volumetric Fog: Unified, Compute Shader Based Solution to
Atmospheric Scattering](https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf).

Unity HDRP likewise describes its implementation as a "froxel" lighting
algorithm, chosen for sub-native-resolution rendering and temporal
reprojection ([SIGGRAPH 2018 HDRP presentation](https://www.advances.realtimerendering.com/s2018/Siggraph%202018%20HDRP%20talk_with%20notes.pdf)).
Its public API exposes the 3D buffer's slice count, depth extent, and
linear-versus-exponential depth distribution
([Unity HDRP `Fog` API](https://docs.unity.cn/Packages/com.unity.render-pipelines.high-definition@10.5/api/UnityEngine.Rendering.HighDefinition.Fog.html)).

Epic documents the same high-level model: participating-media density and
lighting are computed throughout the camera frustum, and quality depends on
the volume texture resolution and number of depth slices
([Unreal Engine Volumetric Fog](https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine)).

Frostbite's unified system extends this model with a volume representing
extinction, voxelized local particles, and volumetric shadowing shared by
lights. The important architectural point is that participating media is one
scene volume rather than a collection of independently composited beam meshes
([Towards Unified and Physically-Based Volumetric Lighting in
Frostbite](https://www.advances.realtimerendering.com/s2015/index.html#_Toc417095387)).

### 2. Integrate scattering and extinction, not just lit distance

The medium has an extinction coefficient `sigma_t` and scattering coefficient
`sigma_s`. For each segment of a view ray, production implementations update
transmittance according to Beer-Lambert:

```text
segment_T = exp(-sigma_t * step_length)
L += path_T * incident_light * phase * sigma_s
     * (1 - segment_T) / sigma_t
path_T *= segment_T
```

The quotient has the `step_length` limit when `sigma_t` approaches zero. This
front-to-back integration makes near density attenuate farther scattering and
provides the depth cue missing from a simple nonlinear remap of total lit
length. Wronski presents the physical basis and numerical volume integration
in slides 8 and 38-41 of the
[Ubisoft presentation](https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf).

### 3. Apply a physical phase function

The phase function controls how much incident light scatters toward the camera.
For dust and aerosol-like media it is anisotropic, commonly approximated with
Henyey-Greenstein. Wronski explains the angular role of the phase function and
the practical Henyey-Greenstein model in slides 9-11 of the
[same presentation](https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf).
Unreal exposes this as **Scattering Distribution**: zero is isotropic and a
value near one is strongly forward-scattering
([Unreal Engine Volumetric Fog](https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine)).
Unity exposes equivalent anisotropy and warns that strong anisotropy can make
temporally reprojected shadows less stable
([Unity HDRP Volumetric Lighting](https://docs.unity.cn/Packages/com.unity.render-pipelines.high-definition@16.0/manual/Volumetric-Lighting.html)).

A phase function legitimately makes shafts change brightness with view angle.
It should not, however, make their geometry jump or reveal a screen-facing
slab. A moderate anisotropy is therefore the appropriate starting point for an
indoor haze; strong forward scattering should be a later visual tuning choice.

### 4. Inject shadows and cookies into the volume

The window mask is a light cookie: each volume cell back-projects to the
window/light projection and samples transmission, while the directional shadow
map determines whether opaque geometry blocks the cell. Unreal explicitly
supports a directional light with shadowing and a Light Function in volumetric
fog, and describes Light Functions as projected light masks that also work with
volumetric fog
([Unreal Volumetric Fog](https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine),
[Unreal Light Functions](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-light-functions-in-unreal-engine)).

Volumetric shadow information is deliberately lower-frequency than surface
shadows. Ubisoft found that wide PCF on the full-resolution cascades was still
costly and unstable, so it downsampled and separably filtered an exponential
shadow representation before volume lighting (slides 29-34 of the
[Ubisoft presentation](https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf)).
This is a better match for stained glass and airborne scattering than repeatedly
sampling a high-frequency binary shadow at every full-resolution pixel.

### 5. Jitter, filter, and reproject the low-resolution result

A regular undersampled grid turns high-frequency shadow and density detail into
bands and flicker. Jitter trades that structured aliasing for noise; spatial
filtering and temporal reprojection then average the noise over multiple
frames. Wronski shows that one-sample temporal jitter and reprojection removes
most volume edge artifacts (slides 55-59 of the
[Ubisoft presentation](https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf)).
Epic says its low-resolution, camera-aligned volume uses a different sub-voxel
jitter each frame and a heavy temporal reprojection filter, with light trails
as the known tradeoff
([Unreal Engine Volumetric Fog](https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine)).
Unity similarly documents reprojection as a quality improvement that can ghost
dynamic lights; HDRP also offers Gaussian denoising for dynamic content
([Unity HDRP Volumetric Lighting](https://docs.unity.cn/Packages/com.unity.render-pipelines.high-definition@16.0/manual/Volumetric-Lighting.html),
[Unity HDRP `Fog` API](https://docs.unity.cn/Packages/com.unity.render-pipelines.high-definition@10.5/api/UnityEngine.Rendering.HighDefinition.Fog.html)).

An alternative first-party sampling result from Studio Gobo keeps volume
samples stationary in the volume and distributes samples approximately as
`1/z`, specifically to avoid motion aliasing while retaining near-camera
quality
([A Novel Sampling Algorithm for Fast and Stable Real-Time Volume
Rendering](https://www.advances.realtimerendering.com/s2015/index.html#_A_Novel_Sampling)).

## Why OpenHP1's current shafts look flat

The current implementation in
`crates/openhp1-render/src/shaders/modern/sky_shafts.wgsl` has several
properties that explain the screenshots:

1. **The visible boundary is authored geometry.** `portal_interval` intersects
   the camera ray with an extruded portal triangle. Even after widening, every
   cross-section is a linear interpolation between two similar triangles. A
   directional sun should produce straight rays, but drawing that boundary
   directly makes the volume read as a translucent prism.
2. **Density is not integrated in 3D.** The 32 samples integrate window
   transmission and shadow visibility, but `volumetric_dust` is evaluated only
   once at the interval midpoint and multiplied into the completed beam. Every
   point on that camera ray therefore shares one density value. As the view
   changes, the midpoint changes, so the apparent density pattern changes with
   the camera rather than revealing stable world-space structure through
   parallax.
3. **There is no path transmittance.** `1 - exp(-lit_length * density)` maps lit
   distance to brightness, but it does not attenuate farther in-scattering by
   the density already crossed. That removes an important depth cue and makes
   long grazing intersections brighten or saturate abruptly.
4. **The phase approximation is unnormalized and strongly shaped.** The current
   constant-plus-fourth-power function creates view-angle brightness changes,
   but it is not tied to scattering or extinction and cannot conserve energy.
5. **Portal triangles are separate additive draws.** A camera ray can cross
   several overlapping surface triangles/frustums. Their independent nonlinear
   integrations then add in HDR, so overlap and grazing-angle changes can show
   as view-dependent seams or wedges even when their source tint matches.
6. **Fixed sample positions expose the 32-step discretization.** There is no
   spatial jitter or temporal history for the shaft march. Increasing the step
   count only moves this ceiling and multiplies the shadow/cookie cost.

The straight light direction is not itself a defect. Sunlight is effectively
directional at room scale, so production-quality shafts remain straight. The
perception of volume comes from stable three-dimensional density variation,
front-to-back extinction, anisotropic scattering, occlusion, and filtered
sampling—not from curving the rays.

## Recommended implementation path

### Stage 1: correct the existing march

This is the smallest correct next step and the fastest test of the diagnosis.

- Move `volumetric_dust(position, ...)` into the existing 32-sample loop.
- Treat it as `sigma_t` and derive `sigma_s = albedo * sigma_t`.
- Accumulate radiance and path transmittance front-to-back using the equations
  above. Keep the existing aperture and shadow lookup as incident-light terms.
- Replace `directional_phase` with normalized Henyey-Greenstein using one
  moderate fixed anisotropy initially. Do not add another tuning UI until the
  model is visually validated.
- Remove the final midpoint-haze multiplication and the nonlinear lit-length
  remap; the physical integration replaces both.
- Validate in scattering-only mode with the floor projection disabled or
  ignored, using the same saved camera positions from the supplied screenshots.

This remains a per-portal approximation, so it will not eliminate every overlap
or camera-grazing artifact. It should, however, make the beam body visibly
three-dimensional and world-anchored. If it does not, a froxel rewrite should
not be started until the light direction, portal classification, and units are
rechecked.

### Stage 2: replace portal rendering with a froxel MVP

Keep the existing aperture masks, portal extraction, directional shadow map,
and world-space density helper. Change only where the scattering result lives:

1. Allocate a camera-aligned 3D `rgba16float` texture at roughly one froxel per
   8x8 screen pixels and 64 exponentially distributed depth slices. At the
   current 1024x768 internal resolution this is `128x96x64`, about 6 MiB per
   RGBA16F texture.
2. In a compute pass, evaluate each froxel's world-space density, window-cookie
   transmission, shadow visibility, phase response, and incident radiance.
   Store scattering RGB and extinction A. The existing portal frustums can
   bound which windows affect a froxel, but must no longer be the rendered
   surface.
3. In a second compute pass, scan each XY column front-to-back and store
   integrated scattering RGB plus transmittance.
4. Composite by sampling the integrated volume at the full-resolution scene
   depth. Trilinear sampling provides spatial filtering and avoids the
   depth-discontinuity problem of a low-resolution 2D post-effect.

For the first MVP, cap and loop over the already-visible window surfaces in the
froxel-lighting pass. Add tiled/clustered portal lists only if profiling shows
the simple loop is too expensive. This deliberately postpones infrastructure
until a real need appears.

### Stage 3: temporal stability and softer source detail

- Jitter the froxel sample within each cell every frame.
- Reproject the previous integrated volume with the previous view-projection
  transform; reject history on camera cuts and out-of-volume samples.
- Clamp history to the current spatial neighborhood before blending to limit
  ghost trails.
- Prefilter or downsample the directional shadow data used by volumetrics.
  Surface shadows can remain sharp; airborne scattering should consume the
  lower-frequency representation.
- Keep density variation low-frequency. One or two world-space noise octaves
  are sufficient for dusty indoor air; more octaves mostly create shimmer.

The static castle and directional sunlight are favorable for temporal
accumulation. Moving dust motes should remain a separate depth-clipped particle
layer rather than being baked into long-lived history.

## Expected result and limits

Stage 1 should replace the camera-dependent translucent slab with stable,
varied density inside a still-straight shaft. Stage 2 removes per-triangle
additive composition and provides smooth 3D interpolation at scene depth.
Stage 3 addresses sampling bands, flicker, and high-frequency shadow noise.

None of these stages should deliberately warp sunlight. If a softer stained
glass source is wanted after the medium is correct, represent it as a small
angular spread or distance-growing cookie/shadow filter. That is a source
model, not a substitute for volumetric integration.

## Primary sources

- Bartlomiej Wronski, Ubisoft Montreal, [Volumetric Fog: Unified, Compute
  Shader Based Solution to Atmospheric Scattering, SIGGRAPH
  2014](https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf).
- Sébastien Hillaire, Electronic Arts / Frostbite, [Towards Unified and
  Physically-Based Volumetric Lighting in Frostbite, SIGGRAPH
  2015](https://www.advances.realtimerendering.com/s2015/index.html#_Toc417095387).
- Unity Technologies, [High Definition Render Pipeline, SIGGRAPH
  2018](https://www.advances.realtimerendering.com/s2018/Siggraph%202018%20HDRP%20talk_with%20notes.pdf).
- Unity Technologies, [HDRP Volumetric
  Lighting](https://docs.unity.cn/Packages/com.unity.render-pipelines.high-definition@16.0/manual/Volumetric-Lighting.html)
  and [`Fog` API](https://docs.unity.cn/Packages/com.unity.render-pipelines.high-definition@10.5/api/UnityEngine.Rendering.HighDefinition.Fog.html).
- Epic Games, [Volumetric Fog in Unreal
  Engine](https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine)
  and [Light
  Functions](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-light-functions-in-unreal-engine).
- Huw Bowles and Daniel Zimmermann, Studio Gobo, [A Novel Sampling Algorithm
  for Fast and Stable Real-Time Volume Rendering, SIGGRAPH
  2015](https://www.advances.realtimerendering.com/s2015/index.html#_A_Novel_Sampling).
