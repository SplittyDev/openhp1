use std::collections::VecDeque;

use anyhow::{Context, Result};
use glam::Vec3;
use openhp1_runtime::{ActorAction, ConsoleCommandHost, ScriptRuntime};
use tracing::{info, warn};

use crate::{LoadedScene, Rotator};

pub fn initialize_runtime(scene: &mut LoadedScene) -> Result<ScriptRuntime> {
    initialize_runtime_with(scene, |_| Ok(()))
}

pub fn initialize_runtime_with(
    scene: &mut LoadedScene,
    external: impl FnMut(ActorAction) -> Result<()>,
) -> Result<ScriptRuntime> {
    let game_root = scene
        .path
        .parent()
        .and_then(|directory| directory.parent())
        .context("map path must be inside the game's Maps directory")?;
    let mut runtime = ScriptRuntime::new(game_root)?;
    initialize_runtime_after_creation(scene, &mut runtime, external, true)?;
    Ok(runtime)
}

pub fn initialize_runtime_with_console(
    scene: &mut LoadedScene,
    console: impl ConsoleCommandHost + 'static,
    in_hub_flow: bool,
    external: impl FnMut(ActorAction) -> Result<()>,
) -> Result<ScriptRuntime> {
    let game_root = scene
        .path
        .parent()
        .and_then(|directory| directory.parent())
        .context("map path must be inside the game's Maps directory")?;
    let mut runtime = ScriptRuntime::new(game_root)?;
    runtime.set_console_command_host(console);
    runtime.set_in_hub_flow(in_hub_flow);
    initialize_runtime_after_creation(scene, &mut runtime, external, true)?;
    Ok(runtime)
}

/// Registers the authored map without running game construction or script
/// events. Save restoration uses this before replacing mutable runtime state.
pub fn initialize_runtime_with_console_unstarted(
    scene: &mut LoadedScene,
    console: impl ConsoleCommandHost + 'static,
    in_hub_flow: bool,
) -> Result<ScriptRuntime> {
    let game_root = scene
        .path
        .parent()
        .and_then(|directory| directory.parent())
        .context("map path must be inside the game's Maps directory")?;
    let mut runtime = ScriptRuntime::new(game_root)?;
    runtime.set_console_command_host(console);
    runtime.set_in_hub_flow(in_hub_flow);
    initialize_runtime_after_creation(scene, &mut runtime, |_| Ok(()), false)?;
    Ok(runtime)
}

fn initialize_runtime_after_creation(
    scene: &mut LoadedScene,
    runtime: &mut ScriptRuntime,
    mut external: impl FnMut(ActorAction) -> Result<()>,
    start: bool,
) -> Result<()> {
    runtime.set_collision(scene.collision(), &scene.path)?;
    let classes = scene
        .actors
        .iter()
        .enumerate()
        .filter_map(|(actor, value)| {
            value.class.as_ref().map(|class| {
                (
                    actor,
                    value.id.package.clone(),
                    value.id.export_index,
                    class.package.clone(),
                    class.export_index,
                )
            })
        })
        .collect::<Vec<_>>();
    for &(actor, ref actor_package, actor_export, ref class_package, class_export) in &classes {
        if let Err(error) = runtime.register_actor(
            actor,
            actor_package,
            actor_export,
            class_package,
            class_export,
        ) {
            scene.actors[actor]
                .diagnostics
                .push(format!("runtime registration failed: {error}"));
        }
        runtime
            .set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))
            .with_context(|| {
                format!(
                    "could not register animation sequences for {}",
                    scene.actors[actor].name
                )
            })?;
        runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
        if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
            runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
        }
    }
    sync_runtime_pose(scene, runtime)?;
    if !start {
        return Ok(());
    }
    let game_actions = runtime.initialize_game()?;
    apply_runtime_actions_with(scene, runtime, game_actions, &mut external)?;
    let mut events = 0;
    let mut animations = 0;
    let mut deferred = 0;
    for event in [
        "PreBeginPlay",
        "BeginPlay",
        "PostBeginPlay",
        "SetInitialState",
    ] {
        for &(actor, _, _, ref package, export) in &classes {
            match runtime.dispatch_event(actor, package, export, event) {
                Ok(actions) => {
                    events += 1;
                    let applied =
                        apply_runtime_actions_with(scene, runtime, actions, &mut external)?;
                    animations += applied.0;
                    deferred += applied.1;
                }
                Err(error) => {
                    deferred += 1;
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime deferred {event}: {error}"));
                }
            }
        }
    }
    let actions = runtime.initialize_actor_bases()?;
    let applied = apply_runtime_actions_with(scene, runtime, actions, &mut external)?;
    animations += applied.0;
    deferred += applied.1;
    info!(events, animations, deferred, "initialized script runtime");
    Ok(())
}

pub fn apply_runtime_actions(
    scene: &mut LoadedScene,
    runtime: &mut ScriptRuntime,
    actions: Vec<ActorAction>,
) -> Result<(usize, usize, bool)> {
    apply_runtime_actions_with(scene, runtime, actions, |_| Ok(()))
}

pub fn apply_runtime_actions_with(
    scene: &mut LoadedScene,
    runtime: &mut ScriptRuntime,
    actions: Vec<ActorAction>,
    mut external: impl FnMut(ActorAction) -> Result<()>,
) -> Result<(usize, usize, bool)> {
    let mut animations = 0;
    let mut deferred = 0;
    let mut transformed = false;
    let mut bone_positions_changed = false;
    let mut actions = VecDeque::from(actions);
    while let Some(action) = actions.pop_front() {
        scene.ensure_runtime_actor(action.actor());
        match action {
            ActorAction::PlayAnimation {
                actor,
                sequence,
                rate,
                tween_time,
                root_motion,
            } => {
                let played = if root_motion {
                    scene.play_actor_animation_with_root_motion(
                        actor, &sequence, rate, tween_time, false,
                    )?
                } else {
                    scene.play_actor_animation_with_tween(actor, &sequence, rate, tween_time)?
                };
                if played {
                    animations += 1;
                } else if scene.animation_request_exposes_capability_gap(actor, &sequence) {
                    let target = &mut scene.actors[actor];
                    record_animation_diagnostic(
                        actor,
                        &target.name,
                        &mut target.diagnostics,
                        format!("runtime could not play animation {sequence}"),
                    );
                }
                bone_positions_changed = true;
            }
            ActorAction::LoopAnimation {
                actor,
                sequence,
                rate,
                tween_time,
                root_motion,
            } => {
                let played = if root_motion {
                    scene.play_actor_animation_with_root_motion(
                        actor, &sequence, rate, tween_time, true,
                    )?
                } else {
                    scene.loop_actor_animation_with_tween(actor, &sequence, rate, tween_time)?
                };
                if played {
                    animations += 1;
                } else if scene.animation_request_exposes_capability_gap(actor, &sequence) {
                    let target = &mut scene.actors[actor];
                    record_animation_diagnostic(
                        actor,
                        &target.name,
                        &mut target.diagnostics,
                        format!("runtime could not play animation {sequence}"),
                    );
                }
                bone_positions_changed = true;
            }
            ActorAction::RestoreAnimation {
                actor,
                sequence,
                rate,
                tween_time,
                looping,
                tween_only,
                root_motion,
                phase,
            } => {
                let played = scene.restore_actor_animation(
                    actor,
                    &sequence,
                    rate,
                    tween_time,
                    looping,
                    tween_only,
                    root_motion,
                    phase,
                )?;
                if played {
                    animations += 1;
                } else if scene.animation_request_exposes_capability_gap(actor, &sequence) {
                    let target = &mut scene.actors[actor];
                    record_animation_diagnostic(
                        actor,
                        &target.name,
                        &mut target.diagnostics,
                        format!("runtime could not restore animation {sequence}"),
                    );
                }
                bone_positions_changed = true;
            }
            ActorAction::SetAnimationFrame { actor, frame } => {
                scene.set_actor_animation_frame(actor, frame)?;
                bone_positions_changed = true;
            }
            ActorAction::AwaitAnimation { actor } => {
                scene.finish_actor_animation(actor);
                if !scene.actor_animation_playing(actor) {
                    actions.extend(runtime.animation_finished(actor)?);
                }
            }
            action @ (ActorAction::PlaySound { .. }
            | ActorAction::ModifySound { .. }
            | ActorAction::StopSound { .. }
            | ActorAction::ClientTravel { .. }
            | ActorAction::UnlockQuidditch { .. }
            | ActorAction::FinishQuidditchMatch { .. }
            | ActorAction::UpdateUrl { .. }) => external(action)?,
            ActorAction::SpawnActor {
                actor,
                name,
                class_package,
                class_export,
                class_name,
                location,
                rotation,
            } => {
                scene.spawn_actor(
                    actor,
                    name,
                    class_package.to_string(),
                    class_export,
                    class_name,
                    Vec3::from_array(location),
                    Rotator {
                        pitch: rotation[0],
                        yaw: rotation[1],
                        roll: rotation[2],
                    },
                )?;
                for diagnostic in &scene.actors[actor].diagnostics {
                    warn!(
                        actor,
                        actor_name = scene.actors[actor].name,
                        class = scene.actors[actor].class_name,
                        draw_type = scene.actors[actor].draw_type,
                        diagnostic,
                        "spawned actor capability diagnostic"
                    );
                }
                runtime
                    .set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))?;
                runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
                if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                    runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
                }
                transformed = true;
                bone_positions_changed = true;
            }
            ActorAction::SetLocation { actor, location } => {
                transformed |= scene.set_actor_location(actor, Vec3::from_array(location))?;
                bone_positions_changed = true;
            }
            ActorAction::SetRotation { actor, rotation } => {
                transformed |= scene.set_actor_rotation(
                    actor,
                    Rotator {
                        pitch: rotation[0],
                        yaw: rotation[1],
                        roll: rotation[2],
                    },
                )?;
                bone_positions_changed = true;
            }
            ActorAction::SetPrePivot { actor, pre_pivot } => {
                transformed |= scene.set_actor_pre_pivot(actor, Vec3::from_array(pre_pivot))?;
                bone_positions_changed = true;
            }
            ActorAction::SetHidden { actor, hidden } => {
                transformed |= scene.set_actor_hidden(actor, hidden)?;
            }
            ActorAction::SetDrawType { actor, draw_type } => {
                transformed |= scene.set_actor_draw_type(actor, draw_type)?;
            }
            ActorAction::SetMesh { actor, mesh } => {
                transformed |= scene.set_actor_mesh(actor, mesh)?;
                runtime
                    .set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))?;
                runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
                runtime.clear_actor_visual_bounds(actor);
                if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                    runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
                }
                bone_positions_changed = true;
            }
            ActorAction::SetPhysics { actor, physics } => {
                transformed |= scene.set_actor_physics(actor, physics)?;
                runtime.clear_actor_visual_bounds(actor);
                if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                    runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
                }
                bone_positions_changed = true;
            }
            ActorAction::SetDrawScale { actor, draw_scale } => {
                transformed |= scene.set_actor_draw_scale(actor, draw_scale)?;
                runtime.clear_actor_visual_bounds(actor);
                if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                    runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
                }
                bone_positions_changed = true;
            }
            ActorAction::SetStyle { actor, style } => {
                transformed |= scene.set_actor_style(actor, style)?;
            }
            ActorAction::SetScaleGlow { actor, scale_glow } => {
                transformed |= scene.set_actor_scale_glow(actor, scale_glow)?;
            }
            ActorAction::SetSkin { actor, skin } => {
                transformed |= scene.set_actor_skin(actor, skin)?;
            }
            ActorAction::SetSkelAnim { actor, skel_anim } => {
                transformed |= scene.set_actor_skeletal_animation(actor, skel_anim)?;
                runtime
                    .set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))?;
                runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
                bone_positions_changed = true;
            }
            ActorAction::SetAmbientGlow {
                actor,
                ambient_glow,
            } => {
                transformed |= scene.set_actor_ambient_glow(actor, ambient_glow)?;
            }
            ActorAction::SetOpacity { actor, opacity } => {
                transformed |= scene.set_actor_opacity(actor, opacity)?;
            }
            ActorAction::SetWarpDestination { actor, destination } => {
                transformed |= scene.set_warp_destination(actor, destination)?;
            }
            ActorAction::SetLightBrightness {
                actor,
                light_brightness,
            } => {
                transformed |= scene.set_light_brightness(actor, light_brightness)?;
            }
            ActorAction::UnsupportedSceneProperty { actor, property } => {
                // ParticleFX state is projected by sync_particle_emitters each frame.
                if scene.actors[actor].draw_type == 8 {
                    continue;
                }
                let diagnostic =
                    format!("runtime property {property} is not projected into the scene");
                if !scene.actors[actor].diagnostics.contains(&diagnostic) {
                    warn!(
                        actor,
                        actor_name = scene.actors[actor].name,
                        %diagnostic,
                        "render capability diagnostic"
                    );
                    scene.actors[actor].diagnostics.push(diagnostic);
                }
            }
            ActorAction::DestroyActor { actor } => {
                transformed |= scene.destroy_actor(actor)?;
            }
            ActorAction::Log {
                actor,
                message,
                tag,
            } => {
                info!(
                    actor,
                    actor_name = scene.actors[actor].name,
                    tag = tag.as_deref().unwrap_or(""),
                    message = %message,
                    "UnrealScript log"
                );
            }
            ActorAction::DeferredCall { actor, message } => {
                deferred += 1;
                let diagnostic = format!("runtime deferred call: {message}");
                if !scene.actors[actor].diagnostics.contains(&diagnostic) {
                    warn!(
                        actor,
                        actor_name = scene.actors[actor].name,
                        message,
                        "deferred UnrealScript call"
                    );
                    scene.actors[actor].diagnostics.push(diagnostic);
                }
            }
            ActorAction::DispatchEvent {
                actor,
                event,
                arguments,
            } => {
                let Some(class) = scene.actors[actor].class.clone() else {
                    continue;
                };
                match runtime.dispatch_event_with_arguments(
                    actor,
                    &class.package,
                    class.export_index,
                    event,
                    &arguments,
                ) {
                    Ok(event_actions) => actions.extend(event_actions),
                    Err(error) => actions.push_back(ActorAction::DeferredCall {
                        actor,
                        message: format!("{event}: {error}"),
                    }),
                }
            }
        }
    }
    if bone_positions_changed {
        sync_runtime_pose(scene, runtime)?;
    }
    Ok((animations, deferred, transformed))
}

pub fn sync_runtime_pose(scene: &LoadedScene, runtime: &mut ScriptRuntime) -> Result<()> {
    for (actor, positions) in scene.runtime_bone_positions()? {
        runtime.set_actor_bone_positions(actor, positions);
    }
    for (actor, location, rotation) in scene.runtime_weapon_poses()? {
        runtime.set_actor_weapon_pose(actor, location, rotation)?;
    }
    Ok(())
}

fn record_animation_diagnostic(
    actor: usize,
    actor_name: &str,
    diagnostics: &mut Vec<String>,
    diagnostic: String,
) {
    if diagnostics.contains(&diagnostic) {
        return;
    }
    warn!(actor, actor_name, %diagnostic, "animation capability diagnostic");
    diagnostics.push(diagnostic);
}

#[cfg(test)]
mod tests {
    use super::record_animation_diagnostic;

    #[test]
    fn animation_diagnostics_are_unique_per_actor() {
        let mut diagnostics = Vec::new();
        record_animation_diagnostic(7, "Wand7", &mut diagnostics, "missing Select".to_owned());
        record_animation_diagnostic(7, "Wand7", &mut diagnostics, "missing Select".to_owned());
        assert_eq!(diagnostics, ["missing Select"]);
    }
}
