use std::{collections::BTreeMap, env, fs, path::PathBuf};

use anyhow::{Context, Result};
use glam::Vec3;
use openhp1_runtime::{ActorAction, ScriptRuntime};
use openhp1_scene::LoadedScene;

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
    let mut total = 0;
    let mut locations = 0;
    let mut relocated = 0;
    let mut deferred = BTreeMap::<String, (usize, String)>::new();

    for path in paths {
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
                let entry = deferred
                    .entry(format!("registration failed: {error}"))
                    .or_insert((0, sample));
                entry.0 += 1;
            }
        }
        let mut applied = 0;
        let mut applied_actors = Vec::new();
        let mut requested = 0;
        let mut actions = Vec::new();
        for event in [
            "PreBeginPlay",
            "BeginPlay",
            "PostBeginPlay",
            "SetInitialState",
        ] {
            for &(actor, _, _, ref package, export) in &classes {
                match runtime.dispatch_event(actor, package, export, event) {
                    Ok(actor_actions) => actions.extend(actor_actions),
                    Err(error) => {
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
                        let entry = deferred
                            .entry(format!("{event}: {error}"))
                            .or_insert((0, sample));
                        entry.0 += 1;
                    }
                }
            }
        }
        let timer_callbacks = runtime.timer_callbacks();
        let mut timer_actions = 0;
        for _ in 0..300 {
            let actor_actions = runtime.tick(1.0 / 60.0)?;
            timer_actions += actor_actions.len();
            actions.extend(actor_actions);
        }
        let timer_callbacks = runtime.timer_callbacks() - timer_callbacks;
        for action in actions {
            if let ActorAction::LoopAnimation {
                actor,
                sequence,
                rate,
            } = action
            {
                requested += 1;
                if scene.loop_actor_animation(actor, &sequence, rate)? {
                    applied += 1;
                    applied_actors.push(actor);
                } else {
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
            } else if let ActorAction::SetLocation { actor, location } = action {
                locations += 1;
                relocated +=
                    usize::from(scene.set_actor_location(actor, Vec3::from_array(location))?);
            } else if let ActorAction::DeferredCall { actor, message } = action {
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
                let entry = deferred.entry(message).or_insert((0, sample));
                entry.0 += 1;
            }
        }
        if timer_callbacks != 0 {
            println!(
                "{}: {timer_callbacks} timer callbacks, {timer_actions} actions",
                scene.path.display()
            );
        }
        if requested != 0 {
            let before = scene.render.mesh.positions.clone();
            scene.tick_animations(1.0 / 60.0)?;
            let moved = applied_actors
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
                "{}: {applied}/{requested} runtime animations, {moved} moved",
                scene.path.display()
            );
        }
        total += applied;
    }
    println!("{total} runtime animations applied");
    println!("{locations} SetLocation actions, {relocated} actor relocations");
    let mut deferred = deferred.into_iter().collect::<Vec<_>>();
    deferred.sort_by(|left, right| right.1.0.cmp(&left.1.0).then_with(|| left.0.cmp(&right.0)));
    for (message, (count, sample)) in deferred.into_iter().take(20) {
        println!("{count:5} deferred: {message} [{sample}]");
    }
    Ok(())
}
