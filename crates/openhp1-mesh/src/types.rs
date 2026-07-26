use glam::{IVec3, Vec2, Vec3};
use openhp1_package::ObjectReference;

use crate::{Error, Result, geometry::sample_triangles};

#[derive(Clone, Copy, Debug)]
pub struct MeshVertex {
    pub position: Vec3,
    pub normal: Vec3,
    /// Normalized texture coordinates.
    pub texture_coordinates: Vec2,
}

#[derive(Clone, Copy, Debug)]
pub struct MeshTriangle {
    pub vertices: [MeshVertex; 3],
    pub poly_flags: u32,
    pub texture_index: i32,
}

#[derive(Clone, Debug)]
pub struct MeshAnimationNotify {
    pub time: f32,
    pub function: String,
}

#[derive(Clone, Debug)]
pub struct MeshAnimationSequence {
    pub name: String,
    pub group: String,
    pub start_frame: usize,
    pub frame_count: usize,
    pub notifications: Vec<MeshAnimationNotify>,
    pub rate: f32,
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub triangles: Vec<MeshTriangle>,
    pub textures: Vec<ObjectReference>,
    pub animation_sequences: Vec<MeshAnimationSequence>,
    pub frame_vertices: usize,
    pub animation_frames: usize,
    pub scale: Vec3,
    pub origin: Vec3,
    pub rotation_origin: IVec3,
    pub(crate) vertices: Vec<Vec3>,
    pub(crate) normals: Vec<Vec3>,
    pub(crate) face_vertices: Vec<[usize; 3]>,
}

impl Mesh {
    /// Samples a looping animation sequence at a normalized phase.
    ///
    /// Phase zero is the first frame and phase one wraps back to it.
    pub fn sample_sequence(
        &self,
        sequence: &MeshAnimationSequence,
        phase: f32,
    ) -> Result<Vec<MeshTriangle>> {
        if !phase.is_finite() {
            return Err(Error::InvalidAnimationPhase(phase));
        }
        if sequence.frame_count == 0 {
            return Err(Error::EmptyAnimationSequence {
                name: sequence.name.clone(),
            });
        }
        if self.frame_vertices == 0 || self.vertices.is_empty() {
            return Err(Error::NoVertexAnimation);
        }

        let frame = phase.rem_euclid(1.0) * sequence.frame_count as f32;
        let local_first = frame.floor() as usize % sequence.frame_count;
        let local_second = (local_first + 1) % sequence.frame_count;
        let blend = frame.fract();
        let sequence_frame = |local| {
            sequence
                .start_frame
                .checked_add(local)
                .ok_or_else(|| Error::InvalidAnimationSequence {
                    name: sequence.name.clone(),
                    start_frame: sequence.start_frame,
                    end_frame: usize::MAX,
                    animation_frames: self.animation_frames,
                })
        };
        let first_frame = sequence_frame(local_first)?;
        let second_frame = sequence_frame(local_second)?;
        let first = self.frame_slice(&self.vertices, first_frame)?;
        let second = self.frame_slice(&self.vertices, second_frame)?;
        let first_normals = self.frame_slice(&self.normals, first_frame)?;
        let second_normals = self.frame_slice(&self.normals, second_frame)?;
        let vertices = first
            .iter()
            .zip(second)
            .map(|(first, second)| first.lerp(*second, blend))
            .collect::<Vec<_>>();
        let normals = first_normals
            .iter()
            .zip(second_normals)
            .map(|(first, second)| first.lerp(*second, blend).normalize_or_zero())
            .collect::<Vec<_>>();
        sample_triangles(&self.triangles, &self.face_vertices, &vertices, &normals)
    }

    fn frame_slice<'a>(&self, values: &'a [Vec3], frame: usize) -> Result<&'a [Vec3]> {
        let invalid_layout = || Error::InvalidAnimationLayout {
            frame_vertices: self.frame_vertices,
            animation_frames: self.animation_frames,
            vertex_count: values.len(),
        };
        let start = frame
            .checked_mul(self.frame_vertices)
            .ok_or_else(invalid_layout)?;
        let end = start
            .checked_add(self.frame_vertices)
            .ok_or_else(invalid_layout)?;
        values.get(start..end).ok_or(Error::InvalidAnimationFrame {
            frame,
            animation_frames: self.animation_frames,
        })
    }
}
