//! 2D drawing in the original's virtual coordinate system (800 units wide, y down). This is a
//! port of the Java `P.j` sprite batcher, the two bitmap fonts (`P.f` / `P.e`) and the Android
//! `Canvas`-rendered text (`P.h`), so every screen can be laid out with the original numbers.

use macroquad::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct Spr {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Layout state for the current frame. The virtual width is always 800; windows narrower than
/// 4:3 are letter-boxed so the original layouts (designed for 16:9 phones) never get stretched.
pub struct Gfx {
    pub k: f32,
    pub ox: f32,
    pub oy: f32,
    /// virtual height (g_fSH)
    pub sh: f32,
    pub tint: Color,
    /// pixel rectangle of the layout area inside the window (logical px)
    pub vp: (f32, f32, f32, f32),
}

impl Gfx {
    pub fn new() -> Gfx {
        let mut g = Gfx { k: 1.0, ox: 0.0, oy: 0.0, sh: 450.0, tint: WHITE, vp: (0.0, 0.0, 800.0, 450.0) };
        g.layout();
        g
    }

    pub fn layout(&mut self) {
        let (sw, sh) = (screen_width(), screen_height());
        let (vw, vh) = if sw / sh >= 4.0 / 3.0 { (sw, sh) } else { (sw, sw * 0.75) };
        self.k = vw / 800.0;
        self.ox = 0.0;
        self.oy = (sh - vh) / 2.0;
        self.sh = vh / self.k;
        self.vp = (0.0, self.oy, vw, vh);
    }

    /// virtual centre helpers (g_fCW, g_fCH)
    pub fn cw(&self) -> f32 {
        400.0
    }
    pub fn ch(&self) -> f32 {
        self.sh / 2.0
    }

    /// Pointer position in the window (logical px) -> virtual coordinates.
    pub fn to_virtual(&self, px: f32, py: f32) -> (f32, f32) {
        ((px - self.ox) / self.k, (py - self.oy) / self.k)
    }

    /// Viewport for a 3D camera, in *physical* pixels with a bottom-left origin, expressed as
    /// fractions of the layout area: (x, y_from_bottom, w, h) in virtual units.
    pub fn viewport_px(&self, vx: f32, vy_from_bottom: f32, vw: f32, vh: f32) -> (i32, i32, i32, i32) {
        let d = screen_dpi_scale();
        let bottom = screen_height() - (self.oy + self.sh * self.k); // gap under layout (letterbox)
        (
            ((self.ox + vx * self.k) * d) as i32,
            ((bottom + vy_from_bottom * self.k) * d) as i32,
            (vw * self.k * d) as i32,
            (vh * self.k * d) as i32,
        )
    }

    pub fn set_tint(&mut self, c: Color) {
        self.tint = c;
    }
    pub fn clear_tint(&mut self) {
        self.tint = WHITE;
    }

    /// Core blit: top-left + size in virtual units, optional horizontal flip and rotation about
    /// the centre (degrees, positive = clockwise on screen).
    #[allow(clippy::too_many_arguments)]
    pub fn blit(&self, tex: &Texture2D, s: Spr, left: f32, top: f32, w: f32, h: f32, flip_x: bool, rot: f32) {
        let k = self.k;
        draw_texture_ex(
            tex,
            self.ox + left * k,
            self.oy + top * k,
            self.tint,
            DrawTextureParams {
                dest_size: Some(vec2(w * k, h * k)),
                source: Some(Rect::new(s.x, s.y, s.w, s.h)),
                rotation: rot.to_radians(),
                flip_x,
                pivot: Some(vec2(self.ox + (left + w / 2.0) * k, self.oy + (top + h / 2.0) * k)),
                ..Default::default()
            },
        );
    }

    /// `batcher.d(x, y, sx, sy, align, sprite)`: scale multipliers; align bit 0-1 (1 = x is the
    /// left edge, 2 = right edge) and bit 2-3 (4 = y is the top edge, 8 = bottom edge).
    pub fn d(&self, tex: &Texture2D, s: Spr, x: f32, y: f32, sx: f32, sy: f32, align: i32) {
        let hw = s.w * 0.5 * sx.abs();
        let hh = s.h * 0.5 * sy.abs();
        let mut left = x - hw;
        let mut top = y - hh;
        match align & 3 {
            1 => left += hw,
            2 => left -= hw,
            _ => {}
        }
        match align & 12 {
            4 => top += hh,
            8 => top -= hh,
            _ => {}
        }
        // a negative x scale mirrors the sprite about its anchor
        self.blit(tex, s, left, top, hw * 2.0, hh * 2.0, sx < 0.0, 0.0);
    }

    /// `batcher.h`: centred, unscaled.
    pub fn h(&self, tex: &Texture2D, s: Spr, x: f32, y: f32) {
        self.d(tex, s, x, y, 1.0, 1.0, 0);
    }
    /// `batcher.e`: centred, scaled.
    pub fn e(&self, tex: &Texture2D, s: Spr, x: f32, y: f32, sx: f32, sy: f32) {
        self.d(tex, s, x, y, sx, sy, 0);
    }
    /// `batcher.f`: centred, rotated.
    pub fn f(&self, tex: &Texture2D, s: Spr, x: f32, y: f32, rot: f32) {
        self.blit(tex, s, x - s.w / 2.0, y - s.h / 2.0, s.w, s.h, false, rot);
    }
    /// `batcher.b`: centred, scaled and rotated.
    pub fn b(&self, tex: &Texture2D, s: Spr, x: f32, y: f32, sx: f32, sy: f32, rot: f32) {
        let (w, h) = (s.w * sx, s.h * sy);
        self.blit(tex, s, x - w / 2.0, y - h / 2.0, w, h, false, rot);
    }
}

// ------------------------------------------------------------------ bitmap fonts

const ROW_A: [f32; 49] = [
    12.0, 25.0, 43.0, 64.0, 86.0, 116.0, 140.0, 152.0, 166.0, 182.0, 195.0, 217.0, 230.0, 246.0, 256.0, 271.0, 294.0, 316.0, 337.0, 359.0, 381.0, 403.0, 425.0,
    447.0, 469.0, 492.0, 506.0, 518.0, 540.0, 562.0, 583.0, 601.0, 627.0, 651.0, 673.0, 696.0, 720.0, 742.0, 761.0, 786.0, 809.0, 823.0, 843.0, 866.0, 887.0,
    915.0, 939.0, 963.0, 984.0,
];
const ROW_B: [f32; 47] = [
    10.0, 35.0, 57.0, 79.0, 101.0, 124.0, 145.0, 172.0, 193.0, 214.0, 235.0, 247.0, 262.0, 277.0, 303.0, 329.0, 352.0, 372.0, 391.0, 410.0, 430.0, 449.0, 464.0,
    484.0, 503.5, 516.0, 531.0, 549.0, 562.0, 591.0, 610.0, 630.0, 650.0, 671.0, 684.0, 702.0, 717.0, 736.0, 754.0, 778.0, 795.0, 813.0, 830.0, 846.0, 857.0,
    872.0, 893.0,
];

/// `P.f` (HUD font in ui.png) and `P.e` (large font in menu.png): 94 glyphs from `!` to `~`.
pub struct BFont {
    glyph: Vec<(Spr, f32)>,
}

impl BFont {
    fn build(y_a: f32, y_b: f32, dx: f32) -> BFont {
        let mut glyph = Vec::new();
        for i in 0..94usize {
            if i < 48 {
                let w = (ROW_A[i + 1] - ROW_A[i]) - 5.0;
                glyph.push((Spr { x: ROW_A[i] + dx, y: y_a, w, h: 30.0 }, w));
            } else {
                let j = i - 48;
                let w = (ROW_B[j + 1] - ROW_B[j]) - 5.0;
                glyph.push((Spr { x: ROW_B[j] + dx, y: y_b, w, h: 30.0 }, w));
            }
        }
        BFont { glyph }
    }
    /// HUD font (ui.png)
    pub fn ui() -> BFont {
        Self::build(514.0, 546.0, 0.0)
    }
    /// Big menu font (menu.png)
    pub fn menu() -> BFont {
        Self::build(147.0, 179.0, -1.0)
    }

    fn idx(c: char) -> Option<usize> {
        let i = c as i32 - '!' as i32;
        if (0..94).contains(&i) {
            Some(i as usize)
        } else {
            None
        }
    }

    /// `a`: left-aligned, unscaled, `y` is the vertical centre.
    pub fn a(&self, g: &Gfx, tex: &Texture2D, s: &str, mut x: f32, y: f32) {
        for c in s.chars() {
            let i = c as i32 - '!' as i32;
            let adv = if i >= 0 {
                match Self::idx(c) {
                    Some(ix) => {
                        g.d(tex, self.glyph[ix].0, x, y, 1.0, 1.0, 1);
                        self.glyph[ix].1
                    }
                    None => 12.0,
                }
            } else {
                12.0
            };
            x += adv;
        }
    }

    /// `b`: left-aligned, scaled.
    pub fn b(&self, g: &Gfx, tex: &Texture2D, s: &str, mut x: f32, y: f32, sc: f32) {
        for c in s.chars() {
            let i = c as i32 - '!' as i32;
            let adv = if i >= 0 {
                match Self::idx(c) {
                    Some(ix) => {
                        g.d(tex, self.glyph[ix].0, x, y, sc, sc, 1);
                        self.glyph[ix].1 * sc
                    }
                    None => sc * 12.0,
                }
            } else {
                sc * 12.0
            };
            x += adv;
        }
    }

    /// `c`: centred, unscaled.
    pub fn c(&self, g: &Gfx, tex: &Texture2D, s: &str, x: f32, y: f32) {
        if s.is_empty() {
            return;
        }
        let mut x0 = x + 2.0;
        for c in s.chars() {
            x0 -= match Self::idx(c) {
                Some(i) => self.glyph[i].1 / 2.0,
                None => 6.0,
            };
        }
        self.a(g, tex, s, x0, y);
    }

    /// `d`: centred, scaled.
    pub fn d(&self, g: &Gfx, tex: &Texture2D, s: &str, x: f32, y: f32, sc: f32) {
        if s.is_empty() {
            return;
        }
        let mut x0 = x + sc * 2.0;
        for c in s.chars() {
            x0 -= match Self::idx(c) {
                Some(i) => self.glyph[i].1 * sc / 2.0,
                None => 6.0 * sc,
            };
        }
        self.b(g, tex, s, x0, y, sc);
    }

    /// `e`: right-aligned, unscaled.
    pub fn e(&self, g: &Gfx, tex: &Texture2D, s: &str, x: f32, y: f32) {
        if s.is_empty() {
            return;
        }
        let mut x0 = x;
        for c in s.chars() {
            x0 -= match Self::idx(c) {
                Some(i) => self.glyph[i].1,
                None => 12.0,
            };
        }
        self.a(g, tex, s, x0, y);
    }

    /// `f`: right-aligned, scaled.
    pub fn f(&self, g: &Gfx, tex: &Texture2D, s: &str, x: f32, y: f32, sc: f32) {
        if s.is_empty() {
            return;
        }
        let mut x0 = x;
        for c in s.chars() {
            x0 -= match Self::idx(c) {
                Some(i) => self.glyph[i].1 * sc,
                None => 12.0,
            };
        }
        self.b(g, tex, s, x0, y, sc);
    }
}

// ------------------------------------------------------------------ system-font text (P.h)

/// Text drawn like the original's `Canvas`-rendered bitmaps: Roboto, white × the current tint,
/// `size` in virtual units, vertical centre at `y` (baseline sits 0.342·size below it, from
/// Roboto's ascent/descent), shrunk when its half-width would exceed `max_w`.
pub struct GlText {
    pub font: Option<Font>,
}

impl GlText {
    pub fn d(&self, g: &Gfx, s: &str, x: f32, y: f32, align: i32, size: f32) -> f32 {
        self.e(g, s, x, y, align, size, 380.0)
    }

    /// Returns the (half) drawn width, like the original.
    pub fn e(&self, g: &Gfx, s: &str, x: f32, y: f32, align: i32, size: f32, max_w: f32) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let px = (size * g.k).round().max(1.0);
        let dim = measure_text(s, self.font.as_ref(), px as u16, 1.0);
        let w_v = dim.width / g.k;
        let half = w_v / 2.0;
        let shrink = if half > max_w { max_w / half } else { 1.0 };
        let size_px = (px * shrink).max(1.0);
        let w = w_v * shrink;
        let left = match align & 3 {
            1 => x,
            2 => x - w,
            _ => x - w / 2.0,
        };
        let baseline = y + 0.3418 * size * shrink;
        draw_text_ex(
            s,
            g.ox + left * g.k,
            g.oy + baseline * g.k,
            TextParams { font: self.font.as_ref(), font_size: size_px as u16, color: g.tint, ..Default::default() },
        );
        half * shrink
    }
}
