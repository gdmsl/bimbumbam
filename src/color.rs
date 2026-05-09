//! Premultiplied-alpha RGBA colors and palette helpers used throughout the renderer.

use rand::Rng;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Returns the color premultiplied by alpha — the form expected by our
    /// `One, OneMinusSrcAlpha` blend equation in [`crate::gpu`].
    pub fn premul(&self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }
}

pub const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);

/// Hand-picked saturated palette that reads as friendly to a small child:
/// every entry is bright enough to pop on the dark background but never harsh.
pub const PALETTE: &[Color] = &[
    Color::new(1.0, 0.2, 0.3, 1.0),
    Color::new(1.0, 0.5, 0.0, 1.0),
    Color::new(1.0, 0.9, 0.0, 1.0),
    Color::new(0.2, 0.9, 0.2, 1.0),
    Color::new(0.2, 0.6, 1.0, 1.0),
    Color::new(0.6, 0.3, 1.0, 1.0),
    Color::new(1.0, 0.4, 0.7, 1.0),
    Color::new(0.0, 0.9, 0.9, 1.0),
    Color::new(1.0, 0.6, 0.4, 1.0),
    Color::new(0.5, 1.0, 0.3, 1.0),
];

pub fn random_color<R: Rng + ?Sized>(rng: &mut R) -> Color {
    PALETTE[rng.random_range(0..PALETTE.len())]
}

pub fn random_bright_color<R: Rng + ?Sized>(rng: &mut R) -> Color {
    Color::new(
        rng.random_range(0.4..1.0),
        rng.random_range(0.4..1.0),
        rng.random_range(0.4..1.0),
        1.0,
    )
}

/// HSL → RGB conversion. Hue is wrapped into [0, 1).
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    (
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    )
}

fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
    let t = ((t % 1.0) + 1.0) % 1.0;
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn premul_zero_alpha_is_black() {
        let c = Color::new(1.0, 1.0, 1.0, 0.0);
        assert_eq!(c.premul(), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn premul_full_alpha_is_identity() {
        let c = Color::new(0.25, 0.5, 0.75, 1.0);
        assert_eq!(c.premul(), [0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn premul_half_alpha() {
        let c = Color::new(0.4, 0.8, 1.0, 0.5);
        let p = c.premul();
        assert!(approx(p[0], 0.2));
        assert!(approx(p[1], 0.4));
        assert!(approx(p[2], 0.5));
        assert!(approx(p[3], 0.5));
    }

    #[test]
    fn hsl_zero_saturation_is_grey() {
        assert_eq!(hsl_to_rgb(0.5, 0.0, 0.6), (0.6, 0.6, 0.6));
    }

    #[test]
    fn hsl_red() {
        let (r, g, b) = hsl_to_rgb(0.0, 1.0, 0.5);
        assert!(approx(r, 1.0) && approx(g, 0.0) && approx(b, 0.0));
    }

    #[test]
    fn hsl_green() {
        let (r, g, b) = hsl_to_rgb(1.0 / 3.0, 1.0, 0.5);
        assert!(approx(r, 0.0) && approx(g, 1.0) && approx(b, 0.0));
    }

    #[test]
    fn hsl_blue() {
        let (r, g, b) = hsl_to_rgb(2.0 / 3.0, 1.0, 0.5);
        assert!(approx(r, 0.0) && approx(g, 0.0) && approx(b, 1.0));
    }

    #[test]
    fn hsl_hue_wraps() {
        // hue=1.0 should equal hue=0.0 due to modulo
        let a = hsl_to_rgb(0.0, 1.0, 0.5);
        let b = hsl_to_rgb(1.0, 1.0, 0.5);
        // Floating-point — close, not identical
        assert!((a.0 - b.0).abs() < 1e-4);
        assert!((a.1 - b.1).abs() < 1e-4);
        assert!((a.2 - b.2).abs() < 1e-4);
    }

    #[test]
    fn hsl_negative_hue_wraps() {
        let a = hsl_to_rgb(0.5, 1.0, 0.5);
        let b = hsl_to_rgb(-0.5, 1.0, 0.5);
        assert!((a.0 - b.0).abs() < 1e-4);
    }
}
