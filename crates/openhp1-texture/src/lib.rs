//! Decoders for the paletted UE1 textures used by Harry Potter 1.

mod decode;
mod error;
mod fire;
mod palette;
mod texture;

pub use error::{Error, Result};
pub use fire::{FireAnimation, FireRng, FireSpark, FireTexture};
pub use palette::{Color, Palette};
pub use texture::{
    IceAnimation, IcePanningStyle, IceTexture, IceTimeMethod, MipLevel, Texture,
    TextureRenderFlags, WaterAnimation, WaterDrop, WetTexture, texture_poly_flags,
};
