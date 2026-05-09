//! Lightweight particle system. Particles are short-lived sprites driven by
//! integration of a velocity field with a constant gravity term.

use crate::color::Color;
use rand::Rng;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParticleShape {
    Circle,
    Star,
    Square,
}

#[derive(Debug)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub color: Color,
    pub lifetime: f32,
    pub max_lifetime: f32,
    pub size: f32,
    pub shape: ParticleShape,
}

/// Constant downward acceleration applied to every particle.
const GRAVITY: f32 = 80.0;
/// Velocity damping per second (multiplicative); chosen to avoid particles drifting forever.
const HORIZONTAL_DAMPING: f32 = 0.99;

impl Particle {
    #[allow(clippy::too_many_arguments)]
    pub fn new<R: Rng + ?Sized>(
        rng: &mut R,
        x: f32,
        y: f32,
        vx: f32,
        vy: f32,
        color: Color,
        lifetime: f32,
        size: f32,
    ) -> Self {
        let shape = match rng.random_range(0..3) {
            0 => ParticleShape::Circle,
            1 => ParticleShape::Star,
            _ => ParticleShape::Square,
        };
        Self {
            x,
            y,
            vx,
            vy,
            color,
            lifetime,
            max_lifetime: lifetime,
            size,
            shape,
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.x += self.vx * dt;
        self.y += self.vy * dt;
        self.vy += GRAVITY * dt;
        self.vx *= HORIZONTAL_DAMPING;
        self.lifetime -= dt;
    }

    pub fn alpha(&self) -> f32 {
        (self.lifetime / self.max_lifetime).clamp(0.0, 1.0)
    }

    pub fn is_alive(&self) -> bool {
        self.lifetime > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Particle {
        Particle {
            x: 0.0,
            y: 0.0,
            vx: 100.0,
            vy: 0.0,
            color: Color::new(1.0, 1.0, 1.0, 1.0),
            lifetime: 1.0,
            max_lifetime: 1.0,
            size: 4.0,
            shape: ParticleShape::Circle,
        }
    }

    #[test]
    fn alpha_starts_full_decays_to_zero() {
        let mut x = p();
        assert!((x.alpha() - 1.0).abs() < 1e-6);
        x.update(0.5);
        assert!((x.alpha() - 0.5).abs() < 1e-5);
        x.update(0.5);
        assert!(x.alpha() <= 0.0);
    }

    #[test]
    fn gravity_pulls_down_and_horizontal_damping_applies() {
        let mut x = p();
        x.update(1.0);
        assert!((x.vy - GRAVITY).abs() < 1e-4);
        assert!((x.vx - 100.0 * HORIZONTAL_DAMPING).abs() < 1e-3);
    }

    #[test]
    fn dies_when_lifetime_exhausted() {
        let mut x = p();
        x.update(2.0);
        assert!(!x.is_alive());
    }
}
