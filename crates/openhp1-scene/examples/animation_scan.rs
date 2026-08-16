use std::{env, error::Error, fs, path::PathBuf};

use openhp1_scene::LoadedScene;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let maps = arguments
        .next()
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

    let mut animated_maps = 0;
    let mut animated_actors = 0;
    let mut water_maps = 0;
    let mut water_textures = 0;
    for path in paths {
        let mut scene = LoadedScene::load(path)?;
        let before = scene.render.mesh.positions.clone();
        scene.tick_animations(1.0 / 60.0)?;
        let textures = scene.tick_textures(1.0 / 30.0)?;
        if scene.animated_actor_meshes == 0 && textures.is_empty() {
            continue;
        }
        let moved = before
            .iter()
            .zip(&scene.render.mesh.positions)
            .filter(|(before, after)| *before != *after)
            .count();
        println!(
            "{}: {} animated actors, {moved} moved vertices, {} changed animated textures",
            scene.path.display(),
            scene.animated_actor_meshes,
            textures.len(),
        );
        animated_maps += usize::from(scene.animated_actor_meshes != 0);
        animated_actors += scene.animated_actor_meshes;
        water_maps += usize::from(!textures.is_empty());
        water_textures += textures.len();
    }
    println!("{animated_maps} maps contain {animated_actors} animated actors");
    println!("{water_maps} maps changed {water_textures} animated textures");
    Ok(())
}
