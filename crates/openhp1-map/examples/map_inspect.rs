use std::{collections::BTreeSet, env, error::Error, path::PathBuf};

use openhp1_map::{Model, PolyFlags, world_model_export};
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
    let translucent_surfaces = model
        .surfaces
        .iter()
        .filter(|surface| is_translucent(surface.poly_flags))
        .count();
    let modulated_surfaces = model
        .surfaces
        .iter()
        .filter(|surface| is_modulated(surface.poly_flags))
        .count();
    let translucent_triangles = mesh
        .triangle_surfaces
        .iter()
        .filter(|&&surface| is_translucent(model.surfaces[surface].poly_flags))
        .count();
    let modulated_triangles = mesh
        .triangle_surfaces
        .iter()
        .filter(|&&surface| is_modulated(model.surfaces[surface].poly_flags))
        .count();
    let translucent_textures = model
        .surfaces
        .iter()
        .filter(|surface| is_translucent(surface.poly_flags))
        .filter_map(|surface| package.summary().object_name(surface.texture))
        .collect::<BTreeSet<_>>();
    let modulated_textures = model
        .surfaces
        .iter()
        .filter(|surface| is_modulated(surface.poly_flags))
        .filter_map(|surface| package.summary().object_name(surface.texture))
        .collect::<BTreeSet<_>>();
    println!(
        "{}: world export {}, {} points, {} vectors, {} nodes, {} surfaces, {} triangles, \
         translucent {translucent_surfaces} surfaces/{translucent_triangles} triangles, \
         modulated {modulated_surfaces} surfaces/{modulated_triangles} triangles",
        path.display(),
        export_index,
        model.points.len(),
        model.vectors.len(),
        model.nodes.len(),
        model.surfaces.len(),
        mesh.indices.len() / 3
    );
    if !translucent_textures.is_empty() {
        println!("  translucent textures: {translucent_textures:?}");
    }
    if !modulated_textures.is_empty() {
        println!("  modulated textures: {modulated_textures:?}");
    }
    Ok(())
}

fn is_translucent(flags: PolyFlags) -> bool {
    flags.contains(PolyFlags::TRANSLUCENT)
}

fn is_modulated(flags: PolyFlags) -> bool {
    !is_translucent(flags) && flags.contains(PolyFlags::MODULATED)
}
