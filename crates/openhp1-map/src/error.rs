use openhp1_package::ObjectReference;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

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

    #[error("lightmap {index} has invalid dimensions {width}x{height}")]
    InvalidLightmapDimensions {
        index: usize,
        width: i32,
        height: i32,
    },

    #[error("lightmap {index} has invalid scale {u}x{v}")]
    InvalidLightmapScale { index: usize, u: f32, v: f32 },

    #[error("lightmap {index} references light-list offset {offset} outside length {length}")]
    InvalidLightList {
        index: usize,
        offset: i32,
        length: usize,
    },

    #[error("lightmap {index} shadow mask range {start}..{end} exceeds light-bit length {length}")]
    InvalidLightBits {
        index: usize,
        start: usize,
        end: usize,
        length: usize,
    },

    #[error("model has {bytes} trailing bytes at {offset:#x}")]
    TrailingModelData { bytes: usize, offset: usize },

    #[error("map package has no Level export")]
    MissingLevel,

    #[error("Level references {reference:?} instead of an exported world Model")]
    InvalidWorldModel { reference: ObjectReference },
}
