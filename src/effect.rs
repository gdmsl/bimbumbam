//! High-level visual effects (the things spawned by a key press) and the
//! spawn routines that translate a [`KeyAction`] into [`Effect`]s and [`Particle`]s.

use crate::color::{Color, PALETTE, random_bright_color, random_color};
use crate::particle::Particle;
use rand::Rng;

const PARTICLE_COUNT_LETTER: usize = 12;
const PARTICLE_COUNT_FIREWORK: usize = 30;
const PARTICLE_COUNT_WAVE: usize = 20;
pub const TEXT_LIFETIME: f32 = 2.5;
pub const PARTICLE_LIFETIME: f32 = 1.8;

#[derive(Debug)]
pub enum EffectKind {
    BigChar {
        ch: char,
        color: Color,
    },
    Shockwave {
        color: Color,
        max_radius: f32,
    },
    FlyingShape {
        dx: f32,
        dy: f32,
        color: Color,
        sides: u8,
        size: f32,
        rotation: f32,
    },
    /// A soft tinted overlay painted across the entire frame. Capped intensity
    /// keeps repeated triggers (e.g. a held space bar) from becoming a strobe.
    SoftFlash {
        color: Color,
    },
    Spiral {
        color: Color,
        rotation: f32,
        arm_count: u8,
    },
    BouncingBall {
        vx: f32,
        vy: f32,
        color: Color,
        size: f32,
    },
}

#[derive(Debug)]
pub struct Effect {
    pub x: f32,
    pub y: f32,
    pub lifetime: f32,
    pub max_lifetime: f32,
    pub kind: EffectKind,
}

impl Effect {
    pub fn alpha(&self) -> f32 {
        (self.lifetime / self.max_lifetime).clamp(0.0, 1.0)
    }
    pub fn progress(&self) -> f32 {
        1.0 - self.alpha()
    }
    pub fn is_alive(&self) -> bool {
        self.lifetime > 0.0
    }

    pub fn update(&mut self, dt: f32, sw: f32, sh: f32) {
        self.lifetime -= dt;
        match &mut self.kind {
            EffectKind::FlyingShape {
                dx, dy, rotation, ..
            } => {
                self.x += *dx * dt;
                self.y += *dy * dt;
                *rotation += 3.0 * dt;
                if self.x < 0.0 || self.x > sw {
                    *dx = -*dx;
                    self.x = self.x.clamp(0.0, sw);
                }
                if self.y < 0.0 || self.y > sh {
                    *dy = -*dy;
                    self.y = self.y.clamp(0.0, sh);
                }
            }
            EffectKind::Spiral { rotation, .. } => {
                *rotation += 2.0 * dt;
            }
            EffectKind::BouncingBall { vx, vy, size, .. } => {
                self.x += *vx * dt;
                self.y += *vy * dt;
                *vy += 200.0 * dt;
                let s = *size;
                if self.x - s < 0.0 {
                    *vx = vx.abs();
                    self.x = s;
                } else if self.x + s > sw {
                    *vx = -vx.abs();
                    self.x = sw - s;
                }
                if self.y + s > sh {
                    *vy = -vy.abs() * 0.8;
                    self.y = sh - s;
                } else if self.y - s < 0.0 {
                    *vy = vy.abs();
                    self.y = s;
                }
            }
            // Stationary effects whose only animation is alpha decay — listed
            // explicitly so adding a new variant gets the exhaustiveness check.
            EffectKind::BigChar { .. }
            | EffectKind::Shockwave { .. }
            | EffectKind::SoftFlash { .. } => {}
        }
    }
}

pub fn spawn_letter<R: Rng + ?Sized>(
    rng: &mut R,
    ch: char,
    sw: f32,
    sh: f32,
    e: &mut Vec<Effect>,
    p: &mut Vec<Particle>,
) {
    let x = rng.random_range(sw * 0.15..sw * 0.85);
    let y = rng.random_range(sh * 0.2..sh * 0.8);
    let color = random_color(rng);
    e.push(Effect {
        x,
        y,
        lifetime: TEXT_LIFETIME,
        max_lifetime: TEXT_LIFETIME,
        kind: EffectKind::BigChar { ch, color },
    });
    for _ in 0..PARTICLE_COUNT_LETTER {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let s = rng.random_range(100.0..350.0);
        let life = rng.random_range(0.8..PARTICLE_LIFETIME);
        let size = rng.random_range(3.0..8.0);
        let bright = random_bright_color(rng);
        p.push(Particle::new(
            rng,
            x,
            y,
            a.cos() * s,
            a.sin() * s - 50.0,
            bright,
            life,
            size,
        ));
    }
}

pub fn spawn_firework<R: Rng + ?Sized>(
    rng: &mut R,
    sw: f32,
    sh: f32,
    flash: bool,
    e: &mut Vec<Effect>,
    p: &mut Vec<Particle>,
) {
    let x = rng.random_range(sw * 0.1..sw * 0.9);
    let y = rng.random_range(sh * 0.1..sh * 0.6);
    let color = random_color(rng);
    if flash {
        e.push(Effect {
            x: 0.0,
            y: 0.0,
            lifetime: 0.5,
            max_lifetime: 0.5,
            kind: EffectKind::SoftFlash { color },
        });
    }
    let max_radius = rng.random_range(200.0..400.0);
    e.push(Effect {
        x,
        y,
        lifetime: 1.2,
        max_lifetime: 1.2,
        kind: EffectKind::Shockwave { color, max_radius },
    });
    for _ in 0..PARTICLE_COUNT_FIREWORK {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let s = rng.random_range(150.0..500.0);
        let life = rng.random_range(1.0..2.5);
        let size = rng.random_range(3.0..10.0);
        let bright = random_bright_color(rng);
        p.push(Particle::new(
            rng,
            x,
            y,
            a.cos() * s,
            a.sin() * s,
            bright,
            life,
            size,
        ));
    }
}

pub fn spawn_rainbow<R: Rng + ?Sized>(
    rng: &mut R,
    sw: f32,
    sh: f32,
    flash: bool,
    e: &mut Vec<Effect>,
    p: &mut Vec<Particle>,
) {
    let cx = sw / 2.0;
    let cy = sh / 2.0;
    for (i, c) in PALETTE.iter().take(6).enumerate() {
        let life = 2.0 + i as f32 * 0.15;
        e.push(Effect {
            x: cx,
            y: cy,
            lifetime: life,
            max_lifetime: life,
            kind: EffectKind::Shockwave {
                color: *c,
                max_radius: 600.0 + i as f32 * 50.0,
            },
        });
    }
    if flash {
        e.push(Effect {
            x: 0.0,
            y: 0.0,
            lifetime: 0.8,
            max_lifetime: 0.8,
            kind: EffectKind::SoftFlash {
                color: Color::new(1.0, 1.0, 1.0, 1.0),
            },
        });
    }
    for _ in 0..PARTICLE_COUNT_WAVE {
        let a = rng.random_range(0.0..std::f32::consts::TAU);
        let s = rng.random_range(200.0..600.0);
        let life = rng.random_range(1.0..2.0);
        let size = rng.random_range(4.0..12.0);
        let color = random_color(rng);
        p.push(Particle::new(
            rng,
            cx,
            cy,
            a.cos() * s,
            a.sin() * s,
            color,
            life,
            size,
        ));
    }
}

pub fn spawn_arrow<R: Rng + ?Sized>(
    rng: &mut R,
    dx: f32,
    dy: f32,
    sw: f32,
    sh: f32,
    e: &mut Vec<Effect>,
    p: &mut Vec<Particle>,
) {
    let x = sw / 2.0 + rng.random_range(-100.0..100.0);
    let y = sh / 2.0 + rng.random_range(-100.0..100.0);
    let color = random_color(rng);
    let sides = rng.random_range(3..8);
    let size = rng.random_range(30.0..60.0);
    e.push(Effect {
        x,
        y,
        lifetime: 3.0,
        max_lifetime: 3.0,
        kind: EffectKind::FlyingShape {
            dx: dx * 300.0,
            dy: dy * 300.0,
            color,
            sides,
            size,
            rotation: 0.0,
        },
    });
    for _ in 0..15 {
        let pvx = -dx * rng.random_range(50.0..150.0) + rng.random_range(-30.0..30.0);
        let pvy = -dy * rng.random_range(50.0..150.0) + rng.random_range(-30.0..30.0);
        let life = rng.random_range(0.5..1.2);
        let psize = rng.random_range(3.0..7.0);
        p.push(Particle::new(rng, x, y, pvx, pvy, color, life, psize));
    }
}

pub fn spawn_misc<R: Rng + ?Sized>(
    rng: &mut R,
    sw: f32,
    sh: f32,
    e: &mut Vec<Effect>,
    p: &mut Vec<Particle>,
) {
    let x = rng.random_range(sw * 0.1..sw * 0.9);
    let y = rng.random_range(sh * 0.1..sh * 0.9);
    match rng.random_range(0..4) {
        0 => {
            let arm_count = rng.random_range(3..7);
            e.push(Effect {
                x,
                y,
                lifetime: 2.5,
                max_lifetime: 2.5,
                kind: EffectKind::Spiral {
                    color: random_color(rng),
                    rotation: 0.0,
                    arm_count,
                },
            });
        }
        1 => {
            let vx = rng.random_range(-300.0..300.0);
            let vy = rng.random_range(-400.0..-100.0);
            let size = rng.random_range(20.0..50.0);
            e.push(Effect {
                x,
                y,
                lifetime: 4.0,
                max_lifetime: 4.0,
                kind: EffectKind::BouncingBall {
                    vx,
                    vy,
                    color: random_color(rng),
                    size,
                },
            });
        }
        2 => {
            let c = random_color(rng);
            for _ in 0..25 {
                let a = rng.random_range(0.0..std::f32::consts::TAU);
                let s = rng.random_range(30.0..120.0);
                let life = rng.random_range(1.5..3.0);
                let size = rng.random_range(8.0..20.0);
                p.push(Particle::new(
                    rng,
                    x,
                    y,
                    a.cos() * s,
                    a.sin() * s - 30.0,
                    c,
                    life,
                    size,
                ));
            }
            e.push(Effect {
                x,
                y,
                lifetime: 1.5,
                max_lifetime: 1.5,
                kind: EffectKind::Shockwave {
                    color: c,
                    max_radius: 150.0,
                },
            });
        }
        _ => {
            for _ in 0..5 {
                let a = rng.random_range(0.0..std::f32::consts::TAU);
                let s = rng.random_range(100.0..300.0);
                let sides = rng.random_range(3..8);
                let size = rng.random_range(15.0..40.0);
                let rot = rng.random_range(0.0..std::f32::consts::TAU);
                e.push(Effect {
                    x,
                    y,
                    lifetime: 2.5,
                    max_lifetime: 2.5,
                    kind: EffectKind::FlyingShape {
                        dx: a.cos() * s,
                        dy: a.sin() * s,
                        color: random_color(rng),
                        sides,
                        size,
                        rotation: rot,
                    },
                });
            }
            for _ in 0..20 {
                let a = rng.random_range(0.0..std::f32::consts::TAU);
                let s = rng.random_range(80.0..250.0);
                let life = rng.random_range(0.8..1.5);
                let size = rng.random_range(3.0..8.0);
                let bright = random_bright_color(rng);
                p.push(Particle::new(
                    rng,
                    x,
                    y,
                    a.cos() * s,
                    a.sin() * s,
                    bright,
                    life,
                    size,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_effect() -> Effect {
        Effect {
            x: 0.0,
            y: 0.0,
            lifetime: 1.0,
            max_lifetime: 1.0,
            kind: EffectKind::Shockwave {
                color: Color::new(1.0, 0.0, 0.0, 1.0),
                max_radius: 100.0,
            },
        }
    }

    #[test]
    fn alpha_and_progress_are_complementary() {
        let mut e = dummy_effect();
        e.update(0.25, 1920.0, 1080.0);
        let total = e.alpha() + e.progress();
        assert!((total - 1.0).abs() < 1e-5);
    }

    #[test]
    fn bouncing_ball_clamps_inside_screen() {
        let mut e = Effect {
            x: -50.0,
            y: 600.0,
            lifetime: 1.0,
            max_lifetime: 1.0,
            kind: EffectKind::BouncingBall {
                vx: -500.0,
                vy: 0.0,
                color: Color::new(1.0, 1.0, 1.0, 1.0),
                size: 20.0,
            },
        };
        e.update(0.016, 1920.0, 1080.0);
        // Should clamp to size and flip vx positive
        assert!(e.x >= 0.0);
        if let EffectKind::BouncingBall { vx, .. } = e.kind {
            assert!(vx > 0.0);
        }
    }
}
