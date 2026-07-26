use std::{
    env,
    error::Error,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use openhp1_mesh::{Mesh, SkeletalAnimation};
use openhp1_package::{ObjectReference, PACKAGE_MAGIC, Package};

fn main() -> Result<(), Box<dyn Error>> {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("res"));
    let mut paths = Vec::new();
    collect_package_paths(&root, &mut paths)?;
    paths.sort();

    let mut meshes = 0;
    let mut sequences = 0;
    let mut sampled = 0;
    let mut skeletal_animations = 0;
    let mut skeletal_sequences = 0;
    let mut skeletal_sampled = 0;
    for path in paths {
        let package = Package::open(&path)?;
        for (export_index, export) in package.summary().exports.iter().enumerate() {
            if package.summary().class_name(export) == Some("Animation") {
                let animation = SkeletalAnimation::decode(&package, export_index)?;
                skeletal_animations += 1;
                skeletal_sequences += animation.sequences.len();
                continue;
            }
            if !matches!(
                package.summary().class_name(export),
                Some("Mesh" | "LodMesh" | "SkeletalMesh")
            ) {
                continue;
            }
            let mesh = Mesh::decode(&package, export_index)?;
            meshes += 1;
            sequences += mesh.animation_sequences.len();
            if !mesh.animation_sequences.is_empty() {
                println!(
                    "{}:{} ({})",
                    path.display(),
                    package.summary().name(export.object_name),
                    mesh.animation_sequences
                        .iter()
                        .map(|sequence| sequence.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if let ObjectReference::Export(animation_index) = mesh.default_animation {
                let animation = SkeletalAnimation::decode(&package, animation_index)?;
                for sequence in 0..animation.sequences.len() {
                    mesh.sample_skeletal_sequence(&animation, sequence, 0.375)
                        .map_err(|error| {
                            format!(
                                "{}:{} sequence {}: {error}",
                                path.display(),
                                package.summary().name(export.object_name),
                                animation.sequences[sequence].name
                            )
                        })?;
                    skeletal_sampled += 1;
                }
            }
            if mesh.frame_vertices == 0 || mesh.animation_frames == 0 {
                continue;
            }
            for sequence in &mesh.animation_sequences {
                mesh.sample_sequence(sequence, 0.375)?;
                sampled += 1;
            }
        }
    }

    println!(
        "decoded {meshes} meshes and {sequences} vertex sequences; sampled {sampled}; \
         decoded {skeletal_animations} skeletal animations and {skeletal_sequences} sequences; \
         sampled {skeletal_sampled}"
    );
    Ok(())
}

fn collect_package_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_package_paths(&path, paths)?;
            continue;
        }

        let mut magic = [0; 4];
        if File::open(&path)?.read_exact(&mut magic).is_ok() && magic == PACKAGE_MAGIC.to_le_bytes()
        {
            paths.push(path);
        }
    }
    Ok(())
}
