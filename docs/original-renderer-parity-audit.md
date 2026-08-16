# Original renderer parity audit

Status: breadth-first inventory, not a parity verdict. This document records behavior visible in the legally obtained shipped game files. It does not use leaked source. Decompiled types and field names are imperfect; rows say **uncertain** where the observable operation is clearer than its semantic name.

## Sources and citation rules

- **R**: shipped `Render.dll` decompilation, [`res/Ghidra_Render.c`](../res/Ghidra_Render.c). A citation gives the exact decompiler line or enclosing function range.
- **E**: shipped `Engine.dll` decompilation, [`res/Ghidra_Engine.c`](../res/Ghidra_Engine.c), followed only where `Render.dll` delegates scene-coordinate, zone, viewport-hit, or render-device behavior.
- **C**: shipped English configuration/localization, [`res/System/0/Default.ini`](../res/System/0/Default.ini) and [`res/System/Startup.int`](../res/System/Startup.int).

The decompilations are primary binary evidence, but inferred class fields and unnamed helper purposes are not automatically facts. “Confirms” below means the operation is directly present; “uncertain” means names or higher-level intent still need package/default/bytecode or live differential proof.

## Coverage method

1. Extract every top-level decompiled function body by counting lines containing only `{`; this yields **514** bodies in `Ghidra_Render.c` ([R:1344](../res/Ghidra_Render.c#L1344)-[R:43361](../res/Ghidra_Render.c#L43361)).
2. Reconcile them into **69 named methods**, **188 unnamed `FUN_` implementations**, **60 `thunk_FUN_` forwarding/duplicate entry points**, **9 module/compiler-runtime functions**, **156 catch handlers**, and **32 unwind handlers**. The appendix lists all 514.
3. Map public renderer responsibilities and behavior-bearing unnamed ranges into the matrices below. Catch/unwind/compiler helpers are tracked for count completeness but are not treated as rendering semantics without a call-path reason.
4. For each behavior, separately audit Classic and Modern OpenHP1, then attach one verification artifact: synthetic test, shipped-corpus scan, deterministic capture/diff, or live retail/OpenHP1 comparison. A checked feature row requires all three status cells to be filled and evidence linked.
5. Continue from exported call paths (`DrawWorld` → `OccludeFrame`/`DrawFrame`; actor, mesh, particle, lighting, decal, and span-buffer paths), then classify the remaining unnamed functions by callers. “Every behavior” is demonstrated only when every appendix entry is either mapped to a checked feature, proven duplicate/forwarder, or proven non-behavioral compiler/runtime scaffolding.

Placeholders: **Classic/Modern** = `⬜ audit`; **verification** = `⬜ none`. These deliberately make no claim about current OpenHP1 support.

## Device selection and shipped quality policy

| Done | Feature / behavior | Primary evidence | Observable semantics | Classic | Modern | Verification |
|---|---|---|---|---|---|---|
| [ ] | Renderer module selection | [C:19-36](../res/System/0/Default.ini#L19) | `Render=Render.Render`; game/windowed defaults select software while fullscreen `RenderDevice` selects Glide. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Selectable render-device families | [C:20-30](../res/System/0/Default.ini#L20), [C:20-25](../res/System/Startup.int#L20) | Shipped UI describes Software, Glide, Direct3D, experimental OpenGL, and Metal devices. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Viewport/color defaults | [C:70-96](../res/System/0/Default.ini#L70) | Windows client defaults to 640×480, 16-bit windowed/fullscreen, brightness `0.4`, mip factor `1`, fullscreen startup, screen flashes, decals, dynamic lights, and particle density `1`. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Software feature policy | [C:216-224](../res/System/0/Default.ini#L216) | Translucency and volumetric lighting are enabled; shiny surfaces, coronas, and high-detail actors are disabled; smoothing and fast-translucency knobs are authored. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Hardware feature policy | [C:226-272](../res/System/0/Default.ini#L226) | Glide/Metal/OpenGL/D3D enable translucency, volumetrics, shiny surfaces, coronas, and high-detail actors, with device-specific detail-texture choices. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | D3D texture/presentation policy | [C:255-272](../res/System/0/Default.ini#L255) | D3D enables mipmapping, multitexture, palettes, gamma correction, triple buffering, and precache; disables trilinear, detail textures, and 32-bit textures. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | SGL fallback policy | [C:274-284](../res/System/0/Default.ini#L274) | SGL disables volumetrics, shiny surfaces, high-detail actors, detail textures, and vertex lighting while retaining translucency/coronas. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Renderer precache hook | [R:10858-10865](../res/Ghidra_Render.c#L10858) | `URender::Precache` delegates to an unnamed viewport helper. Exact resource traversal is not yet classified. | ⬜ audit | ⬜ audit | ⬜ none |

## Scene frames, cameras, projection, and recursion

| Done | Feature / behavior | Primary evidence | Observable semantics | Classic | Modern | Verification |
|---|---|---|---|---|---|---|
| [ ] | Master-frame construction | [R:9243-9372](../res/Ghidra_Render.c#L9243) | Allocates a scene node and full-screen span buffer, computes render coordinates, and initializes the camera’s region from the world model. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Camera render coordinates | [E:27305-27464](../res/Ghidra_Engine.c#L27305) | `ComputeRenderCoords` builds orthographic axis bases for modes `0xd`/`0xe`/`0xf`; otherwise applies the camera rotator, stores its transpose, then computes render size. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Camera region/zone selection | [R:9350-9369](../res/Ghidra_Render.c#L9350), [E:114126-114194](../res/Ghidra_Engine.c#L114126) | `PointRegion` traverses BSP plane sides, returns leaf and zone byte, and falls back to the supplied zone actor when the model has no zone actor. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Child scene frames | [R:9158-9176](../res/Ghidra_Render.c#L9158) | Child frames preserve parent, span buffer, level, zone/leaf, mirror scalar, plane, coordinates, and optional screen bounds through `FUN_10b20790`. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Recursive child rendering | [R:2185-2251](../res/Ghidra_Render.c#L2185), [R:1938-2050](../res/Ghidra_Render.c#L1938) | Both drawing and occlusion recurse through the scene-node child chain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Perspective projection | [R:3740-3887](../res/Ghidra_Render.c#L3740) | Transforms world point into camera axes, protects near-zero depth, projects by focal scale, reports visible only beyond depth `1`, and optionally returns reciprocal depth scale. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Orthographic projections | [R:3754-3798](../res/Ghidra_Render.c#L3754) | Modes `0xd`/`0xe`/`0xf` use axis-aligned scale/offset formulas and always report projected. Exact editor-mode names are uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Screen deprojection | [R:1352-1402](../res/Ghidra_Render.c#L1352) | Orthographic modes map screen coordinates back onto their corresponding world plane; other modes return the camera origin and failure. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Bound/frustum/span visibility | [R:3220-3737](../res/Ghidra_Render.c#L3220) | `BoundVisible` projects transformed bounds and tests them against the frame and optional span buffer. Exact edge conventions need focused classification. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Master-frame lifetime | [R:9246-9299](../res/Ghidra_Render.c#L9246), [R:6438-6447](../res/Ghidra_Render.c#L6438) | First master frame snapshots temporary arenas; `FinishMasterFrame` delegates cleanup/finalization to `FUN_10b20680`. | ⬜ audit | ⬜ audit | ⬜ none |

## World BSP traversal and visibility

| Done | Feature / behavior | Primary evidence | Observable semantics | Classic | Modern | Verification |
|---|---|---|---|---|---|---|
| [ ] | World render ordering | [R:3911-3999](../res/Ghidra_Render.c#L3911) | `DrawWorld` runs occlusion first, then drawing, then optional player `RenderOverlays`, and finally restores temporary arenas. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Model-node precondition | [R:1949-1955](../res/Ghidra_Render.c#L1949), [R:2207-2213](../res/Ghidra_Render.c#L2207) | Occlusion/drawing assert that the world model has BSP nodes. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Dynamics cache sizing | [R:3154-3204](../res/Ghidra_Render.c#L3154) | Point and per-node dynamics caches grow to model point/node counts; new node entries are zeroed and points stamped. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Surface/leaf light lists | [R:1956-2034](../res/Ghidra_Render.c#L1956) | Occlusion allocates surface and leaf light-link arrays, attaches them to saved surfaces/leaves, then clears touched dynamic entries after traversal. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | BSP traversal hard limit | [R:6715-6726](../res/Ghidra_Render.c#L6715) | `OccludeBsp` asserts at more than `0x10000` nodes and returns early for zero nodes. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Span-buffer occlusion | [R:6452-9064](../res/Ghidra_Render.c#L6452), [R:4681-4837](../res/Ghidra_Render.c#L4681) | BSP polygons are raster-clipped into span buffers; opaque cases update the remaining visibility while non-occluding cases copy without consuming it. Exact flag mapping is partly uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Screen-box visibility | [R:14059-14093](../res/Ghidra_Render.c#L14059) | A box is visible when any scanline span overlaps its horizontal interval within the clamped vertical interval. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Span-buffer merge/copy lifecycle | [R:9182-9238](../res/Ghidra_Render.c#L9182), [R:9936-10127](../res/Ghidra_Render.c#L9936) | Span indexes can be copied into an arena and merged; BSP traversal releases temporary buffers after use. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Sky-zone path | [R:6730-6858](../res/Ghidra_Render.c#L6730) | Occlusion queries/asserts a sky-zone actor and can submit a separately transformed scene through the render-device path. Field semantics remain uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Portal/mirror child views | [R:6452-9064](../res/Ghidra_Render.c#L6452), [R:9158-9176](../res/Ghidra_Render.c#L9158) | BSP traversal maintains per-zone span buffers and constructs child frames with plane/coordinates/mirror data. Exact polyflag-to-portal/mirror mapping needs flag-name proof. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Zone-side selection | [R:6810-6840](../res/Ghidra_Render.c#L6810), [E:114126-114194](../res/Ghidra_Engine.c#L114126) | Traversal indexes zone state from BSP side/region information; zone zero is representable and has a fallback actor. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Visible-surface query | [R:15099-15110](../res/Ghidra_Render.c#L15099) | `GetVisibleSurfs` is an exported viewport-to-surface-list query; the decompiler body is a small delegate and its helper remains to classify. | ⬜ audit | ⬜ audit | ⬜ none |

## BSP surfaces, textures, and decals

| Done | Feature / behavior | Primary evidence | Observable semantics | Classic | Modern | Verification |
|---|---|---|---|---|---|---|
| [ ] | Texture resolution/fallback | [R:2396-2423](../res/Ghidra_Render.c#L2396) | Each BSP surface resolves its texture, calls its update hook, follows a replacement at slot `0x24` when present, otherwise uses the level default texture. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Automatic U panning | [R:2430-2463](../res/Ghidra_Render.c#L2430) | Surface flag `0x200` adds `zone TexUPanSpeed × LevelInfo time × 8960`, masks the accumulator, then scales by `1/256`. Field names are supported by the LevelInfo assertion but still require property-layout cross-reference. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Automatic V panning | [R:2464-2497](../res/Ghidra_Render.c#L2464) | Surface flag `0x400` applies the analogous V-zone speed/time calculation. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Wavy texture coordinates | [R:2498-2532](../res/Ghidra_Render.c#L2498) | Surface flag `0x2000` adds two time-varying sine/cosine offsets to U/V. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Texture coordinate basis | [R:2554-2600](../res/Ghidra_Render.c#L2554) | Surface texture base, U vector, and V vector are loaded from model vectors, transformed to camera space, and passed to the device surface draw. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Macro/detail texture attachments | [R:2540-2553](../res/Ghidra_Render.c#L2540) | Up to two auxiliary textures are resolved and locked around a surface draw, gated by client/device state and polyflags. Exact roles need property-name proof. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Debug view modes | [R:2601-2707](../res/Ghidra_Render.c#L2601) | Several viewport modes override BSP color using zone/surface indices, HSV, or texture palette color and set an override flag. Exact mode labels are uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Surface hit proxies | [R:2710-2724](../res/Ghidra_Render.c#L2710) | With hit testing active, BSP draws push a surface hit proxy before device submission and pop it afterward. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Decal enable/gates | [C:94](../res/System/0/Default.ini#L94), [R:2741-2753](../res/Ghidra_Render.c#L2741) | Shipped client enables decals; drawing is gated by device/editor state and a surface exclusion flag. Exact flag name is uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Decal clipping | [R:4204-4598](../res/Ghidra_Render.c#L4204) | Starts from four decal corners, clips them against saved BSP polygon edge planes, transforms survivors, projects them, and emits zero when fully clipped. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Decal opacity/style | [R:4284-4294](../res/Ghidra_Render.c#L4284), [R:2802-2823](../res/Ghidra_Render.c#L2802) | Decal alpha is derived from actor scale-glow/byte opacity and clamped to `[0,1]`; style selects different device flags. Exact style enum mapping is uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Decal surface filtering/batching | [R:2754-2840](../res/Ghidra_Render.c#L2754) | A decal can restrict itself to saved-poly IDs; adjacent decals sharing a texture keep it locked across submissions. | ⬜ audit | ⬜ audit | ⬜ none |

## Lighting and volumetrics

| Done | Feature / behavior | Primary evidence | Observable semantics | Classic | Modern | Verification |
|---|---|---|---|---|---|---|
| [ ] | Global light color | [R:5732-5779](../res/Ghidra_Render.c#L5732) | Converts actor hue/saturation with `FGetHSV`, dispatches one of up to ten brightness/effect helpers, then clamps scalar brightness to `[0,1]`. Effect identities remain uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Static surface lighting | [R:2577-2583](../res/Ghidra_Render.c#L2577) | A surface with a valid light-map index in the applicable render mode asks `GLightManager` to prepare lighting coordinates before device submission. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Dynamic surface lights | [R:1938-2048](../res/Ghidra_Render.c#L1938), [R:6452-9064](../res/Ghidra_Render.c#L6452) | Dynamic lights are linked to touched BSP surfaces during occlusion and supplied to the light manager during draw. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Mesh vertex lighting | [R:10130-10836](../res/Ghidra_Render.c#L10130), [R:15113-16346](../res/Ghidra_Render.c#L15113) | Mesh paths obtain a light-manager mode, shade transformed vertices, then batch triangles by texture/polyflags. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Authored mesh opacity | [R:15316-15340](../res/Ghidra_Render.c#L15316) | `DrawLodMesh` changes render flags when actor opacity at offset `0x194` is below `1`, and stores that opacity on transformed vertices later in the path. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Volumetric feature gate | [C:218](../res/System/0/Default.ini#L218), [C:257](../res/System/0/Default.ini#L257), [R:5822-6214](../res/Ghidra_Render.c#L5822) | Volumetrics are enabled for shipped software/D3D configurations; per-leaf collection is a distinct renderer path. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Leaf volumetric culling | [R:5839-5935](../res/Ghidra_Render.c#L5839) | Static volumetric actors are stamp-deduplicated, rejected outside radius/frustum, transformed into view space, and linked for the leaf. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Dynamic volumetric links | [R:5937-6214](../res/Ghidra_Render.c#L5937) | Dynamic leaf-light links with a nonzero volumetric marker are stamp-deduplicated and prepended with transformed position/intensity data. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Corona/light sprite cache | [R:2852-3137](../res/Ghidra_Render.c#L2852) | Up to 32 cached lights decay with elapsed time; visible leaf and dynamic lights are updated and rendered as screen-space textured quads after depth/radius tests. Property identities are partly uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Device-specific lighting exclusions | [C:276-284](../res/System/0/Default.ini#L276), [R:6452-9064](../res/Ghidra_Render.c#L6452) | Shipped SGL disables volumetric and vertex lighting; render traversal also gates light work on client/device bits. Exact bit-to-config binding needs device decompilation. | ⬜ audit | ⬜ audit | ⬜ none |

## Actors, meshes, sprites, and attachments

| Done | Feature / behavior | Primary evidence | Observable semantics | Classic | Modern | Verification |
|---|---|---|---|---|---|---|
| [ ] | Actor eligibility scan | [R:11984-12103](../res/Ghidra_Render.c#L11984) | `SetupDynamics` scans level actors, applies hidden/editor/owner/distance/vertical-range visibility gates, and excludes the supplied actor. Several field names remain uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Actor render iterators | [R:12104-12470](../res/Ghidra_Render.c#L12104) | Actors with custom render iterators construct/cache the iterator, enumerate child actors, and create dynamic sprite records; grouped actors create a parent containing child records. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Dynamic-light actor discovery | [R:12472-12610](../res/Ghidra_Render.c#L12472) | Actors with nonzero light type/brightness/radius are frustum/radius tested and inserted into dynamic-light caches, with a volumetric-capability marker. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Draw-order buckets | [R:2227-2395](../res/Ghidra_Render.c#L2227), [R:2841-2850](../res/Ghidra_Render.c#L2841) | Saved surfaces are split into three lists; one is reversed, one sorted, and actor sprites are drawn in multiple passes based on style/opacity and BSP plane relation. Exact bucket names are uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Actor dispatch | [R:4640-4678](../res/Ghidra_Render.c#L4640), [R:13034-13265](../res/Ghidra_Render.c#L13034) | Public `DrawActor` creates a dynamic sprite then `DrawActorSprite` dispatches particle, mesh, or textured-sprite paths based on actor draw type. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Actor hit proxies | [R:13121-13135](../res/Ghidra_Render.c#L13121), [R:13599-13631](../res/Ghidra_Render.c#L13599) | Actor draws push the hit actor while hit testing and pop after all actor/debug geometry. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Textured sprite scaling/fade | [R:13134-13239](../res/Ghidra_Render.c#L13134) | Sprite size uses texture dimensions, actor draw scale and glow; one draw type walks animation frames and computes a life/default-based fade. Enum names are uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Mesh path selection | [R:10130-10836](../res/Ghidra_Render.c#L10130) | `DrawMesh` handles classic mesh layout and delegates recognized LOD meshes to `DrawLodMesh`, always adding base mesh flag `0x4000`. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Mesh camera clipping/backface | [R:10277-10490](../res/Ghidra_Render.c#L10277) | Transformed vertices carry clip codes; triangle admission rejects common outside planes and applies a backface test unless flags request two-sided/environment behavior. Exact flags need enum proof. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Mesh texture resolution | [R:10574-10648](../res/Ghidra_Render.c#L10574), [R:15917-15970](../res/Ghidra_Render.c#L15917) | Up to 16 mesh textures are resolved, replacement textures followed, locked before drawing, and unlocked afterward; actor skin can override mesh/default texture. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Mesh opacity blending | [R:15316-15340](../res/Ghidra_Render.c#L15316) | Opacity below `1` forces additional blend/polyflags (`0x10004004` in decompiler output). Exact public flag names require header/default correlation. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | LOD selection | [R:15113-16346](../res/Ghidra_Render.c#L15113) | `DrawLodMesh` computes screen-size-dependent vertex/triangle selection from mesh LOD tables and actor/camera scale, then shades/batches the chosen geometry. Exact thresholds remain to decode. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Attached/weapon meshes | [R:13266-13528](../res/Ghidra_Render.c#L13266) | Actor-sprite drawing temporarily swaps mesh/skin/transform state to draw attached meshes, weapon-like overlays, and alternate meshes, then restores actor state. Field names are uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Orthographic mesh fallback | [R:10230-10243](../res/Ghidra_Render.c#L10230) | Orthographic/editor modes replace the supplied actor transform with identity camera coordinates. | ⬜ audit | ⬜ audit | ⬜ none |

## Particles, overlays, debug drawing, and instrumentation

| Done | Feature / behavior | Primary evidence | Observable semantics | Classic | Modern | Verification |
|---|---|---|---|---|---|---|
| [ ] | Particle update-before-draw | [R:13800-13848](../res/Ghidra_Render.c#L13800) | Particle actors call `AParticleFX::Update(0.0)` and skip drawing when it returns false. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Particle render modes | [R:13849-13956](../res/Ghidra_Render.c#L13849) | A particle mode byte dispatches five renderer helper paths (`0`–`4`); unsupported values assert. Concrete mode names are uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Particle local/world variants | [R:13957-14031](../res/Ghidra_Render.c#L13957) | A particle flag switches between two helper variants; both use frame/viewport scale when building submission state. Exact flag meaning is uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Player overlays | [R:3955-3980](../res/Ghidra_Render.c#L3955) | Outside editor, an eligible player/owned actor with the overlay bit invokes its `RenderOverlays` event after world drawing under a temporary hack flag. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Debug primitives | [R:4908-5433](../res/Ghidra_Render.c#L4908), [R:9374-9920](../res/Ghidra_Render.c#L9374), [R:14842-15020](../res/Ghidra_Render.c#L14842) | Renderer exports cylinder, two box variants, and circle drawing through line/primitive device paths. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Actor debug axes/bounds | [R:13495-13618](../res/Ghidra_Render.c#L13495) | Editor/debug flags draw actor axes and collision/bounds geometry with state-dependent colors. Exact flags and draw modes are uncertain. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Hit-testing stack semantics | [E:33519-33569](../res/Ghidra_Engine.c#L33519), [E:72038-72108](../res/Ghidra_Engine.c#L72038) | Viewport hit proxies are serialized onto a bounded hit stack and popped after draws; actor and BSP paths use it. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Pre/post render hooks | [R:12768-12817](../res/Ghidra_Render.c#L12768), [R:16907-16923](../res/Ghidra_Render.c#L16907) | `PreRender` and `PostRender` are exported hooks that delegate to internal helpers; exact device/state work remains to classify. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Runtime renderer commands | [R:5561-5602](../res/Ghidra_Render.c#L5561) | `URender::Exec` is an exported console-command handler delegating to `FUN_10b1f240`; accepted commands remain to enumerate from the helper. | ⬜ audit | ⬜ audit | ⬜ none |
| [ ] | Render statistics | [R:16412-16835](../res/Ghidra_Render.c#L16412) | `DrawStats` formats and draws renderer counters/timings; `ShowStat` is its formatted text helper. | ⬜ audit | ⬜ audit | ⬜ none |

## Function-coverage appendix

The lists below are the exact top-level-body inventory. `L…` is the decompiler line; address-bearing Ghidra names retain their embedded address. Classification and feature mapping apply to every entry in the subsection.

### Named renderer and support methods — 69

Classification: behavior-bearing exported/internal method unless described as a constructor, assignment, assertion, allocator, or destructor. Family is evident from the owning type/name; overloads remain separate bodies.

`L1342 URender::Destroy`; `L1355 URender::Deproject`; `L1938 URender::OccludeFrame`; `L2054 FSpanBuffer::Release`; `L2185 URender::DrawFrame`; `L3156 URender::AllocDynamicsCache`; `L3209 URender::StaticConstructor`; `L3223 URender::BoundVisible`.
`L3744 URender::Project`; `L3894 FSpanBuffer::operator=`; `L3913 URender::DrawWorld`; `L4208 URender::ClipDecal`; `L4642 URender::DrawActor`; `L4688 FSpanBuffer::CopyFromRaster`; `L4842 FSpanBuffer::GetValidRange`; `L4911 URender::DrawCylinder`.
`L5438 URender::StaticClass`; `L5449 FSpanBuffer::FSpanBuffer`; `L5563 URender::Exec`; `L5607 URender::URender`; `L5736 URender::GlobalLighting`; `L5799 FLightManagerBase::FLightManagerBase`; `L5822 URender::LeafVolumetricLighting`; `L6219 FSpanBuffer::AssertValid`.
`L6440 URender::FinishMasterFrame`; `L6454 URender::OccludeBsp`; `L9069 URender::operator_new`; `L9161 URender::CreateChildFrame`; `L9182 FSpanBuffer::CopyIndexFrom`; `L9246 URender::CreateMasterFrame`; `L9377 URender::DrawBox`; `L9925 URender::DrawBox`.
`L9938 FSpanBuffer::MergeWith`; `L10134 URender::DrawMesh`; `L10842 myclass::operator=`; `L10860 URender::Precache`; `L11320 FSpanBuffer::AllocIndex`; `L11368 FSpanBuffer::AllocIndexForScreen`; `L11423 FSpanBuffer::CopyFromRasterUpdate`; `L11725 URender::operator_new`.
`L11760 FLightManagerBase::operator=`; `L11986 URender::SetupDynamics`; `L12616 URender::operator=`; `L12770 URender::PostRender`; `L12823 URender::URender`; `L12866 FSpanBuffer::FSpanBuffer`; `L12962 FLightManagerBase::FLightManagerBase`; `L13036 URender::DrawActorSprite`.
`L13638 myclass::timtim`; `L13803 URender::DrawParticleSystem`; `L14062 FSpanBuffer::BoxIsVisible`; `L14151 URender::ClipBspSurf`; `L14560 myclass::myclass`; `L14710 myclass::xyzzy`; `L14724 FSpanBuffer::AssertEmpty`; `L14845 URender::DrawCircle`.
`L15025 URender::~URender`; `L15061 FSpanBuffer::AssertNotEmpty`; `L15113 URender::DrawLodMesh`; `L15101 URender::GetVisibleSurfs`; `L16351 URender::Init`; `L16414 URender::DrawStats`; `L16840 URender::ShowStat`; `L16909 URender::PreRender`.
`L16969 myclass::myclass`; `L16981 URender::InternalConstructor`; `L16997 URender::Dynamic`; `L17008 FSpanBuffer::AssertGoodEnough`; `L42986 type_info::~type_info`.

### Unnamed implementations — 188

Classification: behavior-bearing or support implementation requiring caller classification. Family map by address range: `10b01760–10b0ff00` low-level renderer/raster/mesh submission; `10b13970–10b1cdb0` containers, particles, geometry and sorting; `10b1d020–10b27ba0` object lifecycle, frame/world/BSP and lighting; `10b2a9a0–10b34e00` math, span/dynamic-item, actor submission and support; `10b35160–10b3535f` compiler/module runtime. An item is not parity-complete merely because it appears in a mapped range.

`L17041 FUN_10b01760`; `L17050 FUN_10b017c0`; `L17059 FUN_10b017f0`; `L17073 FUN_10b01830`; `L17081 FUN_10b01850`; `L17158 FUN_10b01990`; `L17173 FUN_10b019c0`; `L17182 FUN_10b019e0`.
`L17191 FUN_10b01a00`; `L17199 FUN_10b01a20`; `L17208 FUN_10b01a40`; `L17218 FUN_10b01a80`; `L17226 FUN_10b01aa0`; `L17259 FUN_10b01b70`; `L17273 FUN_10b01b90`; `L17281 FUN_10b01bc0`.
`L17405 FUN_10b01ed0`; `L17525 FUN_10b021c0`; `L17537 FUN_10b023a0`; `L17717 FUN_10b027d0`; `L17766 FUN_10b028e0`; `L17807 FUN_10b02970`; `L17903 FUN_10b02b10`; `L18265 FUN_10b03430`.
`L18422 FUN_10b037c0`; `L18568 FUN_10b03bc0`; `L18710 FUN_10b03f80`; `L18855 FUN_10b04360`; `L18994 FUN_10b04700`; `L19133 FUN_10b04aa0`; `L19268 FUN_10b04ea0`; `L19415 FUN_10b05330`.
`L19558 FUN_10b05710`; `L19703 FUN_10b05ae0`; `L19813 FUN_10b05dc0`; `L19924 FUN_10b06090`; `L20035 FUN_10b06380`; `L20043 FUN_10b06390`; `L20064 FUN_10b06410`; `L20079 FUN_10b06590`.
`L20132 FUN_10b066d0`; `L20174 FUN_10b06810`; `L20246 FUN_10b06920`; `L20798 FUN_10b077c0`; `L20812 FUN_10b077f0`; `L21959 FUN_10b095d7`; `L22041 FUN_10b096c0`; `L22094 FUN_10b097b0`.
`L22155 FUN_10b098f0`; `L22613 FUN_10b0a650`; `L22661 FUN_10b0c990`; `L22673 FUN_10b0c9b0`; `L22705 FUN_10b0ca70`; `L22831 FUN_10b0d030`; `L22935 FUN_10b0d320`; `L22969 FUN_10b0d3c0`.
`L23770 FUN_10b0e810`; `L23802 FUN_10b0e900`; `L23814 FUN_10b0e920`; `L24540 FUN_10b0ff00`; `L25759 FUN_10b13970`; `L25790 FUN_10b139e0`; `L25856 FUN_10b13be0`; `L25872 FUN_10b13c50`.
`L25880 FUN_10b13c70`; `L25918 FUN_10b13d10`; `L25995 FUN_10b13e90`; `L26056 FUN_10b14050`; `L26099 FUN_10b14110`; `L26129 FUN_10b14200`; `L26159 FUN_10b142f0`; `L26189 FUN_10b143e0`.
`L26217 FUN_10b144d0`; `L26230 FUN_10b14510`; `L26277 FUN_10b145f0`; `L26310 FUN_10b14710`; `L26323 FUN_10b14750`; `L26356 FUN_10b14870`; `L26381 FUN_10b14930`; `L26399 FUN_10b14980`.
`L26446 FUN_10b14a60`; `L26493 FUN_10b14b40`; `L26540 FUN_10b14c20`; `L26659 FUN_10b14e40`; `L26778 FUN_10b15090`; `L27143 FUN_10b15830`; `L27326 FUN_10b15b50`; `L27525 FUN_10b16950`.
`L27535 FUN_10b16980`; `L27545 FUN_10b169b0`; `L27555 FUN_10b169e0`; `L27565 FUN_10b16a10`; `L27599 FUN_10b16ab0`; `L27651 FUN_10b16c70`; `L27778 FUN_10b17330`; `L27789 FUN_10b17350`.
`L27992 FUN_10b17ce0`; `L28156 FUN_10b184d0`; `L28426 FUN_10b18e80`; `L28609 FUN_10b194c0`; `L29220 FUN_10b1b330`; `L29335 FUN_10b1b7d0`; `L29648 FUN_10b1c490`; `L29729 FUN_10b1c6c0`.
`L29811 FUN_10b1c910`; `L29893 FUN_10b1cb60`; `L29975 FUN_10b1cdb0`; `L30055 FUN_10b1d020`; `L30125 FUN_10b1d117`; `L30175 FUN_10b1d5c0`; `L30236 FUN_10b1d6a0`; `L30244 FUN_10b1d6b0`.
`L30326 FUN_10b1d810`; `L30379 FUN_10b1d900`; `L30576 FUN_10b1dc50`; `L30853 FUN_10b1ed30`; `L30912 FUN_10b1edf0`; `L30993 FUN_10b1ef80`; `L31113 FUN_10b1f240`; `L31396 FUN_10b1f8f0`.
`L31815 FUN_10b20370`; `L31963 FUN_10b20680`; `L32034 FUN_10b20790`; `L32339 FUN_10b20df0`; `L32501 FUN_10b21030`; `L33022 FUN_10b22090`; `L33173 FUN_10b22370`; `L33226 FUN_10b22480`.
`L34958 FUN_10b24f30`; `L35129 FUN_10b25240`; `L35271 FUN_10b254f0`; `L36276 FUN_10b26f10`; `L36334 FUN_10b27060`; `L36730 FUN_10b27ba0`; `L36848 FUN_10b2a9a0`; `L36887 FUN_10b2aa40`.
`L36923 FUN_10b2aab0`; `L36939 FUN_10b2ab20`; `L36986 FUN_10b2ac00`; `L37033 FUN_10b2ace0`; `L37081 FUN_10b2adc0`; `L37184 FUN_10b2afa0`; `L37300 FUN_10b2b200`; `L37361 FUN_10b2b3c0`.
`L37407 FUN_10b2b4d0`; `L37447 FUN_10b2b5c0`; `L37986 FUN_10b2c4f0`; `L38182 FUN_10b2c8b0`; `L38427 FUN_10b2cf80`; `L38788 FUN_10b2e4d0`; `L38858 FUN_10b2e5b0`; `L38933 FUN_10b2e6c0`.
`L38978 FUN_10b2e750`; `L39020 FUN_10b2e7c0`; `L39054 FUN_10b2e830`; `L39250 FUN_10b2eb90`; `L39411 FUN_10b2ee30`; `L39622 FUN_10b2f150`; `L39691 FUN_10b2f230`; `L39744 FUN_10b2f2f0`.
`L39803 FUN_10b2f3e0`; `L39880 FUN_10b2f540`; `L39959 FUN_10b2fb30`; `L40469 FUN_10b30790`; `L40651 FUN_10b30c70`; `L40824 FUN_10b31160`; `L41216 FUN_10b31c70`; `L41505 FUN_10b322c0`.
`L41566 FUN_10b323f0`; `L41728 FUN_10b32850`; `L42390 FUN_10b338f0`; `L42431 FUN_10b33980`; `L42491 FUN_10b34a60`; `L42509 FUN_10b34ab0`; `L42558 FUN_10b34bc0`; `L42567 FUN_10b34be0`.
`L42649 FUN_10b34dc0`; `L42658 FUN_10b34de0`; `L42667 FUN_10b34e00`; `L42675 FUN_10b35160`; `L42701 FUN_10b351c8`; `L42715 FUN_10b351e0`; `L42739 FUN_10b3523e`; `L42751 FUN_10b35254`.
`L42764 FUN_10b35280`; `L42775 FUN_10b3529d`; `L42830 FUN_10b35310`; `L42871 FUN_10b3535f`.

### Forwarding/duplicate thunks — 60

Classification: duplicate entry/forwarder. The duplicate target is the `FUN_…` suffix; map it to the same family and feature row as that implementation.

`L1405 thunk_FUN_10b06920`; `L2078 thunk_FUN_10b2adc0`; `L3810 thunk_FUN_10b1c910`; `L4008 thunk_FUN_10b0c9b0`; `L4038 thunk_FUN_10b0d030`; `L4142 thunk_FUN_10b13e90`; `L4602 thunk_FUN_10b13c70`; `L4827 thunk_FUN_10b14710`.
`L4897 thunk_FUN_10b169e0`; `L5460 thunk_FUN_10b1cb60`; `L5540 thunk_FUN_10b2ab20`; `L5577 thunk_FUN_10b142f0`; `L5784 thunk_FUN_10b144d0`; `L5809 thunk_FUN_10b017c0`; `L5975 thunk_FUN_10b2afa0`; `L6091 thunk_FUN_10b0ca70`.
`L6229 thunk_FUN_10b30c70`; `L6400 thunk_FUN_10b2b4d0`; `L8081 thunk_FUN_10b2ace0`; `L8105 thunk_FUN_10b03430`; `L8260 thunk_FUN_10b14a60`; `L8284 thunk_FUN_10b0d3c0`; `L9078 thunk_FUN_10b097b0`; `L9137 thunk_FUN_10b14510`.
`L9230 thunk_FUN_10b0c990`; `L9603 thunk_FUN_10b15b50`; `L9803 thunk_FUN_10b14c20`; `L10870 thunk_FUN_10b15090`; `L11238 thunk_FUN_10b1c490`; `L11602 thunk_FUN_10b14e40`; `L11738 thunk_FUN_10b2ac00`; `L11775 thunk_FUN_10b30790`.
`L11957 thunk_FUN_10b16950`; `L11967 thunk_FUN_10b2aab0`; `L12480 thunk_FUN_10b145f0`; `L12513 thunk_FUN_10b34ab0`; `L12536 thunk_FUN_10b1cdb0`; `L12739 thunk_FUN_10b13970`; `L12780 thunk_FUN_10b14050`; `L12943 thunk_FUN_10b13be0`.
`L12972 thunk_FUN_10b14870`; `L12997 thunk_FUN_10b2aa40`; `L13724 thunk_FUN_10b14110`; `L13754 thunk_FUN_10b14200`; `L13782 thunk_FUN_10b13c50`; `L13790 thunk_FUN_10b16980`; `L13964 thunk_FUN_10b13d10`; `L14041 thunk_FUN_10b14930`.
`L14097 thunk_FUN_10b14750`; `L14130 thunk_FUN_10b34a60`; `L14570 thunk_FUN_10b16a10`; `L14606 thunk_FUN_10b1c6c0`; `L14686 thunk_FUN_10b14980`; `L14734 thunk_FUN_10b02970`; `L14828 thunk_FUN_10b017f0`; `L16300 thunk_FUN_10b2a9a0`.
`L16339 thunk_FUN_10b169b0`; `L16678 thunk_FUN_10b323f0`; `L16878 thunk_FUN_10b143e0`; `L17018 thunk_FUN_10b14b40`.

### Module/compiler-runtime bodies — 9

Classification/family: non-rendering compiler/module glue unless a future call-path proves otherwise.

`L4887 _DllMain_12`; `L42787 _CxxThrowException`; `L42799 ftol`; `L42815 __allmul`; `L42852 __allshl`; `L42915 entry`; `L42963 terminate`; `L42975 __dllonexit`; `L42997 initterm`.

### Catch handlers — 156

Classification/family: compiler-generated exception handlers; mapped to the immediately preceding address-family implementation, not independent renderer behavior.

`L17695 Catch_10b0279c`; `L17707 Catch_10b027b3`; `L17785 Catch_10b02934`; `L17797 Catch_10b02948`; `L18241 Catch_10b033d4`; `L18253 Catch_10b033eb`; `L20222 Catch_10b068e9`; `L20234 Catch_10b068fd`.
`L20776 Catch_10b07790`; `L20788 Catch_10b077a4`; `L21847 Catch_10b07b5a`; `L21859 Catch_10b07b74`; `L21869 Catch_10b07cde`; `L21881 Catch_10b07cf8`; `L21891 Catch_10b0852a`; `L21903 Catch_10b08547`.
`L21913 Catch_10b08952`; `L21925 Catch_10b0896c`; `L21935 Catch_10b0959e`; `L21947 Catch_10b095bb`; `L21975 Catch_10b09610`; `L21987 Catch_10b0962d`; `L21997 Catch_10b09649`; `L22009 Catch_10b09663`.
`L22019 Catch_10b0967f`; `L22031 Catch_10b0969c`; `L22072 Catch_10b0972d`; `L22084 Catch_10b09741`; `L22591 Catch_10b0a619`; `L22603 Catch_10b0a62d`; `L22639 Catch_10b0a6bd`; `L22651 Catch_10b0a6d1`.
`L23748 Catch_10b0e5bc`; `L23760 Catch_10b0e5d9`; `L24509 Catch_10b0fe9b`; `L24521 Catch_10b0feb8`; `L25710 Catch_10b120af`; `L25722 Catch_10b120c9`; `L25732 Catch_10b12489`; `L25744 Catch_10b124a6`.
`L26251 Catch_10b1456d`; `L26263 Catch_10b14581`; `L26420 Catch_10b149dd`; `L26432 Catch_10b149f1`; `L26467 Catch_10b14abd`; `L26479 Catch_10b14ad1`; `L26514 Catch_10b14b9d`; `L26526 Catch_10b14bb1`.
`L27301 Catch_10b15b08`; `L27313 Catch_10b15b1c`; `L30153 Catch_10b1d17d`; `L30165 Catch_10b1d191`; `L30214 Catch_10b1d664`; `L30226 Catch_10b1d678`; `L30304 Catch_10b1d7df`; `L30316 Catch_10b1d7f3`.
`L30357 Catch_10b1d8c3`; `L30369 Catch_10b1d8d7`; `L30498 Catch_10b1da10`; `L30510 Catch_10b1da24`; `L30525 Catch_10b1db97`; `L30537 Catch_10b1dbab`; `L30554 Catch_10b1dbff`; `L30566 Catch_10b1dc13`.
`L30831 Catch_10b1eceb`; `L30843 Catch_10b1ecff`; `L30888 Catch_10b1edaa`; `L30900 Catch_10b1edbe`; `L30969 Catch_10b1ef4c`; `L30981 Catch_10b1ef60`; `L31091 Catch_10b1f20c`; `L31103 Catch_10b1f220`.
`L31372 Catch_10b1f8b9`; `L31384 Catch_10b1f8cd`; `L31791 Catch_10b20300`; `L31803 Catch_10b20314`; `L31941 Catch_10b20645`; `L31953 Catch_10b20659`; `L32009 Catch_10b2075f`; `L32021 Catch_10b20773`.
`L32315 Catch_10b20db9`; `L32327 Catch_10b20dcd`; `L32475 Catch_10b20fe7`; `L32487 Catch_10b20ffb`; `L32998 Catch_10b22052`; `L33010 Catch_10b2206f`; `L34846 Catch_10b22b1e`; `L34858 Catch_10b22b3b`.
`L34868 Catch_10b23a23`; `L34880 Catch_10b23a40`; `L34890 Catch_10b23d87`; `L34902 Catch_10b23da4`; `L34912 Catch_10b24462`; `L34924 Catch_10b2447f`; `L34934 Catch_10b24ef3`; `L34946 Catch_10b24f10`.
`L35105 Catch_10b250c6`; `L35117 Catch_10b250da`; `L35246 Catch_10b254a6`; `L35258 Catch_10b254ba`; `L36232 Catch_10b265aa`; `L36244 Catch_10b265c4`; `L36254 Catch_10b26ed1`; `L36266 Catch_10b26eee`.
`L36706 Catch_10b27b20`; `L36718 Catch_10b27b34`; `L36826 Catch_10b27d86`; `L36838 Catch_10b27d9a`; `L36960 Catch_10b2ab7d`; `L36972 Catch_10b2ab91`; `L37007 Catch_10b2ac5d`; `L37019 Catch_10b2ac71`.
`L37055 Catch_10b2ad3d`; `L37067 Catch_10b2ad51`; `L37964 Catch_10b2c49a`; `L37976 Catch_10b2c4b7`; `L38160 Catch_10b2c85d`; `L38172 Catch_10b2c874`; `L38403 Catch_10b2cf4a`; `L38415 Catch_10b2cf5e`.
`L38766 Catch_10b2da58`; `L38778 Catch_10b2da75`; `L38832 Catch_10b2e547`; `L38844 Catch_10b2e55b`; `L38911 Catch_10b2e67c`; `L38923 Catch_10b2e690`; `L38956 Catch_10b2e714`; `L38968 Catch_10b2e728`.
`L39228 Catch_10b2eb59`; `L39240 Catch_10b2eb6d`; `L39389 Catch_10b2edcb`; `L39401 Catch_10b2eddf`; `L39600 Catch_10b2f0ef`; `L39612 Catch_10b2f103`; `L39669 Catch_10b2f1fc`; `L39681 Catch_10b2f210`.
`L39722 Catch_10b2f29f`; `L39734 Catch_10b2f2b3`; `L39781 Catch_10b2f3a5`; `L39793 Catch_10b2f3b9`; `L39858 Catch_10b2f505`; `L39870 Catch_10b2f519`; `L39935 Catch_10b2f679`; `L39947 Catch_10b2f68d`.
`L40444 Catch_10b30716`; `L40456 Catch_10b3072d`; `L42324 Catch_10b32c93`; `L42336 Catch_10b32cad`; `L42346 Catch_10b334e8`; `L42358 Catch_10b33502`; `L42368 Catch_10b338af`; `L42380 Catch_10b338cc`.
`L42469 Catch_10b33a1d`; `L42481 Catch_10b33a31`; `L42530 Catch_10b34b0d`; `L42542 Catch_10b34b21`.

### Unwind handlers — 32

Classification/family: compiler-generated cleanup handlers; mapped to their owning function’s exception path, not independent renderer behavior.

`L43008 Unwind_10b36480`; `L43021 Unwind_10b36489`; `L43034 Unwind_10b36492`; `L43047 Unwind_10b364c4`; `L43058 Unwind_10b36530`; `L43069 Unwind_10b36538`; `L43080 Unwind_10b36560`; `L43091 Unwind_10b36572`.
`L43102 Unwind_10b3657a`; `L43113 Unwind_10b3658c`; `L43124 Unwind_10b3659e`; `L43135 Unwind_10b365a6`; `L43146 Unwind_10b365ae`; `L43157 Unwind_10b365b6`; `L43168 Unwind_10b365be`; `L43179 Unwind_10b365d0`.
`L43190 Unwind_10b365d8`; `L43201 Unwind_10b365e0`; `L43212 Unwind_10b365e8`; `L43223 Unwind_10b365f0`; `L43234 Unwind_10b365f8`; `L43245 Unwind_10b36600`; `L43256 Unwind_10b36680`; `L43267 Unwind_10b36688`.
`L43278 Unwind_10b36690`; `L43289 Unwind_10b366a2`; `L43300 Unwind_10b366e0`; `L43313 Unwind_10b36775`; `L43324 Unwind_10b367a5`; `L43335 Unwind_10b36910`; `L43346 Unwind_10b36960`; `L43359 Unwind_10b36969`.

Count reconciliation: **69 + 188 + 60 + 9 + 156 + 32 = 514** top-level decompiled bodies.
