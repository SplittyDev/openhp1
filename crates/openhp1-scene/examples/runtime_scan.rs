use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    env, fs,
    path::PathBuf,
};

use anyhow::{Context, Result};
use glam::Vec3;
use openhp1_runtime::{ActorAction, ConsoleCommands, PlayerInput, ScriptRuntime};
use openhp1_scene::{LoadedScene, Rotator};

#[derive(Default)]
struct ScanStats {
    animations_requested: usize,
    animations_applied: usize,
    animated_actors: HashSet<usize>,
    sounds: usize,
    music_changes: usize,
    spawned: usize,
    locations: usize,
    relocated: usize,
    rotations: usize,
    rotated: usize,
    visibility: usize,
    visibility_changed: usize,
    destroyed: usize,
    logs: usize,
}

fn spawned_configured_hud(actions: &[ActorAction]) -> bool {
    matches!(actions.first(), Some(ActorAction::SpawnActor { .. }))
}

fn main() -> Result<()> {
    let seconds = env::args()
        .nth(2)
        .map(|value| value.parse::<usize>())
        .transpose()
        .context("simulation duration must be a whole number of seconds")?
        .unwrap_or(5);
    let ticks = seconds
        .checked_mul(60)
        .context("simulation duration is too large")?;
    let maps = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("res/Maps"));
    let mut paths = if maps.is_file() {
        vec![maps]
    } else {
        fs::read_dir(maps)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>()
    };
    paths.sort();
    let mut stats = ScanStats::default();
    let mut state_resumes = 0;
    let mut deferred = BTreeMap::<String, (usize, String)>::new();

    for path in paths {
        stats.animated_actors.clear();
        let mut scene = LoadedScene::load(path)?;
        let game_root = scene
            .path
            .parent()
            .and_then(|directory| directory.parent())
            .context("map must be inside the game's Maps directory")?;
        let mut runtime = ScriptRuntime::new(game_root)?;
        runtime.set_console_command_host(ConsoleCommands::headless(game_root)?);
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
                record_deferred(
                    &scene,
                    actor,
                    format!("registration failed: {error}"),
                    &mut deferred,
                );
            }
            runtime.set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))?;
            runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
            if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
            }
        }
        let actions = runtime.initialize_game()?;
        apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;

        let map_animations = stats.animations_applied;
        let map_requested = stats.animations_requested;
        let map_state_resumes = runtime.state_resumes();
        let before = scene.render.mesh.positions.clone();
        for event in [
            "PreBeginPlay",
            "BeginPlay",
            "PostBeginPlay",
            "SetInitialState",
        ] {
            for &(actor, _, _, ref package, export) in &classes {
                match runtime.dispatch_event(actor, package, export, event) {
                    Ok(actions) => {
                        apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?
                    }
                    Err(error) => {
                        record_deferred(&scene, actor, format!("{event}: {error}"), &mut deferred)
                    }
                }
            }
        }
        for actor in &scene.actors {
            let Some(render) = &actor.render else {
                continue;
            };
            anyhow::ensure!(
                actor.draw_type != 0
                    || scene.render.mesh.positions[render.vertices.clone()]
                        .iter()
                        .all(|position| *position == Vec3::ZERO),
                "{} retained visible geometry after switching to DT_None",
                actor.name
            );
        }
        let player = runtime.player_actor();
        if let Some(player) = player {
            anyhow::ensure!(
                scene
                    .actors
                    .iter()
                    .any(|actor| actor.class_name.eq_ignore_ascii_case("CamTarget")),
                "{}: authored player setup did not spawn its camera target",
                scene.path.display()
            );
            let actions = runtime.dispatch_player_event("Possess", &[])?;
            apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
            let actions = runtime.initialize_player_hud()?;
            anyhow::ensure!(
                spawned_configured_hud(&actions),
                "{}: local player setup did not spawn its configured HUDType",
                scene.path.display()
            );
            apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
            anyhow::ensure!(
                runtime.initialize_player_hud()?.is_empty(),
                "{}: local player setup spawned HPHud twice",
                scene.path.display()
            );
            let actions = runtime.tick_player(PlayerInput::default(), 1.0 / 60.0)?;
            apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
            let actor = &scene.actors[player];
            let (view, actions) = runtime.player_view(
                actor.location.to_array(),
                [
                    actor.rotation.pitch,
                    actor.rotation.yaw,
                    actor.rotation.roll,
                ],
            )?;
            anyhow::ensure!(
                view.location.iter().all(|value| value.is_finite()),
                "player camera location is invalid"
            );
            anyhow::ensure!(
                view.actor != player,
                "authored player setup did not select a camera actor"
            );
            apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
        }

        let timer_callbacks = runtime.timer_callbacks();
        let mut timer_actions = 0;
        for _ in 0..ticks {
            let (_, completed) = scene.tick_animations_with_completions(1.0 / 60.0)?;
            for (actor, delta) in scene.take_root_motions() {
                let actions = runtime.apply_root_motion(actor, delta.to_array())?;
                apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
            }
            for actor in completed {
                let actions = runtime.animation_finished(actor)?;
                apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
            }
            openhp1_scene::sync_runtime_bone_positions(&scene, &mut runtime)?;
            if player.is_some() {
                runtime.set_player_input(PlayerInput::default())?;
            }
            let actions = runtime.tick(1.0 / 60.0)?;
            timer_actions += actions.len();
            apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
            if player.is_some() {
                stats.music_changes += usize::from(runtime.take_player_music()?.is_some());
            }
        }
        let timer_callbacks = runtime.timer_callbacks() - timer_callbacks;
        let map_state_resumes = runtime.state_resumes() - map_state_resumes;
        state_resumes += map_state_resumes;
        if timer_callbacks != 0 || map_state_resumes != 0 {
            println!(
                "{}: {timer_callbacks} timer callbacks, {map_state_resumes} state resumes, {timer_actions} actions",
                scene.path.display()
            );
        }

        let map_applied = stats.animations_applied - map_animations;
        let map_requested = stats.animations_requested - map_requested;
        if map_requested != 0 {
            let moved = stats
                .animated_actors
                .iter()
                .filter(|&&actor| {
                    scene.actors[actor].render.as_ref().is_some_and(|render| {
                        render.vertices.clone().any(|vertex| {
                            before.get(vertex).is_none_or(|before| {
                                !before.abs_diff_eq(scene.render.mesh.positions[vertex], 1.0e-4)
                            })
                        })
                    })
                })
                .count();
            println!(
                "{}: {map_applied}/{map_requested} runtime animations, {moved} moved",
                scene.path.display()
            );
        }
    }

    println!(
        "{}/{} runtime animations applied",
        stats.animations_applied, stats.animations_requested
    );
    println!("{} sounds requested", stats.sounds);
    println!("{} music changes requested", stats.music_changes);
    println!("{state_resumes} state frames resumed");
    println!("{} actors spawned", stats.spawned);
    println!(
        "{} SetLocation actions, {} actor relocations",
        stats.locations, stats.relocated
    );
    println!(
        "{} SetRotation actions, {} actor rotations",
        stats.rotations, stats.rotated
    );
    println!(
        "{} render visibility actions, {} actor visibility changes",
        stats.visibility, stats.visibility_changed
    );
    println!("{} actors destroyed", stats.destroyed);
    println!("{} script log messages", stats.logs);
    let mut deferred = deferred.into_iter().collect::<Vec<_>>();
    deferred.sort_by(|left, right| right.1.0.cmp(&left.1.0).then_with(|| left.0.cmp(&right.0)));
    for (message, (count, sample)) in deferred.into_iter().take(20) {
        println!("{count:5} deferred: {message} [{sample}]");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_configured_hud_subclass() {
        assert!(spawned_configured_hud(&[ActorAction::SpawnActor {
            actor: 1,
            name: "QuidHud1".to_owned(),
            class_package: "Test.u".into(),
            class_export: 0,
            class_name: "QuidHud".to_owned(),
            location: [0.0; 3],
            rotation: [0; 3],
        }]));
    }
}

fn apply_actions(
    scene: &mut LoadedScene,
    runtime: &mut ScriptRuntime,
    actions: Vec<ActorAction>,
    stats: &mut ScanStats,
    deferred: &mut BTreeMap<String, (usize, String)>,
) -> Result<()> {
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
                stats.animations_requested += 1;
                let played = if root_motion {
                    scene.play_actor_animation_with_root_motion(
                        actor, &sequence, rate, tween_time, false,
                    )?
                } else {
                    scene.play_actor_animation_with_tween(actor, &sequence, rate, tween_time)?
                };
                if played {
                    stats.animations_applied += 1;
                    stats.animated_actors.insert(actor);
                } else {
                    unavailable_animation(scene, actor, &sequence);
                }
            }
            ActorAction::LoopAnimation {
                actor,
                sequence,
                rate,
                tween_time,
                root_motion,
            } => {
                stats.animations_requested += 1;
                let played = if root_motion {
                    scene.play_actor_animation_with_root_motion(
                        actor, &sequence, rate, tween_time, true,
                    )?
                } else {
                    scene.loop_actor_animation_with_tween(actor, &sequence, rate, tween_time)?
                };
                if played {
                    stats.animations_applied += 1;
                    stats.animated_actors.insert(actor);
                } else {
                    unavailable_animation(scene, actor, &sequence);
                }
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
                stats.animations_requested += 1;
                if scene.restore_actor_animation(
                    actor,
                    &sequence,
                    rate,
                    tween_time,
                    looping,
                    tween_only,
                    root_motion,
                    phase,
                )? {
                    stats.animations_applied += 1;
                    stats.animated_actors.insert(actor);
                } else {
                    unavailable_animation(scene, actor, &sequence);
                }
            }
            ActorAction::AwaitAnimation { actor } => {
                scene.finish_actor_animation(actor);
                if !scene.actor_animation_playing(actor) {
                    actions.extend(runtime.animation_finished(actor)?);
                }
            }
            ActorAction::PlaySound { .. } => {
                stats.sounds += 1;
            }
            ActorAction::ModifySound { .. } => {}
            ActorAction::StopSound { .. } => {}
            ActorAction::ClientTravel { .. } => {}
            ActorAction::UpdateUrl { .. } => {}
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
                runtime
                    .set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))?;
                runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
                if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                    runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
                }
                stats.spawned += 1;
            }
            ActorAction::SetLocation { actor, location } => {
                stats.locations += 1;
                stats.relocated +=
                    usize::from(scene.set_actor_location(actor, Vec3::from_array(location))?);
            }
            ActorAction::SetRotation { actor, rotation } => {
                stats.rotations += 1;
                stats.rotated += usize::from(scene.set_actor_rotation(
                    actor,
                    Rotator {
                        pitch: rotation[0],
                        yaw: rotation[1],
                        roll: rotation[2],
                    },
                )?);
            }
            ActorAction::SetPrePivot { actor, pre_pivot } => {
                scene.set_actor_pre_pivot(actor, Vec3::from_array(pre_pivot))?;
            }
            ActorAction::SetHidden { actor, hidden } => {
                stats.visibility += 1;
                stats.visibility_changed += usize::from(scene.set_actor_hidden(actor, hidden)?);
            }
            ActorAction::SetDrawType { actor, draw_type } => {
                stats.visibility += 1;
                stats.visibility_changed +=
                    usize::from(scene.set_actor_draw_type(actor, draw_type)?);
            }
            ActorAction::SetMesh { actor, mesh } => {
                stats.visibility += 1;
                stats.visibility_changed += usize::from(scene.set_actor_mesh(actor, mesh)?);
                runtime
                    .set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))?;
                runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
                runtime.clear_actor_visual_bounds(actor);
                if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                    runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
                }
            }
            ActorAction::SetDrawScale { actor, draw_scale } => {
                stats.visibility += 1;
                stats.visibility_changed +=
                    usize::from(scene.set_actor_draw_scale(actor, draw_scale)?);
                runtime.clear_actor_visual_bounds(actor);
                if let Some((minimum, maximum)) = scene.actor_visual_bounds(actor) {
                    runtime.set_actor_visual_bounds(actor, minimum, maximum)?;
                }
            }
            ActorAction::SetStyle { actor, style } => {
                stats.visibility += 1;
                stats.visibility_changed += usize::from(scene.set_actor_style(actor, style)?);
            }
            ActorAction::SetScaleGlow { actor, scale_glow } => {
                stats.visibility += 1;
                stats.visibility_changed +=
                    usize::from(scene.set_actor_scale_glow(actor, scale_glow)?);
            }
            ActorAction::SetSkin { actor, skin } => {
                stats.visibility += 1;
                stats.visibility_changed += usize::from(scene.set_actor_skin(actor, skin)?);
            }
            ActorAction::SetSkelAnim { actor, skel_anim } => {
                stats.visibility += 1;
                stats.visibility_changed +=
                    usize::from(scene.set_actor_skeletal_animation(actor, skel_anim)?);
                runtime
                    .set_actor_animation_sequences(actor, scene.actor_animation_sequences(actor))?;
                runtime.set_actor_bone_names(actor, scene.actor_bone_names(actor));
            }
            ActorAction::SetAmbientGlow {
                actor,
                ambient_glow,
            } => {
                stats.visibility += 1;
                stats.visibility_changed +=
                    usize::from(scene.set_actor_ambient_glow(actor, ambient_glow)?);
            }
            ActorAction::SetOpacity { actor, opacity } => {
                stats.visibility += 1;
                stats.visibility_changed += usize::from(scene.set_actor_opacity(actor, opacity)?);
            }
            ActorAction::SetLightBrightness {
                actor,
                light_brightness,
            } => {
                stats.visibility += 1;
                stats.visibility_changed +=
                    usize::from(scene.set_light_brightness(actor, light_brightness)?);
            }
            ActorAction::UnsupportedSceneProperty { actor, property } => {
                if scene.actors[actor].draw_type != 8 {
                    let diagnostic =
                        format!("runtime property {property} is not projected into the scene");
                    if !scene.actors[actor].diagnostics.contains(&diagnostic) {
                        scene.actors[actor].diagnostics.push(diagnostic);
                    }
                }
            }
            ActorAction::DestroyActor { actor } => {
                stats.destroyed += usize::from(scene.destroy_actor(actor)?);
            }
            ActorAction::Log { .. } => {
                stats.logs += 1;
            }
            ActorAction::DeferredCall { actor, message } => {
                record_deferred(scene, actor, message, deferred);
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
                    Err(error) => {
                        record_deferred(scene, actor, format!("{event}: {error}"), deferred)
                    }
                }
            }
        }
    }
    Ok(())
}

fn unavailable_animation(scene: &LoadedScene, actor: usize, sequence: &str) {
    let target = &scene.actors[actor];
    println!(
        "  {} (mesh {}, rendered {}, animation {}): sequence {sequence} is unavailable",
        target.name,
        target.mesh_name.as_deref().unwrap_or("none"),
        target.render.is_some(),
        target
            .animation
            .as_ref()
            .map_or("none", |animation| animation.sequence.as_str()),
    );
    for diagnostic in &target.diagnostics {
        println!("    {diagnostic}");
    }
}

fn record_deferred(
    scene: &LoadedScene,
    actor: usize,
    message: String,
    deferred: &mut BTreeMap<String, (usize, String)>,
) {
    let target = &scene.actors[actor];
    let sample = format!(
        "{} ({} from {})",
        target.name,
        target.class_name,
        target
            .class
            .as_ref()
            .map_or("unknown", |class| &class.package)
    );
    deferred.entry(message).or_insert((0, sample)).0 += 1;
}
