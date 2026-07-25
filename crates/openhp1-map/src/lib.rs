//! Typed decoding and triangulation of the UE1 map structures used by HP1.

use std::sync::Arc;

use glam::Vec3;
use openhp1_package::{ObjectReader, ObjectReference, Package};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub struct PrimitiveBounds {
    pub minimum: Vec3,
    pub maximum: Vec3,
    pub valid: bool,
    pub sphere: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct BspNode {
    pub plane: [f32; 4],
    pub zone_mask: u64,
    pub flags: u8,
    pub vertex_pool: i32,
    pub surface: i32,
    pub back: i32,
    pub front: i32,
    pub coplanar: i32,
    pub collision_bound: i32,
    pub render_bound: i32,
    pub zones: [i32; 2],
    pub vertex_count: u8,
    pub leaves: [i32; 2],
}

#[derive(Clone, Debug)]
pub struct BspSurface {
    pub texture: ObjectReference,
    pub poly_flags: u32,
    pub base_point: i32,
    pub normal: i32,
    pub texture_u: i32,
    pub texture_v: i32,
    pub light_map: i32,
    pub brush_poly: i32,
    pub pan_u: i16,
    pub pan_v: i16,
    pub brush_actor: ObjectReference,
}

#[derive(Clone, Debug)]
pub struct BspVertex {
    pub point: i32,
    pub side: i32,
}

#[derive(Clone, Debug)]
pub struct Zone {
    pub actor: ObjectReference,
    pub connectivity: u64,
    pub visibility: u64,
}

#[derive(Clone, Debug)]
pub struct LightMap {
    pub data_offset: i32,
    pub pan: Vec3,
    pub clamp: [i32; 2],
    pub scale: [f32; 2],
    pub light_actors: i32,
}

#[derive(Clone, Debug)]
pub struct ConvexLeaf {
    pub zone: i32,
    pub permeating: i32,
    pub volumetric: i32,
    pub visible_zones: u64,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub bounds: PrimitiveBounds,
    pub vectors: Vec<Vec3>,
    pub points: Vec<Vec3>,
    pub nodes: Vec<BspNode>,
    pub surfaces: Vec<BspSurface>,
    pub vertices: Vec<BspVertex>,
    pub shared_side_count: i32,
    pub zones: Vec<Zone>,
    pub polys: ObjectReference,
    pub light_maps: Vec<LightMap>,
    pub light_bits: Vec<u8>,
    pub collision_bounds: Vec<PrimitiveBounds>,
    pub leaf_hulls: Vec<i32>,
    pub leaves: Vec<ConvexLeaf>,
    pub lights: Vec<ObjectReference>,
    pub root_outside: bool,
    pub linked: bool,
}

/// Indexed triangles plus the BSP surface responsible for each triangle.
#[derive(Clone, Debug, Default)]
pub struct TriangleMesh {
    pub positions: Vec<Vec3>,
    pub indices: Vec<u32>,
    pub triangle_surfaces: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Level {
    pub actors: Vec<ObjectReference>,
    pub model: ObjectReference,
}

impl Level {
    /// Decodes enough of `ULevel` to identify the authoritative world model.
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Level")?;
        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}

        let actor_count = fixed_count(&mut reader, "level actors")?;
        let _actor_capacity = reader.read_i32()?;
        let mut actors = Vec::with_capacity(actor_count);
        for _ in 0..actor_count {
            actors.push(reader.read_object_reference()?);
        }

        // ULevelBase serializes the URL that was used to enter the map.
        for _ in 0..4 {
            reader.read_string()?;
        }
        let option_count = compact_count(&mut reader, 1, "level URL options")?;
        for _ in 0..option_count {
            reader.read_string()?;
        }
        reader.read_i32()?; // port
        reader.read_u32()?; // legacy URL field
        let model = reader.read_object_reference()?;
        Ok(Self { actors, model })
    }
}

impl Model {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Model")?;
        if package.summary().header.version <= 61 {
            return Err(Error::LegacyModelLayout {
                version: package.summary().header.version,
            });
        }

        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}
        let bounds = read_primitive_bounds(&mut reader, true)?;

        let vector_count = compact_count(&mut reader, 12, "model vectors")?;
        let mut vectors = Vec::with_capacity(vector_count);
        for _ in 0..vector_count {
            vectors.push(read_vec3(&mut reader)?);
        }

        let point_count = compact_count(&mut reader, 12, "model points")?;
        let mut points = Vec::with_capacity(point_count);
        for _ in 0..point_count {
            points.push(read_vec3(&mut reader)?);
        }

        let node_count = compact_count(&mut reader, 43, "BSP nodes")?;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            nodes.push(BspNode {
                plane: [
                    reader.read_f32()?,
                    reader.read_f32()?,
                    reader.read_f32()?,
                    reader.read_f32()?,
                ],
                zone_mask: reader.read_u64()?,
                flags: reader.read_u8()?,
                vertex_pool: reader.read_compact_index()?,
                surface: reader.read_compact_index()?,
                back: reader.read_compact_index()?,
                front: reader.read_compact_index()?,
                coplanar: reader.read_compact_index()?,
                collision_bound: reader.read_compact_index()?,
                render_bound: reader.read_compact_index()?,
                zones: [reader.read_compact_index()?, reader.read_compact_index()?],
                vertex_count: reader.read_u8()?,
                leaves: [reader.read_i32()?, reader.read_i32()?],
            });
        }

        let surface_count = compact_count(&mut reader, 16, "BSP surfaces")?;
        let mut surfaces = Vec::with_capacity(surface_count);
        for _ in 0..surface_count {
            surfaces.push(BspSurface {
                texture: reader.read_object_reference()?,
                poly_flags: reader.read_u32()?,
                base_point: reader.read_compact_index()?,
                normal: reader.read_compact_index()?,
                texture_u: reader.read_compact_index()?,
                texture_v: reader.read_compact_index()?,
                light_map: reader.read_compact_index()?,
                brush_poly: reader.read_compact_index()?,
                pan_u: reader.read_i16()?,
                pan_v: reader.read_i16()?,
                brush_actor: reader.read_object_reference()?,
            });
        }

        let vertex_count = compact_count(&mut reader, 2, "BSP vertex pool")?;
        let mut vertices = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            vertices.push(BspVertex {
                point: reader.read_compact_index()?,
                side: reader.read_compact_index()?,
            });
        }
        let shared_side_count = reader.read_i32()?;

        let zone_count = fixed_count(&mut reader, "model zones")?;
        if zone_count > reader.remaining() / 17 {
            return Err(invalid_count(&reader, "model zones", zone_count));
        }
        let mut zones = Vec::with_capacity(zone_count);
        for _ in 0..zone_count {
            zones.push(Zone {
                actor: reader.read_object_reference()?,
                connectivity: reader.read_u64()?,
                visibility: reader.read_u64()?,
            });
        }

        let polys = reader.read_object_reference()?;
        let light_map_count = compact_count(&mut reader, 30, "light maps")?;
        let mut light_maps = Vec::with_capacity(light_map_count);
        for _ in 0..light_map_count {
            light_maps.push(LightMap {
                data_offset: reader.read_i32()?,
                pan: read_vec3(&mut reader)?,
                clamp: [reader.read_compact_index()?, reader.read_compact_index()?],
                scale: [reader.read_f32()?, reader.read_f32()?],
                light_actors: reader.read_i32()?,
            });
        }

        let light_bit_count = compact_count(&mut reader, 1, "light bits")?;
        let light_bits = reader.read_bytes(light_bit_count)?.to_vec();

        let bound_count = compact_count(&mut reader, 25, "collision bounds")?;
        let mut collision_bounds = Vec::with_capacity(bound_count);
        for _ in 0..bound_count {
            collision_bounds.push(read_box(&mut reader)?);
        }

        let hull_count = compact_count(&mut reader, 4, "leaf hulls")?;
        let mut leaf_hulls = Vec::with_capacity(hull_count);
        for _ in 0..hull_count {
            leaf_hulls.push(reader.read_i32()?);
        }

        let leaf_count = compact_count(&mut reader, 11, "convex leaves")?;
        let mut leaves = Vec::with_capacity(leaf_count);
        for _ in 0..leaf_count {
            leaves.push(ConvexLeaf {
                zone: reader.read_compact_index()?,
                permeating: reader.read_compact_index()?,
                volumetric: reader.read_compact_index()?,
                visible_zones: reader.read_u64()?,
            });
        }

        let light_count = compact_count(&mut reader, 1, "model lights")?;
        let mut lights = Vec::with_capacity(light_count);
        for _ in 0..light_count {
            lights.push(reader.read_object_reference()?);
        }
        let root_outside = reader.read_i32()? != 0;
        let linked = reader.read_i32()? != 0;
        if reader.remaining() != 0 {
            return Err(Error::TrailingModelData {
                bytes: reader.remaining(),
                offset: reader.absolute_position(),
            });
        }

        let model = Self {
            bounds,
            vectors,
            points,
            nodes,
            surfaces,
            vertices,
            shared_side_count,
            zones,
            polys,
            light_maps,
            light_bits,
            collision_bounds,
            leaf_hulls,
            leaves,
            lights,
            root_outside,
            linked,
        };
        model.validate_indices()?;
        Ok(model)
    }

    pub fn triangulate(&self) -> Result<TriangleMesh> {
        let positions = self.points.clone();
        let mut indices = Vec::new();
        let mut triangle_surfaces = Vec::new();

        for node in &self.nodes {
            if node.vertex_count < 3 {
                continue;
            }
            let surface = index(node.surface, self.surfaces.len(), "node surface")?;
            let first_pool = index(node.vertex_pool, self.vertices.len(), "node vertex pool")?;
            let end_pool = first_pool
                .checked_add(usize::from(node.vertex_count))
                .filter(|end| *end <= self.vertices.len())
                .ok_or(Error::InvalidVertexPool {
                    start: first_pool,
                    count: node.vertex_count,
                    pool_len: self.vertices.len(),
                })?;
            let polygon = &self.vertices[first_pool..end_pool];
            let first = point_index(polygon[0].point, self.points.len())?;
            for pair in polygon[1..].windows(2) {
                indices.extend_from_slice(&[
                    first,
                    point_index(pair[0].point, self.points.len())?,
                    point_index(pair[1].point, self.points.len())?,
                ]);
                triangle_surfaces.push(surface);
            }
        }
        Ok(TriangleMesh {
            positions,
            indices,
            triangle_surfaces,
        })
    }

    fn validate_indices(&self) -> Result<()> {
        for node in &self.nodes {
            index(node.surface, self.surfaces.len(), "node surface")?;
            index(node.vertex_pool, self.vertices.len(), "node vertex pool")?;
        }
        for surface in &self.surfaces {
            index(surface.base_point, self.points.len(), "surface base point")?;
            index(surface.normal, self.vectors.len(), "surface normal")?;
            index(surface.texture_u, self.vectors.len(), "surface texture U")?;
            index(surface.texture_v, self.vectors.len(), "surface texture V")?;
        }
        Ok(())
    }
}

pub fn world_model_export(package: &Package) -> Result<usize> {
    let level_index = package
        .summary()
        .exports
        .iter()
        .position(|export| package.summary().class_name(export) == Some("Level"))
        .ok_or(Error::MissingLevel)?;
    match Level::decode(package, level_index)?.model {
        ObjectReference::Export(index) => Ok(index),
        reference => Err(Error::InvalidWorldModel { reference }),
    }
}

fn read_primitive_bounds(
    reader: &mut ObjectReader<'_>,
    sphere_radius: bool,
) -> Result<PrimitiveBounds> {
    let mut bounds = read_box(reader)?;
    bounds.sphere = [
        reader.read_f32()?,
        reader.read_f32()?,
        reader.read_f32()?,
        if sphere_radius {
            reader.read_f32()?
        } else {
            0.0
        },
    ];
    Ok(bounds)
}

fn read_box(reader: &mut ObjectReader<'_>) -> Result<PrimitiveBounds> {
    Ok(PrimitiveBounds {
        minimum: read_vec3(reader)?,
        maximum: read_vec3(reader)?,
        valid: reader.read_u8()? != 0,
        sphere: [0.0; 4],
    })
}

fn read_vec3(reader: &mut ObjectReader<'_>) -> Result<Vec3> {
    Ok(Vec3::new(
        reader.read_f32()?,
        reader.read_f32()?,
        reader.read_f32()?,
    ))
}

fn compact_count(
    reader: &mut ObjectReader<'_>,
    minimum_item_size: usize,
    field: &'static str,
) -> Result<usize> {
    let offset = reader.absolute_position();
    let value = reader.read_compact_index()?;
    let count = usize::try_from(value).map_err(|_| Error::InvalidCount {
        field,
        value,
        offset,
    })?;
    if minimum_item_size != 0 && count > reader.remaining() / minimum_item_size {
        return Err(invalid_count(reader, field, count));
    }
    Ok(count)
}

fn fixed_count(reader: &mut ObjectReader<'_>, field: &'static str) -> Result<usize> {
    let offset = reader.absolute_position();
    let value = reader.read_i32()?;
    usize::try_from(value).map_err(|_| Error::InvalidCount {
        field,
        value,
        offset,
    })
}

fn invalid_count(reader: &ObjectReader<'_>, field: &'static str, count: usize) -> Error {
    Error::CountExceedsPayload {
        field,
        count,
        remaining: reader.remaining(),
        offset: reader.absolute_position(),
    }
}

fn index(value: i32, length: usize, field: &'static str) -> Result<usize> {
    usize::try_from(value)
        .ok()
        .filter(|index| *index < length)
        .ok_or(Error::InvalidIndex {
            field,
            value,
            length,
        })
}

fn point_index(value: i32, length: usize) -> Result<u32> {
    let index = index(value, length, "BSP vertex point")?;
    u32::try_from(index).map_err(|_| Error::MeshTooLarge {
        point_count: length,
    })
}

fn require_class(package: &Package, export_index: usize, expected: &'static str) -> Result<()> {
    let summary = package.summary();
    let export = summary.exports.get(export_index).ok_or_else(|| {
        openhp1_package::Error::InvalidExportIndex {
            package: Arc::clone(&summary.source),
            index: export_index,
            export_count: summary.exports.len(),
        }
    })?;
    let actual = summary.class_name(export).unwrap_or("<class>");
    if actual != expected {
        return Err(Error::WrongClass {
            expected,
            actual: actual.to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),

    #[error("expected an export of class {expected}, found {actual}")]
    WrongClass {
        expected: &'static str,
        actual: String,
    },

    #[error("UE1 package version {version} uses split legacy model objects, not inline BSP data")]
    LegacyModelLayout { version: u16 },

    #[error("{field} count {value} at byte {offset:#x} is invalid")]
    InvalidCount {
        field: &'static str,
        value: i32,
        offset: usize,
    },

    #[error(
        "{field} count {count} at byte {offset:#x} cannot fit in the {remaining} remaining bytes"
    )]
    CountExceedsPayload {
        field: &'static str,
        count: usize,
        remaining: usize,
        offset: usize,
    },

    #[error("{field} index {value} is outside length {length}")]
    InvalidIndex {
        field: &'static str,
        value: i32,
        length: usize,
    },

    #[error(
        "BSP vertex range {start}..{end} exceeds pool length {pool_len}",
        end = start + usize::from(*count)
    )]
    InvalidVertexPool {
        start: usize,
        count: u8,
        pool_len: usize,
    },

    #[error("world mesh has {point_count} points, too many for 32-bit GPU indices")]
    MeshTooLarge { point_count: usize },

    #[error("model has {bytes} trailing bytes at {offset:#x}")]
    TrailingModelData { bytes: usize, offset: usize },

    #[error("map package has no Level export")]
    MissingLevel,

    #[error("Level references {reference:?} instead of an exported world Model")]
    InvalidWorldModel { reference: ObjectReference },
}
