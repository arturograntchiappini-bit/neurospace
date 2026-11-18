//! The Second Brain logo: the window icon and the texture of the welcome screen.

use std::cell::OnceCell;

use eframe::egui::{ColorImage, Context, IconData, TextureHandle, TextureOptions};

/// The 256 px rendering of `assets/logo/second-brain.svg`.
const LOGO_PNG: &[u8] = include_bytes!("../../../../assets/logo/second-brain-256.png");

fn decode() -> Result<IconData, String> {
    eframe::icon_data::from_png_bytes(LOGO_PNG).map_err(|error| error.to_string())
}

/// The window and taskbar icon; `None` (after a note on stderr) when the logo cannot be decoded.
pub fn window_icon() -> Option<IconData> {
    decode()
        .inspect_err(|error| eprintln!("secondbrain: no window icon: {error}"))
        .ok()
}

/// The logo as a texture, decoded once on first use and kept for the life of the application.
#[derive(Default)]
pub struct Logo {
    texture: OnceCell<Option<TextureHandle>>,
}

impl Logo {
    /// The texture, or `None` when the logo cannot be decoded (the screen then shows no logo).
    pub fn texture(&self, ctx: &Context) -> Option<&TextureHandle> {
        self.texture
            .get_or_init(|| {
                let icon = window_icon()?;
                let size = [icon.width as usize, icon.height as usize];
                let image = ColorImage::from_rgba_unmultiplied(size, &icon.rgba);
                Some(ctx.load_texture("logo", image, TextureOptions::LINEAR))
            })
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_decode_embedded_logo_as_256_square_rgba_with_transparent_corners() {
        let icon = decode().unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert_eq!(icon.rgba[3], 0, "the top-left pixel is outside the tile");
    }

    #[test]
    fn should_load_logo_texture_once_when_asked_twice() {
        let ctx = Context::default();
        let logo = Logo::default();
        let first = logo.texture(&ctx).unwrap().id();
        assert_eq!(logo.texture(&ctx).unwrap().id(), first);
    }
}
