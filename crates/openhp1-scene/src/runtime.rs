use std::collections::VecDeque;

use anyhow::{Context, Result};
use glam::Vec3;
use openhp1_runtime::{ActorAction, ScriptRuntime};
use tracing::info;

use crate::{LoadedScene, Rotator};

pub fn initialize_runtime(scene: &mut LoadedScene) -> Result<ScriptRuntime> {
    let game_root = scene
        .path
        .parent()
        .and_then(|directory| directory.parent())
        .context("map path must be inside the game's Maps directory")?;
    let mut runtime = ScriptRuntime::new(game_root)?;
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
        runtime.set_actor_animation_groups(actor, scene.actor_animation_groups(actor));
    }
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
                    let applied = apply_runtime_actions(scene, &mut runtime, actions)?;
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
    info!(events, animations, deferred, "initialized script runtime");
    Ok(runtime)
}

pub fn apply_runtime_actions(
    scene: &mut LoadedScene,
    runtime: &mut ScriptRuntime,
    actions: Vec<ActorAction>,
) -> Result<(usize, usize, bool)> {
    let mut animations = 0;
    let mut deferred = 0;
    let mut transformed = false;
    let mut actions = VecDeque::from(actions);
    while let Some(action) = actions.pop_front() {
        scene.ensure_runtime_actor(action.actor());
        match action {
            ActorAction::PlayAnimation {
                actor,
                sequence,
                rate,
                tween_time,
            } => {
                if scene.play_actor_animation_with_tween(actor, &sequence, rate, tween_time)? {
                    animations += 1;
                } else {
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime could not play animation {sequence}"));
                }
            }
            ActorAction::LoopAnimation {
                actor,
                sequence,
                rate,
                tween_time,
            } => {
                if scene.loop_actor_animation_with_tween(actor, &sequence, rate, tween_time)? {
                    animations += 1;
                } else {
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime could not play animation {sequence}"));
                }
            }
            ActorAction::AwaitAnimation { actor } => {
                scene.finish_actor_animation(actor);
                if !scene.actor_animation_playing(actor) {
                    actions.extend(runtime.animation_finished(actor)?);
                }
            }
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
            }
            ActorAction::SetLocation { actor, location } => {
                transformed |= scene.set_actor_location(actor, Vec3::from_array(location))?;
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
            }
            ActorAction::SetHidden { actor, hidden } => {
                transformed |= scene.set_actor_hidden(actor, hidden)?;
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
                if scene.actors[actor]
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.starts_with("runtime deferred"))
                    .count()
                    < 3
                {
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime deferred call: {message}"));
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
    Ok((animations, deferred, transformed))
}
