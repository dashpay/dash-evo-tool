use egui::{Context, TextureHandle};
use rust_embed::RustEmbed;
use tracing::error;

#[derive(RustEmbed)]
#[folder = "icons/"]
struct Assets;

/// Load a raster icon from the embedded `icons/` assets, caching the resulting
/// texture in the egui context so repeated frames reuse it.
pub(crate) fn load_icon(ctx: &Context, path: &str) -> Option<TextureHandle> {
    ctx.data_mut(|d| d.get_temp::<TextureHandle>(egui::Id::new(path)))
        .or_else(|| {
            let content = Assets::get(path).or_else(|| {
                error!("Image not found in embedded assets at path: {}", path);
                None
            })?;
            match image::load_from_memory(&content.data) {
                Ok(image) => {
                    let size = [image.width() as usize, image.height() as usize];
                    let pixels = image.into_rgba8().into_raw();
                    let texture = ctx.load_texture(
                        path,
                        egui::ColorImage::from_rgba_unmultiplied(size, &pixels),
                        egui::TextureOptions::LINEAR,
                    );
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new(path), texture.clone()));
                    Some(texture)
                }
                Err(e) => {
                    error!("Failed to load image from embedded data at {}: {}", path, e);
                    None
                }
            }
        })
}

/// Load an SVG from the embedded `icons/` assets, rasterized to `width`×`height`
/// and cached per dimension in the egui context.
pub fn load_svg_icon(ctx: &Context, path: &str, width: u32, height: u32) -> Option<TextureHandle> {
    let cache_key = format!("{}_{}_{}", path, width, height);
    ctx.data_mut(|d| d.get_temp::<TextureHandle>(egui::Id::new(&cache_key)))
        .or_else(|| {
            let content = Assets::get(path).or_else(|| {
                error!("SVG not found in embedded assets at path: {}", path);
                None
            })?;

            let options = resvg::usvg::Options::default();
            let tree = match resvg::usvg::Tree::from_data(&content.data, &options) {
                Ok(tree) => tree,
                Err(e) => {
                    error!("Failed to parse SVG at {}: {}", path, e);
                    return None;
                }
            };

            let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;

            // Scale the SVG to fit the target box and center it.
            let svg_size = tree.size();
            let scale = (width as f32 / svg_size.width()).min(height as f32 / svg_size.height());
            let offset_x = (width as f32 - svg_size.width() * scale) / 2.0;
            let offset_y = (height as f32 - svg_size.height() * scale) / 2.0;
            let transform = resvg::tiny_skia::Transform::from_scale(scale, scale)
                .post_translate(offset_x, offset_y);

            resvg::render(&tree, transform, &mut pixmap.as_mut());

            let pixels = pixmap.data().to_vec();
            let texture = ctx.load_texture(
                &cache_key,
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &pixels,
                ),
                egui::TextureOptions::LINEAR,
            );
            ctx.data_mut(|d| d.insert_temp(egui::Id::new(&cache_key), texture.clone()));
            Some(texture)
        })
}
