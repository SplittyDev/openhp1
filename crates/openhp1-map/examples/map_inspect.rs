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
    let sky_zone = model.sky_zone(&package)?;
    let fake_backdrop_surfaces = model
        .surfaces
        .iter()
        .filter(|surface| surface.poly_flags.contains(PolyFlags::FAKE_BACKDROP))
        .count();
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
         modulated {modulated_surfaces} surfaces/{modulated_triangles} triangles, \
         fake backdrops {fake_backdrop_surfaces}, sky zone {}",
        path.display(),
        export_index,
        model.points.len(),
        model.vectors.len(),
        model.nodes.len(),
        model.surfaces.len(),
        mesh.indices.len() / 3,
        sky_zone.map_or("none".to_owned(), |sky| format!(
            "at {:?}, rotation {:?}",
            sky.location, sky.rotation
        ))
    );
    if !translucent_textures.is_empty() {
        println!("  translucent textures: {translucent_textures:?}");
    }
    if !modulated_textures.is_empty() {
        println!("  modulated textures: {modulated_textures:?}");
    }
    if let Some(sky_zone_index) = model.zones.iter().position(|zone| {
        let openhp1_package::ObjectReference::Export(index) = zone.actor else {
            return false;
        };
        package
            .summary()
            .exports
            .get(index)
            .and_then(|export| package.summary().class_name(export))
            == Some("SkyZoneInfo")
    }) {
        let sky_textures = model
            .nodes
            .iter()
            .filter(|node| node.zones.contains(&(sky_zone_index as i32)))
            .filter_map(|node| model.surfaces.get(node.surface as usize))
            .filter_map(|surface| package.summary().object_name(surface.texture))
            .collect::<BTreeSet<_>>();
        let (sky_unlit, sky_lit) = model
            .nodes
            .iter()
            .filter(|node| node.zones.contains(&(sky_zone_index as i32)))
            .filter_map(|node| model.surfaces.get(node.surface as usize))
            .fold((0, 0), |(unlit, lit), surface| {
                if surface.poly_flags.contains(PolyFlags::UNLIT) {
                    (unlit + 1, lit)
                } else {
                    (unlit, lit + 1)
                }
            });
        println!("  sky-zone textures: {sky_textures:?} ({sky_unlit} unlit/{sky_lit} lit nodes)");
    }
    Ok(())
}

fn is_translucent(flags: PolyFlags) -> bool {
    flags.contains(PolyFlags::TRANSLUCENT)
}

fn is_modulated(flags: PolyFlags) -> bool {
    !is_translucent(flags) && flags.contains(PolyFlags::MODULATED)
}
