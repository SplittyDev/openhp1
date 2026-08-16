use std::sync::Arc;

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

    #[error("package `{package}` texture export {export_index} has no Palette property")]
    MissingPalette {
        package: Arc<str>,
        export_index: usize,
    },

    #[error("{field} count {value} at byte {offset:#x} is negative")]
    NegativeCount {
        field: &'static str,
        value: i32,
        offset: usize,
    },

    #[error("palette color count {count} is too large")]
    InvalidPaletteCount { count: usize },

    #[error(
        "package `{package}` texture export {export_index} mip {mip_index} ends at {actual:#x}, not its serialized lazy-array end {expected:#x}"
    )]
    InvalidLazyArrayEnd {
        package: Arc<str>,
        export_index: usize,
        mip_index: usize,
        expected: usize,
        actual: usize,
    },

    #[error("mip dimensions {width}x{height} overflow the host address space")]
    InvalidMipDimensions { width: u32, height: u32 },

    #[error("fire spark count {count} overflows the host address space")]
    InvalidFireSparkCount { count: usize },

    #[error("water drop {index} has {actual} bytes, expected 8")]
    InvalidWaterDropLength { index: usize, actual: usize },

    #[error("water texture has {count} drops, maximum is 256")]
    InvalidWaterDropCount { count: usize },

    #[error("water texture declares {count} drops but only {available} were serialized")]
    MissingWaterDrops { count: usize, available: usize },

    #[error("water texture dimensions must be nonzero, got {width}x{height}")]
    InvalidWaterDimensions { width: u32, height: u32 },

    #[error("water source is {width}x{height} but contains {actual} palette indices")]
    InvalidWaterSourceLength {
        width: u32,
        height: u32,
        actual: usize,
    },

    #[error("ice {field} dimensions must be nonzero powers of two, got {width}x{height}")]
    InvalidIceDimensions {
        field: &'static str,
        width: u32,
        height: u32,
    },

    #[error("ice {field} is {width}x{height} but contains {actual} palette indices")]
    InvalidIcePixels {
        field: &'static str,
        width: u32,
        height: u32,
        actual: usize,
    },

    #[error(
        "ice {field} dimensions {actual_width}x{actual_height} do not match {expected_width}x{expected_height}"
    )]
    IceDimensionMismatch {
        field: &'static str,
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },

    #[error("unsupported ice panning style {0}")]
    UnsupportedIcePanningStyle(u8),

    #[error("unsupported water drop type {0}")]
    UnsupportedWaterDropType(u8),

    #[error("mip {mip_index} is {width}x{height} but contains {actual} palette indices")]
    InvalidMipLength {
        mip_index: usize,
        width: u32,
        height: u32,
        actual: usize,
    },

    #[error("texture has no mip at index {mip_index}")]
    MissingMip { mip_index: usize },

    #[error("palette index {index} is outside its {color_count} colors")]
    PaletteIndexOutOfRange { index: u8, color_count: usize },
}
