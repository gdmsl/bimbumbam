//! CPU-side geometry batching and frame composition.
//!
//! [`DrawBatch`] accumulates triangles in a single vertex/index buffer that the
//! GPU draws in one call per frame. [`build_frame`] walks the live `Effect`
//! and `Particle` lists and writes geometry + a list of [`FrameText`]s for the
//! glyphon text renderer to handle.

use crate::color::{Color, WHITE};
use crate::effect::{Effect, EffectKind};
use crate::particle::{Particle, ParticleShape};
use bytemuck::{Pod, Zeroable};

/// Maximum vertices/indices the renderer will emit in one frame. Bounded so
/// the GPU buffers can be sized statically. The simulation caps `Effect`s and
/// `Particle`s well under what would saturate these.
pub const MAX_VERTICES: usize = 65_536;
pub const MAX_INDICES: usize = 131_072;

/// Tessellation segments used for filled circles and ring strokes.
const CIRCLE_SEGMENTS: u32 = 32;

/// Soft full-screen flashes are clamped at this peak alpha to avoid strobing
/// when a key (e.g. space bar) is held with auto-repeat.
const FLASH_PEAK_ALPHA: f32 = 0.18;

/// UV coordinate sampling the center of the 1×1 white texture used as the
/// shape/text atlas fallback.
const WHITE_UV: [f32; 2] = [0.5, 0.5];

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub color: [f32; 4],
    pub uv: [f32; 2],
}

#[derive(Debug)]
pub struct FrameText {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub color: Color,
}

pub struct DrawBatch {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl DrawBatch {
    pub fn new() -> Self {
        Self {
            vertices: Vec::with_capacity(MAX_VERTICES),
            indices: Vec::with_capacity(MAX_INDICES),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: Color) {
        if r <= 0.5 || c.a <= 0.0 {
            return;
        }
        let pm = c.premul();
        let base = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            pos: [cx, cy],
            color: pm,
            uv: WHITE_UV,
        });
        for i in 0..=CIRCLE_SEGMENTS {
            let a = (i as f32) * std::f32::consts::TAU / CIRCLE_SEGMENTS as f32;
            self.vertices.push(Vertex {
                pos: [cx + a.cos() * r, cy + a.sin() * r],
                color: pm,
                uv: WHITE_UV,
            });
        }
        for i in 0..CIRCLE_SEGMENTS {
            self.indices
                .extend_from_slice(&[base, base + 1 + i, base + 2 + i]);
        }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, w: f32, c: Color) {
        if r <= 0.5 || c.a <= 0.0 {
            return;
        }
        let pm = c.premul();
        let inner = (r - w * 0.5).max(0.0);
        let outer = r + w * 0.5;
        let base = self.vertices.len() as u32;
        for i in 0..=CIRCLE_SEGMENTS {
            let a = (i as f32) * std::f32::consts::TAU / CIRCLE_SEGMENTS as f32;
            let cos = a.cos();
            let sin = a.sin();
            self.vertices.push(Vertex {
                pos: [cx + cos * inner, cy + sin * inner],
                color: pm,
                uv: WHITE_UV,
            });
            self.vertices.push(Vertex {
                pos: [cx + cos * outer, cy + sin * outer],
                color: pm,
                uv: WHITE_UV,
            });
        }
        for i in 0..CIRCLE_SEGMENTS {
            let j = i * 2 + base;
            self.indices
                .extend_from_slice(&[j, j + 1, j + 2, j + 1, j + 3, j + 2]);
        }
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        if c.a <= 0.0 {
            return;
        }
        let pm = c.premul();
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&[
            Vertex {
                pos: [x, y],
                color: pm,
                uv: WHITE_UV,
            },
            Vertex {
                pos: [x + w, y],
                color: pm,
                uv: WHITE_UV,
            },
            Vertex {
                pos: [x + w, y + h],
                color: pm,
                uv: WHITE_UV,
            },
            Vertex {
                pos: [x, y + h],
                color: pm,
                uv: WHITE_UV,
            },
        ]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn fill_polygon(&mut self, cx: f32, cy: f32, sides: u8, radius: f32, rot: f32, c: Color) {
        if sides < 3 || c.a <= 0.0 {
            return;
        }
        let pm = c.premul();
        let base = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            pos: [cx, cy],
            color: pm,
            uv: WHITE_UV,
        });
        for i in 0..=sides {
            let a = rot + (i as f32) * std::f32::consts::TAU / sides as f32;
            self.vertices.push(Vertex {
                pos: [cx + a.cos() * radius, cy + a.sin() * radius],
                color: pm,
                uv: WHITE_UV,
            });
        }
        for i in 0..sides as u32 {
            self.indices
                .extend_from_slice(&[base, base + 1 + i, base + 2 + i]);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_polygon(
        &mut self,
        cx: f32,
        cy: f32,
        sides: u8,
        radius: f32,
        rot: f32,
        w: f32,
        c: Color,
    ) {
        if sides < 3 || c.a <= 0.0 {
            return;
        }
        let pm = c.premul();
        let inner = (radius - w * 0.5).max(0.0);
        let outer = radius + w * 0.5;
        let base = self.vertices.len() as u32;
        for i in 0..=sides {
            let a = rot + (i as f32) * std::f32::consts::TAU / sides as f32;
            let cos = a.cos();
            let sin = a.sin();
            self.vertices.push(Vertex {
                pos: [cx + cos * inner, cy + sin * inner],
                color: pm,
                uv: WHITE_UV,
            });
            self.vertices.push(Vertex {
                pos: [cx + cos * outer, cy + sin * outer],
                color: pm,
                uv: WHITE_UV,
            });
        }
        for i in 0..sides as u32 {
            let j = i * 2 + base;
            self.indices
                .extend_from_slice(&[j, j + 1, j + 2, j + 1, j + 3, j + 2]);
        }
    }

    pub fn fill_star(&mut self, cx: f32, cy: f32, size: f32, c: Color) {
        if c.a <= 0.0 {
            return;
        }
        let pm = c.premul();
        let base = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            pos: [cx, cy],
            color: pm,
            uv: WHITE_UV,
        });
        for i in 0..10u32 {
            let a = (i as f32) * std::f32::consts::TAU / 10.0 - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 { size } else { size * 0.4 };
            self.vertices.push(Vertex {
                pos: [cx + a.cos() * r, cy + a.sin() * r],
                color: pm,
                uv: WHITE_UV,
            });
        }
        for i in 0..10u32 {
            self.indices
                .extend_from_slice(&[base, base + 1 + i, base + 1 + (i + 1) % 10]);
        }
    }
}

impl Default for DrawBatch {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FrameInputs<'a> {
    pub effects: &'a [Effect],
    pub particles: &'a [Particle],
    pub exit_progress: f32,
    pub splash_time: f32,
    pub canvas_w: f32,
    pub canvas_h: f32,
}

pub fn build_frame(batch: &mut DrawBatch, texts: &mut Vec<FrameText>, inputs: FrameInputs<'_>) {
    let FrameInputs {
        effects,
        particles,
        exit_progress,
        splash_time,
        canvas_w: sw,
        canvas_h: sh,
    } = inputs;

    for e in effects {
        draw_effect(batch, texts, e, sw, sh);
    }
    for p in particles {
        draw_particle(batch, p);
    }

    if exit_progress > 0.0 {
        let bw = 200.0;
        let bx = sw - bw - 20.0;
        batch.fill_rect(bx, 20.0, bw, 6.0, Color::new(1.0, 1.0, 1.0, 0.2));
        batch.fill_rect(
            bx,
            20.0,
            bw * exit_progress,
            6.0,
            Color::new(1.0, 0.3, 0.3, 0.8),
        );
        texts.push(FrameText {
            text: "hold to exit".into(),
            x: bx,
            y: 30.0,
            font_size: 16.0,
            color: WHITE.with_alpha(0.4 * exit_progress),
        });
    }

    if splash_time > 0.0 {
        let sd = 2.0;
        let alpha = if splash_time > sd - 0.5 {
            (sd - splash_time) / 0.5
        } else if splash_time < 0.5 {
            splash_time / 0.5
        } else {
            1.0
        };
        batch.fill_rect(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.5 * alpha));
        let font_size = sh * 0.12;
        texts.push(FrameText {
            text: "bimbumbam!".into(),
            x: sw * 0.22,
            y: sh / 2.0 - font_size * 0.7,
            font_size,
            color: Color::new(1.0, 0.5, 0.0, alpha),
        });
        texts.push(FrameText {
            text: "bash away!".into(),
            x: sw * 0.38,
            y: sh / 2.0 + font_size * 0.4,
            font_size: font_size * 0.3,
            color: WHITE.with_alpha(alpha * 0.7),
        });
        // Always-visible exit hint during the splash so a parent who hasn't
        // read the README knows how to leave.
        let hint_size = (sh * 0.022).max(14.0);
        texts.push(FrameText {
            text: "hold Ctrl+Alt+Q for 3s to exit".into(),
            x: sw * 0.5 - hint_size * 9.0,
            y: sh - hint_size * 2.5,
            font_size: hint_size,
            color: WHITE.with_alpha(alpha * 0.6),
        });
    }
}

fn draw_particle(batch: &mut DrawBatch, p: &Particle) {
    let a = p.alpha();
    let c = p.color.with_alpha(a);
    let s = p.size * (0.5 + 0.5 * a);
    match p.shape {
        ParticleShape::Circle => batch.fill_circle(p.x, p.y, s, c),
        ParticleShape::Star => batch.fill_star(p.x, p.y, s, c),
        ParticleShape::Square => batch.fill_rect(p.x - s, p.y - s, s * 2.0, s * 2.0, c),
    }
}

fn draw_effect(batch: &mut DrawBatch, texts: &mut Vec<FrameText>, e: &Effect, sw: f32, sh: f32) {
    let alpha = e.alpha();
    let progress = e.progress();
    match &e.kind {
        EffectKind::BigChar { ch, color } => {
            let bounce = if progress < 0.2 {
                progress / 0.2 * 1.3
            } else if progress < 0.35 {
                1.3 - 0.3 * ((progress - 0.2) / 0.15)
            } else {
                1.0
            };
            let font_size = sh * 0.25 * bounce;
            // Glyphon would give us exact metrics, but the approximation reads
            // well enough for a single glyph and avoids a layout query in the
            // hot path. Centering shifts a hair on different fonts; acceptable.
            let approx_w = font_size * 0.65;
            texts.push(FrameText {
                text: ch.to_string(),
                x: e.x - approx_w / 2.0 + 4.0,
                y: e.y - font_size / 2.0 + 4.0,
                font_size,
                color: Color::new(0.0, 0.0, 0.0, alpha * 0.3),
            });
            texts.push(FrameText {
                text: ch.to_string(),
                x: e.x - approx_w / 2.0,
                y: e.y - font_size / 2.0,
                font_size,
                color: color.with_alpha(alpha),
            });
        }
        EffectKind::Shockwave { color, max_radius } => {
            batch.stroke_circle(
                e.x,
                e.y,
                progress * max_radius,
                4.0 + 8.0 * alpha,
                color.with_alpha(alpha * 0.7),
            );
        }
        EffectKind::FlyingShape {
            color,
            sides,
            size,
            rotation,
            ..
        } => {
            batch.fill_polygon(e.x, e.y, *sides, *size, *rotation, color.with_alpha(alpha));
            batch.stroke_polygon(
                e.x,
                e.y,
                *sides,
                *size,
                *rotation,
                3.0,
                WHITE.with_alpha(alpha * 0.5),
            );
        }
        EffectKind::SoftFlash { color } => {
            batch.fill_rect(0.0, 0.0, sw, sh, color.with_alpha(alpha * FLASH_PEAK_ALPHA));
        }
        EffectKind::Spiral {
            color,
            rotation,
            arm_count,
        } => {
            let arms = *arm_count as usize;
            for arm in 0..arms {
                let ba = *rotation + (arm as f32) * std::f32::consts::TAU / arms as f32;
                for i in 0..15 {
                    let t = i as f32 / 15.0;
                    let a = ba + t * 3.0;
                    let r = t * 150.0;
                    batch.fill_circle(
                        e.x + a.cos() * r,
                        e.y + a.sin() * r,
                        6.0 * (1.0 - t * 0.3),
                        color.with_alpha(alpha * (1.0 - t * 0.5)),
                    );
                }
            }
        }
        EffectKind::BouncingBall { color, size, .. } => {
            batch.fill_circle(e.x, e.y, *size, color.with_alpha(alpha));
            // Specular highlight to suggest a glossy ball.
            batch.fill_circle(
                e.x - *size * 0.3,
                e.y - *size * 0.3,
                *size * 0.3,
                WHITE.with_alpha(alpha * 0.4),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    #[test]
    fn empty_batch_emits_nothing() {
        let b = DrawBatch::new();
        assert!(b.vertices.is_empty());
        assert!(b.indices.is_empty());
    }

    #[test]
    fn rect_emits_six_indices_pointing_to_vertices() {
        let mut b = DrawBatch::new();
        b.fill_rect(0.0, 0.0, 10.0, 10.0, Color::new(1.0, 1.0, 1.0, 1.0));
        assert_eq!(b.vertices.len(), 4);
        assert_eq!(b.indices.len(), 6);
        for &i in &b.indices {
            assert!((i as usize) < b.vertices.len());
        }
    }

    #[test]
    fn circle_indices_in_range() {
        let mut b = DrawBatch::new();
        b.fill_circle(50.0, 50.0, 25.0, Color::new(1.0, 0.0, 0.0, 1.0));
        for &i in &b.indices {
            assert!((i as usize) < b.vertices.len());
        }
    }

    #[test]
    fn star_indices_in_range() {
        let mut b = DrawBatch::new();
        b.fill_star(50.0, 50.0, 20.0, Color::new(1.0, 1.0, 0.0, 1.0));
        for &i in &b.indices {
            assert!((i as usize) < b.vertices.len());
        }
    }

    #[test]
    fn zero_alpha_does_not_emit() {
        let mut b = DrawBatch::new();
        b.fill_rect(0.0, 0.0, 10.0, 10.0, Color::new(1.0, 1.0, 1.0, 0.0));
        b.fill_circle(0.0, 0.0, 10.0, Color::new(1.0, 1.0, 1.0, 0.0));
        b.fill_star(0.0, 0.0, 10.0, Color::new(1.0, 1.0, 1.0, 0.0));
        assert!(b.vertices.is_empty());
        assert!(b.indices.is_empty());
    }
}
