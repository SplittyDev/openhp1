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

/// Identifies authored window texture paths without deciding whether they may
/// emit light in a particular renderer.
pub fn is_window_texture_name(name: &str) -> bool {
    let mut segments = name.split('.');
    let _package = segments.next();
    let Some(object) = segments.next_back() else {
        return false;
    };
    object.to_ascii_lowercase().contains("window")
        || segments.any(|segment| {
            ["window", "windows", "window frame", "window frames"]
                .iter()
                .any(|group| segment.eq_ignore_ascii_case(group))
        })
}

#[cfg(test)]
mod tests {
    use super::is_window_texture_name;

    #[test]
    fn identifies_window_groups_and_object_names() {
        assert!(is_window_texture_name("HP_K.Window Frames.Win14_T1"));
        assert!(is_window_texture_name("HP_Outside.windows.Exteriorwindow"));
        assert!(is_window_texture_name("HP_C.Detail.gryffindorwindow"));
        assert!(!is_window_texture_name("HPBase.FXPackage.Spells.WIN_A"));
        assert!(!is_window_texture_name(
            "Hub2_Greenhouse.Wall.Greenhousewall"
        ));
    }
}
