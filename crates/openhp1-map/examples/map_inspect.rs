use std::{env, error::Error, path::PathBuf};

use openhp1_map::{Model, world_model_export};
use openhp1_package::Package;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: map_inspect <map.unr>")?;
    let package = Package::open(&path)?;
    let export_index = world_model_export(&package)?;
    let model = Model::decode(&package, export_index)?;
    let mesh = model.triangulate()?;
    println!(
        "{}: world export {}, {} points, {} vectors, {} nodes, {} surfaces, {} triangles",
        path.display(),
        export_index,
        model.points.len(),
        model.vectors.len(),
        model.nodes.len(),
        model.surfaces.len(),
        mesh.indices.len() / 3
    );
    Ok(())
}
