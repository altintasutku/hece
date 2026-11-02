//! Anti-aliased one-pixel lines (Xiaolin Wu) and small discs blended straight into a pixmap. The
//! sphere has 480 edges and dozens of dots per frame; going through tiny-skia's path pipeline for
//! each costs several times more than the rest of the frame.

use tiny_skia::{Pixmap, PremultipliedColorU8};

/// Draws a line from `from` to `to` (pixel coordinates) in `rgb` at opacity `alpha`, blended
/// over what is there. Parts outside the pixmap are skipped.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "pixel coordinates of a pixmap a few hundred pixels wide"
)]
pub fn line(pixmap: &mut Pixmap, from: (f32, f32), to: (f32, f32), rgb: [f32; 3], alpha: f32) {
    let (mut x0, mut y0) = from;
    let (mut x1, mut y1) = to;
    let steep = (y1 - y0).abs() > (x1 - x0).abs();
    if steep {
        std::mem::swap(&mut x0, &mut y0);
        std::mem::swap(&mut x1, &mut y1);
    }
    if x0 > x1 {
        std::mem::swap(&mut x0, &mut x1);
        std::mem::swap(&mut y0, &mut y1);
    }
    let dx = x1 - x0;
    let gradient = if dx < f32::EPSILON {
        0.0
    } else {
        (y1 - y0) / dx
    };
    // Only the columns that can land in the pixmap.
    let limit = if steep {
        pixmap.height()
    } else {
        pixmap.width()
    } as f32;
    let start = x0.round().max(0.0);
    let end = x1.round().min(limit - 1.0);
    if start > end {
        return;
    }
    let mut canvas = Canvas::new(pixmap);
    let mut y = y0 + gradient * (start - x0);
    let (first, last) = (start as i32, end as i32);
    for x in first..=last {
        let row = y.floor();
        let frac = y - row;
        let row = row as i32;
        for (r, coverage) in [(row, 1.0 - frac), (row + 1, frac)] {
            let (px, py) = if steep { (r, x) } else { (x, r) };
            canvas.blend(px, py, rgb, alpha * coverage);
        }
        y += gradient;
    }
}

/// Fills a disc of `radius` around `center` (pixel coordinates), with a one-pixel soft edge.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "pixel coordinates of a pixmap a few hundred pixels wide"
)]
pub fn disc(pixmap: &mut Pixmap, center: (f32, f32), radius: f32, rgb: [f32; 3], alpha: f32) {
    let (cx, cy) = center;
    let reach = radius + 1.0;
    let mut canvas = Canvas::new(pixmap);
    for y in (cy - reach).floor() as i32..=(cy + reach).ceil() as i32 {
        for x in (cx - reach).floor() as i32..=(cx + reach).ceil() as i32 {
            // Distance from the pixel centre to the disc's edge decides the coverage.
            let distance = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                canvas.blend(x, y, rgb, alpha * coverage);
            }
        }
    }
}

/// The pixels of one pixmap, borrowed once per shape.
struct Canvas<'a> {
    pixels: &'a mut [PremultipliedColorU8],
    width: u32,
    height: u32,
}

impl<'a> Canvas<'a> {
    fn new(pixmap: &'a mut Pixmap) -> Self {
        let (width, height) = (pixmap.width(), pixmap.height());
        Self {
            pixels: pixmap.pixels_mut(),
            width,
            height,
        }
    }

    /// Source-over of one pixel; `alpha` is the coverage-weighted opacity. Coverage too small
    /// to change an 8-bit pixel is skipped.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "values are clamped to 0..=255 first"
    )]
    fn blend(&mut self, x: i32, y: i32, rgb: [f32; 3], alpha: f32) {
        if alpha < 0.5 / 255.0 {
            return;
        }
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        if x >= self.width || y >= self.height {
            return;
        }
        let Some(pixel) = self.pixels.get_mut((y * self.width + x) as usize) else {
            return;
        };
        blend_pixel(pixel, rgb, alpha);
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to 0..=255 first"
)]
fn blend_pixel(pixel: &mut PremultipliedColorU8, rgb: [f32; 3], alpha: f32) {
    let a = alpha.min(1.0);
    let keep = 1.0 - a;
    let to_u8 = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    let out_a = to_u8(a * 255.0 + f32::from(pixel.alpha()) * keep);
    let channel = |src: f32, dst: u8| to_u8(src * a * 255.0 + f32::from(dst) * keep).min(out_a);
    let red = channel(rgb[0], pixel.red());
    let green = channel(rgb[1], pixel.green());
    let blue = channel(rgb[2], pixel.blue());
    if let Some(color) = PremultipliedColorU8::from_rgba(red, green, blue, out_a) {
        *pixel = color;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(pixmap: &Pixmap, x: u32, y: u32) -> u8 {
        pixmap.pixel(x, y).map_or(0, PremultipliedColorU8::alpha)
    }

    #[test]
    fn horizontal_line_covers_its_pixels_only() {
        let mut pixmap = Pixmap::new(20, 20).unwrap();
        line(&mut pixmap, (2.0, 10.0), (17.0, 10.0), [1.0, 0.0, 0.0], 1.0);
        for x in 3..17 {
            assert!(alpha_at(&pixmap, x, 10) > 200, "x={x}");
        }
        assert_eq!(alpha_at(&pixmap, 10, 2), 0);
        assert_eq!(alpha_at(&pixmap, 19, 10), 0);
    }

    #[test]
    fn steep_diagonal_line_is_continuous() {
        let mut pixmap = Pixmap::new(20, 20).unwrap();
        line(&mut pixmap, (3.0, 1.0), (8.0, 18.0), [0.0, 1.0, 0.0], 1.0);
        for y in 2..18 {
            let covered = (0..20)
                .map(|x| u32::from(alpha_at(&pixmap, x, y)))
                .sum::<u32>();
            assert!(covered > 150, "y={y}");
        }
    }

    #[test]
    fn blending_keeps_premultiplied_pixels_valid() {
        let mut pixmap = Pixmap::new(10, 10).unwrap();
        for _ in 0..5 {
            line(&mut pixmap, (0.0, 4.3), (9.0, 5.6), [0.4, 0.9, 1.0], 0.6);
        }
        for p in pixmap.pixels() {
            assert!(p.red() <= p.alpha() && p.green() <= p.alpha() && p.blue() <= p.alpha());
        }
    }

    #[test]
    fn disc_covers_its_area_only() {
        let mut pixmap = Pixmap::new(20, 20).unwrap();
        disc(&mut pixmap, (10.0, 10.0), 3.0, [1.0, 1.0, 0.0], 1.0);
        assert!(alpha_at(&pixmap, 10, 10) > 240);
        assert!(alpha_at(&pixmap, 12, 10) > 200);
        assert_eq!(alpha_at(&pixmap, 15, 10), 0);
        assert_eq!(alpha_at(&pixmap, 10, 4), 0);
    }

    #[test]
    fn lines_outside_are_clipped() {
        let mut pixmap = Pixmap::new(10, 10).unwrap();
        line(
            &mut pixmap,
            (-50.0, -3.0),
            (60.0, 40.0),
            [1.0, 1.0, 1.0],
            1.0,
        );
    }
}
