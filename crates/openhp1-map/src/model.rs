use glam::Vec3;
use openhp1_package::{ObjectReference, Package};

use crate::{
    BspNode, BspSurface, BspVertex, ConvexLeaf, Error, LightMap, PrimitiveBounds, Result, Zone,
    decode::{
        compact_count, fixed_count, index, invalid_count, read_box, read_primitive_bounds,
        read_vec3, require_class,
    },
};

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
                poly_flags: crate::PolyFlags::from_bits(reader.read_u32()?),
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
