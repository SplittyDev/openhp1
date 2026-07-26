use std::{collections::HashSet, env, fs, path::PathBuf};

use anyhow::{Result, ensure};
use openhp1_scene::LoadedScene;

fn main() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let maps = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("res/Maps"));
    let single_map = maps.is_file();
    let mut paths = if single_map {
        vec![maps]
    } else {
        fs::read_dir(maps)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>()
    };
    paths.sort();

    let mut total_actors = 0;
    let mut rendered_actors = 0;
    let mut animated_actors = 0;
    let mut diagnostics = 0;
    for path in paths {
        let scene = LoadedScene::load(path)?;
        let mut ids = HashSet::new();
        for actor in &scene.actors {
            ensure!(
                ids.insert(actor.id.clone()),
                "duplicate scene actor identity {:?}",
                actor.id
            );
            if let Some(render) = &actor.render {
                ensure!(
                    render.vertices.end <= scene.render.mesh.positions.len()
                        && render.indices.end <= scene.render.mesh.indices.len(),
                    "{} has an out-of-bounds render range",
                    actor.name
                );
                ensure!(
                    scene.render.mesh.indices[render.indices.clone()]
                        .iter()
                        .all(|&index| render.vertices.contains(&(index as usize))),
                    "{} has indices outside its vertex range",
                    actor.name
                );
            }
        }
        let rendered = scene
            .actors
            .iter()
            .filter(|actor| actor.render.is_some())
            .count();
        let failed = scene
            .actors
            .iter()
            .filter(|actor| !actor.diagnostics.is_empty())
            .count();
        println!(
            "{}: {} actors, {rendered} rendered, {} animated, {failed} diagnostics",
            scene.path.display(),
            scene.actors.len(),
            scene.animated_actor_meshes
        );
        if single_map {
            for actor in scene
                .actors
                .iter()
                .filter(|actor| actor.render.is_none())
                .take(5)
            {
                println!(
                    "  {} ({}): {}",
                    actor.name,
                    actor.class_name,
                    actor
                        .diagnostics
                        .first()
                        .map(String::as_str)
                        .unwrap_or("not rendered")
                );
            }
        }
        total_actors += scene.actors.len();
        rendered_actors += rendered;
        animated_actors += scene.animated_actor_meshes;
        diagnostics += failed;
    }
    println!(
        "{total_actors} actors, {rendered_actors} rendered, \
         {animated_actors} animated, {diagnostics} diagnostics"
    );
    Ok(())
}
