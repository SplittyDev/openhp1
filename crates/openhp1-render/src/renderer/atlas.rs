use openhp1_scene::{LightmapImage, TextureImage};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AtlasRectangle {
    pub(super) x: u32,
    pub(super) y: u32,
    width: u32,
    height: u32,
}

pub(super) struct LightmapAtlas {
    pub(super) image: TextureImage,
    pub(super) rectangles: Vec<AtlasRectangle>,
    neutral: AtlasRectangle,
}

impl LightmapAtlas {
    pub(super) fn neutral_coordinates(&self) -> [f32; 2] {
        [
            (self.neutral.x as f32 + 0.5) / self.image.width as f32,
            (self.neutral.y as f32 + 0.5) / self.image.height as f32,
        ]
    }
}

#[derive(Clone, Copy)]
struct AtlasItem {
    source: Option<usize>,
    width: u32,
    height: u32,
}

pub(super) fn build_lightmap_atlas(
    lightmaps: &[LightmapImage],
    maximum_dimension: u32,
) -> LightmapAtlas {
    let mut items = Vec::with_capacity(lightmaps.len() + 1);
    items.push(AtlasItem {
        source: None,
        width: 1,
        height: 1,
    });
    items.extend(
        lightmaps
            .iter()
            .enumerate()
            .map(|(source, image)| AtlasItem {
                source: Some(source),
                width: image.width,
                height: image.height,
            }),
    );
    items.sort_unstable_by_key(|item| std::cmp::Reverse(item.height));

    let widest = items.iter().map(|item| item.width + 2).max().unwrap_or(3);
    let mut atlas_width = widest.next_power_of_two().max(512).min(maximum_dimension);
    let (placements, atlas_height) = loop {
        if let Some(result) = pack_atlas(&items, atlas_width, maximum_dimension) {
            break result;
        }
        assert!(
            atlas_width < maximum_dimension,
            "lightmaps exceed the GPU's {maximum_dimension}px texture limit"
        );
        atlas_width = (atlas_width * 2).min(maximum_dimension);
    };

    let mut rgba = vec![128; atlas_width as usize * atlas_height as usize * 4];
    for alpha in rgba.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    let mut rectangles = vec![AtlasRectangle::default(); lightmaps.len()];
    let mut neutral = AtlasRectangle::default();
    for (item, rectangle) in items.iter().zip(placements) {
        match item.source {
            Some(source) => {
                copy_with_gutter(&mut rgba, atlas_width, rectangle, &lightmaps[source].rgba);
                rectangles[source] = rectangle;
            }
            None => neutral = rectangle,
        }
    }
    LightmapAtlas {
        image: TextureImage {
            width: atlas_width,
            height: atlas_height,
            rgba,
        },
        rectangles,
        neutral,
    }
}

fn pack_atlas(
    items: &[AtlasItem],
    atlas_width: u32,
    maximum_height: u32,
) -> Option<(Vec<AtlasRectangle>, u32)> {
    let mut placements = Vec::with_capacity(items.len());
    let (mut x, mut y, mut row_height) = (0, 0, 0);
    for item in items {
        let padded_width = item.width + 2;
        let padded_height = item.height + 2;
        if padded_width > atlas_width {
            return None;
        }
        if x + padded_width > atlas_width {
            x = 0;
            y += row_height;
            row_height = 0;
        }
        if y + padded_height > maximum_height {
            return None;
        }
        placements.push(AtlasRectangle {
            x: x + 1,
            y: y + 1,
            width: item.width,
            height: item.height,
        });
        x += padded_width;
        row_height = row_height.max(padded_height);
    }
    Some((placements, (y + row_height).max(1)))
}

fn copy_with_gutter(atlas: &mut [u8], atlas_width: u32, rectangle: AtlasRectangle, source: &[u8]) {
    for target_y in rectangle.y - 1..=rectangle.y + rectangle.height {
        let source_y = target_y
            .saturating_sub(rectangle.y)
            .min(rectangle.height - 1);
        for target_x in rectangle.x - 1..=rectangle.x + rectangle.width {
            let source_x = target_x
                .saturating_sub(rectangle.x)
                .min(rectangle.width - 1);
            let source_offset = ((source_y * rectangle.width + source_x) * 4) as usize;
            let target_offset = ((target_y * atlas_width + target_x) * 4) as usize;
            atlas[target_offset..target_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
        }
    }
}
