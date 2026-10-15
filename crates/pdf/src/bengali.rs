//! Bengali shaping and vector glyph rendering for PDF output.
//!
//! Uses embedded `NotoSansBengali-Regular.ttf`, shaped with `harfrust` and outlined with `skrifa`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::sync::OnceLock;

use harfrust::{Direction, FontRef as HbFontRef, ShapeOptions, ShaperData, UnicodeBuffer};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef as SkrifaFontRef, GlyphId, MetadataProvider};

use super::{Rgb, extent, finite, n, rgb_op};

pub static BENGALI_FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/NotoSansBengali-Regular.ttf");

/// True if `text` contains any Bengali Unicode character (U+0980..U+09FF).
pub fn has_bengali(text: &str) -> bool {
    text.chars().any(|c| ('\u{0980}'..='\u{09FF}').contains(&c))
}

struct FontContext {
    hb_font: HbFontRef<'static>,
    shaper_data: ShaperData,
    upem: f32,
}

fn get_context() -> Option<&'static FontContext> {
    static CTX: OnceLock<Option<FontContext>> = OnceLock::new();
    CTX.get_or_init(|| {
        let hb_font = HbFontRef::from_index(BENGALI_FONT_BYTES, 0).ok()?;
        let shaper_data = ShaperData::new(&hb_font);
        let skrifa_font = SkrifaFontRef::from_index(BENGALI_FONT_BYTES, 0).ok()?;
        let upem = skrifa_font.head().map(|h| h.units_per_em()).unwrap_or(1000) as f32;
        Some(FontContext { hb_font, shaper_data, upem: if upem > 0.0 { upem } else { 1000.0 } })
    })
    .as_ref()
}

pub struct ShapedGlyph {
    pub gid: u32,
    pub x_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// Shapes `text` with Noto Sans Bengali. Returns shaped glyphs and total advance width (in points).
pub fn shape_bengali(text: &str, size: f32) -> (Vec<ShapedGlyph>, f32) {
    let Some(ctx) = get_context() else {
        return (Vec::new(), 0.0);
    };
    let scale = size / ctx.upem;
    let shaper = ctx.shaper_data.shaper(&ctx.hb_font).build();
    let mut buf = UnicodeBuffer::new();
    for (i, c) in text.char_indices() {
        buf.add(c, i as u32);
    }
    buf.guess_segment_properties();
    buf.set_direction(Direction::LeftToRight);
    let shaped = shaper.shape(buf, ShapeOptions::default());

    let mut glyphs = Vec::with_capacity(shaped.len());
    let mut total_w = 0.0f32;
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        let adv = pos.x_advance as f32 * scale;
        total_w += adv;
        glyphs.push(ShapedGlyph { gid: info.glyph_id, x_advance: adv, x_offset: pos.x_offset as f32 * scale, y_offset: pos.y_offset as f32 * scale });
    }
    (glyphs, total_w)
}

/// Computes the advance width of Bengali text at `size` points.
pub fn bengali_text_width(text: &str, size: f32) -> f32 {
    shape_bengali(text, size).1
}

struct PdfPen<'a> {
    buf: &'a mut String,
    ox: f32,
    oy: f32,
    scale: f32,
    cos: f32,
    sin: f32,
    cur_x: f32,
    cur_y: f32,
}

impl<'a> PdfPen<'a> {
    fn transform(&self, x: f32, y: f32) -> (f32, f32) {
        let x_pt = x * self.scale;
        let y_pt = y * self.scale;
        let px = self.ox + x_pt * self.cos + y_pt * self.sin;
        let py = self.oy - x_pt * self.sin + y_pt * self.cos;
        (px, py)
    }
}

impl<'a> OutlinePen for PdfPen<'a> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (px, py) = self.transform(x, y);
        self.cur_x = px;
        self.cur_y = py;
        let _ = write!(self.buf, "{} {} m ", n(px), n(py));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (px, py) = self.transform(x, y);
        self.cur_x = px;
        self.cur_y = py;
        let _ = write!(self.buf, "{} {} l ", n(px), n(py));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (cx, cy) = self.transform(cx0, cy0);
        let (px, py) = self.transform(x, y);
        let cp1_x = self.cur_x + 2.0 / 3.0 * (cx - self.cur_x);
        let cp1_y = self.cur_y + 2.0 / 3.0 * (cy - self.cur_y);
        let cp2_x = px + 2.0 / 3.0 * (cx - px);
        let cp2_y = py + 2.0 / 3.0 * (cy - py);
        self.cur_x = px;
        self.cur_y = py;
        let _ = write!(self.buf, "{} {} {} {} {} {} c ", n(cp1_x), n(cp1_y), n(cp2_x), n(cp2_y), n(px), n(py));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (c1x, c1y) = self.transform(cx0, cy0);
        let (c2x, c2y) = self.transform(cx1, cy1);
        let (px, py) = self.transform(x, y);
        self.cur_x = px;
        self.cur_y = py;
        let _ = write!(self.buf, "{} {} {} {} {} {} c ", n(c1x), n(c1y), n(c2x), n(c2y), n(px), n(py));
    }

    fn close(&mut self) {
        self.buf.push_str("h ");
    }
}

/// Renders Bengali text rotated by `angle` (radians clockwise) about baseline origin `(x, y)`.
/// Returns PDF operators drawing vector glyph paths.
pub fn render_bengali_vector(x: f32, y_down: f32, page_height: f32, size: f32, color: Rgb, text: &str, angle: f32) -> Option<String> {
    if text.is_empty() || !extent(size) || !finite(&[x, y_down]) {
        return None;
    }
    let ctx = get_context()?;
    let skrifa_font = SkrifaFontRef::from_index(BENGALI_FONT_BYTES, 0).ok()?;
    let outlines = skrifa_font.outline_glyphs();

    let (glyphs, _total_w) = shape_bengali(text, size);
    if glyphs.is_empty() {
        return None;
    }

    let (sn, cs) = if angle.is_finite() { angle.sin_cos() } else { (0.0, 1.0) };
    let pdf_oy = page_height - y_down;
    let glyph_scale = size / ctx.upem;

    let mut buf = String::with_capacity(glyphs.len() * 128);
    buf.push_str("q\n");
    let _ = writeln!(buf, "{}", rgb_op(color, false));

    let mut pen_advance = 0.0f32;
    let mut has_paths = false;

    for g in glyphs {
        let gid = g.gid;
        let glyph_origin_x = x + (pen_advance + g.x_offset) * cs;
        let glyph_origin_y = pdf_oy - (pen_advance + g.x_offset) * sn + g.y_offset * cs;

        if let Some(outline) = outlines.get(GlyphId::new(gid)) {
            let mut pen =
                PdfPen { buf: &mut buf, ox: glyph_origin_x, oy: glyph_origin_y, scale: glyph_scale, cos: cs, sin: sn, cur_x: 0.0, cur_y: 0.0 };
            if outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).is_ok() {
                has_paths = true;
            }
        }
        pen_advance += g.x_advance;
    }

    if has_paths {
        buf.push_str("f\n");
    }
    buf.push('Q');
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_bengali() {
        assert!(has_bengali("বাংলাদেশ"));
        assert!(has_bengali("Hello বাংলা"));
        assert!(!has_bengali("Hello World 123"));
    }

    #[test]
    fn test_shape_bengali() {
        let (glyphs, w) = shape_bengali("বাংলাদেশ", 12.0);
        assert!(!glyphs.is_empty());
        assert!(w > 0.0);
    }

    #[test]
    fn test_render_bengali_vector() {
        let res = render_bengali_vector(50.0, 100.0, 792.0, 12.0, [0, 0, 0], "বাংলাদেশ কৃষি ব্যাংক", 0.0);
        assert!(res.is_some());
        let s = res.unwrap();
        assert!(s.starts_with("q\n"));
        assert!(s.ends_with("Q"));
        assert!(s.contains(" m "));
        assert!(s.contains(" c "));
        assert!(s.contains("f\n"));
    }
}
