//! Decoders for the paletted UE1 textures used by Harry Potter 1.

mod decode;
mod error;
mod palette;
mod texture;

pub use error::{Error, Result};
pub use palette::{Color, Palette};
pub use texture::{
    MipLevel, Texture, TextureRenderFlags, WaterAnimation, WaterDrop, WetTexture,
    texture_poly_flags,
};
