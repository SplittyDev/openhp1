use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    env, fs,
    path::PathBuf,
};

use anyhow::{Context, Result};
use glam::Vec3;
use openhp1_runtime::{ActorAction, ScriptRuntime};
use openhp1_scene::{LoadedScene, Rotator};

#[derive(Default)]
struct ScanStats {
    animations_requested: usize,
    animations_applied: usize,
    animated_actors: HashSet<usize>,
    spawned: usize,
    locations: usize,
    relocated: usize,
    rotations: usize,
    rotated: usize,
    destroyed: usize,
    logs: usize,
}

fn main() -> Result<()> {
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
        }

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

        let timer_callbacks = runtime.timer_callbacks();
        let mut timer_actions = 0;
        for _ in 0..300 {
            let (_, completed) = scene.tick_animations_with_completions(1.0 / 60.0)?;
            for actor in completed {
                let actions = runtime.animation_finished(actor)?;
                apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
            }
            let actions = runtime.tick(1.0 / 60.0)?;
            timer_actions += actions.len();
            apply_actions(&mut scene, &mut runtime, actions, &mut stats, &mut deferred)?;
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
                            !before[vertex].abs_diff_eq(scene.render.mesh.positions[vertex], 1.0e-4)
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
    println!("{} actors destroyed", stats.destroyed);
    println!("{} script log messages", stats.logs);
    let mut deferred = deferred.into_iter().collect::<Vec<_>>();
    deferred.sort_by(|left, right| right.1.0.cmp(&left.1.0).then_with(|| left.0.cmp(&right.0)));
    for (message, (count, sample)) in deferred.into_iter().take(20) {
        println!("{count:5} deferred: {message} [{sample}]");
    }
    Ok(())
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
            } => {
                stats.animations_requested += 1;
                if scene.play_actor_animation_with_tween(actor, &sequence, rate, tween_time)? {
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
            } => {
                stats.animations_requested += 1;
                if scene.loop_actor_animation_with_tween(actor, &sequence, rate, tween_time)? {
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
            ActorAction::DestroyActor { actor } => {
                stats.destroyed += usize::from(scene.destroy_actor(actor)?);
            }
            ActorAction::Log { .. } => {
                stats.logs += 1;
            }
            ActorAction::DeferredCall { actor, message } => {
                record_deferred(scene, actor, message, deferred);
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
