//! ComputerCraft terminal bitmap-font atlas.
//!
//! Ported from `@squid-dev/cc-web-term/dist/terminal/render.js`:
//!
//! ```text
//! cellWidth  = 6
//! cellHeight = 9
//! font.scale = font.margin = image.width / 256;    // 1 for 256×256, 2 for 512×512
//! imageW = cellWidth  * font.scale
//! imageH = cellHeight * font.scale
//! imgX = font.margin + (char % 16) * (imageW + font.margin * 2)
//! imgY = font.margin + (char / 16) * (imageH + font.margin * 2)
//! ```
//!
//! The atlas glyphs are white-on-transparent so we blit with a tint for the
//! foreground colour and no per-colour pre-rendering (egui handles the RGB
//! multiply for us).

use egui::{Color32, ColorImage, Context, Rect, TextureHandle, TextureId, TextureOptions};
use once_cell::sync::OnceCell;

pub const CELL_W: f32 = 6.0;
pub const CELL_H: f32 = 9.0;
pub const TERMINAL_MARGIN: f32 = 4.0;

const FONT_STANDARD: &[u8] = include_bytes!("../assets/term_font.png");
const FONT_HD: &[u8] = include_bytes!("../assets/term_font_hd.png");

pub struct TerminalFont {
    pub texture: TextureHandle,
    /// Atlas-native scale: 1 for the 256×256 standard atlas, 2 for the HD one.
    pub scale: f32,
    pub image_w: f32,
    pub image_h: f32,
}

impl TerminalFont {
    /// `hd = true` uses the 512×512 atlas, which looks much less blurry when
    /// the terminal is rendered at anything above 1x.
    pub fn load(ctx: &Context, hd: bool) -> Self {
        let bytes = if hd { FONT_HD } else { FONT_STANDARD };
        let decoded = image::load_from_memory(bytes)
            .expect("bundled atlas is malformed — rebuild desktop crate")
            .to_rgba8();
        let (w, h) = decoded.dimensions();

        // Ensure glyph alpha comes from a white-on-transparent source. The
        // standard atlas is paletted; after RGBA conversion the glyph pixels
        // are already white so we can use the image as-is.
        let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], decoded.as_raw());

        let texture = ctx.load_texture(
            if hd { "term_font_hd" } else { "term_font" },
            image,
            TextureOptions::NEAREST,
        );

        let scale = (w as f32) / 256.0;
        Self {
            texture,
            scale,
            image_w: CELL_W * scale,
            image_h: CELL_H * scale,
        }
    }

    /// UV rect of a single character in normalized [0,1] atlas coordinates.
    pub fn uv_for(&self, code: u8) -> Rect {
        let margin = self.scale;
        let step_x = self.image_w + margin * 2.0;
        let step_y = self.image_h + margin * 2.0;

        let img_x = margin + (code % 16) as f32 * step_x;
        let img_y = margin + (code / 16) as f32 * step_y;

        let tex_w = (self.texture.size()[0]) as f32;
        let tex_h = (self.texture.size()[1]) as f32;
        Rect::from_min_size(
            egui::pos2(img_x / tex_w, img_y / tex_h),
            egui::vec2(self.image_w / tex_w, self.image_h / tex_h),
        )
    }

    pub fn texture_id(&self) -> TextureId { self.texture.id() }
}

// ---- CC default palette (index '0'..'f') ---------------------------------
//
// Values lifted from cc-web-term's `data.ts` / CC's own default palette. We
// consult this only when the server hasn't sent its own palette entry for a
// code yet.

pub fn default_palette(code: char) -> Color32 {
    match code.to_ascii_lowercase() {
        '0' => Color32::from_rgb(240, 240, 240),
        '1' => Color32::from_rgb(242, 178,  51),
        '2' => Color32::from_rgb(229, 127, 216),
        '3' => Color32::from_rgb(153, 178, 242),
        '4' => Color32::from_rgb(222, 222, 108),
        '5' => Color32::from_rgb(127, 204,  25),
        '6' => Color32::from_rgb(242, 178, 204),
        '7' => Color32::from_rgb( 76,  76,  76),
        '8' => Color32::from_rgb(153, 153, 153),
        '9' => Color32::from_rgb( 76, 153, 178),
        'a' => Color32::from_rgb(178, 102, 229),
        'b' => Color32::from_rgb( 37,  49, 146),
        'c' => Color32::from_rgb(127, 102,  76),
        'd' => Color32::from_rgb( 87, 166,  78),
        'e' => Color32::from_rgb(204,  76,  76),
        'f' => Color32::from_rgb( 17,  17,  17),
        _   => Color32::from_rgb(240, 240, 240),
    }
}

// ---- Process-lifetime cache ---------------------------------------------
//
// We load the atlas once per egui::Context. Storing it in the Context's
// `data()` memory (egui's pattern for per-context singletons) would also
// work; a `OnceCell` keyed by a pointer is simpler.

static TERMINAL_FONT: OnceCell<TerminalFont> = OnceCell::new();

pub fn get(ctx: &Context, hd: bool) -> &'static TerminalFont {
    TERMINAL_FONT.get_or_init(|| TerminalFont::load(ctx, hd))
}
