//! The overlay's state over time: which phase shows, the exit animation, the eased colour
//! weights and the syllable waves. Drawing reads this state; nothing here touches a window.

use std::time::Duration;

use tiny_skia::{
    Color, FillRule, GradientStop, Paint, PathBuilder, Pixmap, Point, RadialGradient, SpreadMode,
    Stroke, Transform,
};

use super::sphere::Sphere;
use super::{Meter, Outcome, Phase, Voice};

/// Length of every exit animation.
pub const EXIT: Duration = Duration::from_millis(300);
/// How long a syllable wave travels over the sphere.
const WAVE_LIFE: Duration = Duration::from_millis(1400);
/// Easing of the colour weights per frame.
const EASE: f32 = 0.07;

/// How much of each look is showing; eased toward the phase, so colours never jump.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Weights {
    ready: f32,
    speak: f32,
    silent: f32,
    work: f32,
}

impl Weights {
    const READY: Self = Self {
        ready: 1.0,
        speak: 0.0,
        silent: 0.0,
        work: 0.0,
    };

    fn ease_to(&mut self, target: Self) {
        self.ready += (target.ready - self.ready) * EASE;
        self.speak += (target.speak - self.speak) * EASE;
        self.silent += (target.silent - self.silent) * EASE;
        self.work += (target.work - self.work) * EASE;
    }
}

/// The overlay's animation state.
#[derive(Debug)]
pub struct Scene {
    sphere: Sphere,
    reduced_motion: bool,
    phase: Phase,
    visible: bool,
    /// When the overlay last appeared (the entry animation runs from here).
    shown_at: Duration,
    /// The running exit animation: how it ends and when it started.
    exit: Option<(Outcome, Duration)>,
    meter: Meter,
    voice: Voice,
    weights: Weights,
    /// Start times of the syllable waves still travelling.
    waves: Vec<Duration>,
    now: Duration,
}

impl Scene {
    /// A hidden scene. With `reduced_motion` the sphere neither rotates nor deforms.
    #[must_use]
    pub fn new(reduced_motion: bool) -> Self {
        Self {
            sphere: Sphere::new(),
            reduced_motion,
            phase: Phase::Idle,
            visible: false,
            shown_at: Duration::ZERO,
            exit: None,
            meter: Meter::default(),
            voice: Voice::default(),
            weights: Weights::READY,
            waves: Vec::new(),
            now: Duration::ZERO,
        }
    }

    /// Switches to `phase` at `now` (time since the overlay started). Setting the current
    /// phase again changes nothing.
    pub fn set_phase(&mut self, phase: Phase, now: Duration) {
        match phase {
            Phase::Opening | Phase::Listening | Phase::Transcribing => {
                if !self.visible {
                    self.visible = true;
                    self.shown_at = now;
                    self.weights = Weights::READY;
                    self.waves.clear();
                }
                self.exit = None;
            }
            Phase::Finished(outcome) => {
                if self.visible && self.exit.is_none() {
                    self.exit = Some((outcome, now));
                }
            }
            Phase::Idle => {
                if self.visible && self.exit.is_none() {
                    self.exit = Some((Outcome::NothingHeard, now));
                }
            }
        }
        self.phase = phase;
    }

    /// Advances to `now` with the latest capture RMS; returns whether the overlay shows.
    pub fn tick(&mut self, now: Duration, rms: f32) -> bool {
        self.now = now;
        if let Some((_, started)) = self.exit
            && now.saturating_sub(started) >= EXIT
        {
            self.visible = false;
            self.exit = None;
        }
        if !self.visible {
            return false;
        }
        self.voice = self.meter.update(rms);
        let listening = self.phase == Phase::Listening;
        if listening && self.voice.onset {
            self.waves.push(now);
        }
        self.waves.retain(|w| now.saturating_sub(*w) < WAVE_LIFE);
        let target = match self.phase {
            Phase::Opening => Weights::READY,
            Phase::Listening if self.voice.speaking => Weights {
                speak: 1.0,
                ..Weights::default()
            },
            Phase::Listening => Weights {
                silent: 1.0,
                ..Weights::default()
            },
            Phase::Transcribing => Weights {
                work: 1.0,
                ..Weights::default()
            },
            // Exiting: keep the look it had.
            Phase::Idle | Phase::Finished(_) => self.weights,
        };
        self.weights.ease_to(target);
        true
    }

    /// Whether the overlay shows.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.visible
    }
}

/// The prototype's canvas size; drawing is scaled from it to the pixmap.
const SIZE: f32 = 140.0;
/// Entry animation length, in seconds.
const ENTRY: f32 = 0.15;

type Rgb = [f32; 3];

const VIOLET: Rgb = rgb(0x96, 0x82, 0xff);
const LIGHT_BLUE: Rgb = rgb(0x5a, 0xb4, 0xe6);
const SILVER: Rgb = rgb(0xa8, 0xac, 0xb6);
const DANGER: Rgb = rgb(0xb3, 0x26, 0x1e);
const AMBER: Rgb = rgb(0xf5, 0xa5, 0x24);

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

fn mix(a: Rgb, b: Rgb, k: f32) -> Rgb {
    [
        a[0] + (b[0] - a[0]) * k,
        a[1] + (b[1] - a[1]) * k,
        a[2] + (b[2] - a[2]) * k,
    ]
}

/// HSL to RGB; `hue` in degrees (any value), `s` and `l` in 0..1.
fn hsl(hue: f32, saturation: f32, lightness: f32) -> Rgb {
    let sector = hue.rem_euclid(360.0) / 30.0;
    let chroma = saturation * lightness.min(1.0 - lightness);
    let channel = |offset: f32| {
        let k = (offset + sector) % 12.0;
        lightness - chroma * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [channel(0.0), channel(8.0), channel(4.0)]
}

fn color(c: Rgb, alpha: f32) -> Color {
    Color::from_rgba(
        c[0].clamp(0.0, 1.0),
        c[1].clamp(0.0, 1.0),
        c[2].clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    )
    .unwrap_or(Color::TRANSPARENT)
}

fn solid(c: Rgb, alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color(c, alpha));
    paint.anti_alias = true;
    paint
}

/// A point of the sphere after deformation and rotation.
struct Projected {
    x: f32,
    y: f32,
    z: f32,
    /// How strongly syllable waves lift this point, 0..~1.
    wave: f32,
}

impl Scene {
    /// Draws the current frame into `pixmap` (square, any size), clearing it first. Draws
    /// nothing while hidden.
    pub fn draw(&self, pixmap: &mut Pixmap) {
        pixmap.fill(Color::TRANSPARENT);
        if !self.visible {
            return;
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "the overlay is a few hundred pixels wide"
        )]
        let scale = pixmap.width() as f32 / SIZE;
        let transform = Transform::from_scale(scale, scale);
        let wt = self.weights;
        let level = self.voice.level;
        let secs = self.now.as_secs_f32();
        // Motion time: frozen when Windows animations are turned off.
        let tm = if self.reduced_motion { 0.0 } else { secs };
        let entry = (self.now.saturating_sub(self.shown_at).as_secs_f32() / ENTRY).min(1.0);
        let (exit_k, outcome) = self.exit.map_or((0.0, None), |(outcome, started)| {
            let k = self.now.saturating_sub(started).as_secs_f32() / EXIT.as_secs_f32();
            (k.min(1.0), Some(outcome))
        });
        let fade = 1.0 - exit_k;
        // Inserted collapses into the core; the other endings shrink a little and fade.
        let shrink = if outcome == Some(Outcome::Inserted) {
            1.0 - exit_k
        } else {
            1.0 - 0.3 * exit_k
        };
        let size = (0.6 + 0.4 * entry) * (1.0 - 0.15 * wt.work) * shrink;
        let tint = match outcome {
            Some(Outcome::Failed) => Some(DANGER),
            Some(Outcome::NotPasted) => Some(AMBER),
            Some(Outcome::NothingHeard) => Some(SILVER),
            Some(Outcome::Inserted) | None => None,
        };
        let points = self.project(tm, size);

        let calm = mix(
            mix(VIOLET, LIGHT_BLUE, 0.35),
            SILVER,
            wt.silent / (wt.silent + wt.ready + 1e-6),
        );
        let base = mix(calm, VIOLET, wt.speak);
        let hue_speed = 60.0 * (1.0 - wt.work) + 30.0 * wt.work;
        let stroke = Stroke {
            width: 0.75,
            ..Stroke::default()
        };
        for &(a, b) in &self.sphere.edges {
            let (p, q) = (&points[a], &points[b]);
            let (va, vb) = (self.sphere.vertices[a], self.sphere.vertices[b]);
            let wave = p.wave.max(q.wave);
            let hue = (va[1] + vb[1]).atan2(va[0] + vb[0]).to_degrees() + hue_speed * secs;
            let rainbow = (wave * 1.4 + wt.speak * level * 0.25 + wt.work * 0.5).min(1.0);
            let mut shade = mix(base, hsl(hue, 0.95, 0.62), rainbow);
            if let Some(tint) = tint {
                shade = mix(shade, tint, exit_k);
            }
            let depth = f32::midpoint(f32::midpoint(p.z, q.z) / 40.0, 1.0);
            let alpha = (0.18 + 0.5 * depth + 0.35 * wave).min(0.95) * fade;
            let mut path = PathBuilder::new();
            path.move_to(p.x, p.y);
            path.line_to(q.x, q.y);
            if let Some(path) = path.finish() {
                pixmap.stroke_path(&path, &solid(shade, alpha), &stroke, transform, None);
            }
        }
        for (p, v) in points.iter().zip(&self.sphere.vertices) {
            if p.z < 0.0 || p.wave < 0.15 {
                continue;
            }
            let shade = hsl(v[1].atan2(v[0]).to_degrees() + hue_speed * secs, 1.0, 0.7);
            if let Some(dot) = PathBuilder::from_circle(p.x, p.y, 1.0 + 1.5 * p.wave) {
                let paint = solid(shade, p.wave.min(1.0) * fade);
                pixmap.fill_path(&dot, &paint, FillRule::Winding, transform, None);
            }
        }
        self.draw_core(pixmap, transform, secs, entry, exit_k, tint, base);
    }

    /// The syllable dot in the middle and its glow.
    #[allow(
        clippy::too_many_arguments,
        reason = "frame values computed once in draw"
    )]
    fn draw_core(
        &self,
        pixmap: &mut Pixmap,
        transform: Transform,
        secs: f32,
        entry: f32,
        exit_k: f32,
        tint: Option<Rgb>,
        base: Rgb,
    ) {
        let wt = self.weights;
        let pulse = if self.reduced_motion { 0.0 } else { 1.0 };
        let radius = (3.5
            + wt.speak * self.voice.level * 5.0
            + pulse * (wt.ready * (2.0 * secs).sin() * 0.6 + wt.work * (3.0 * secs).sin() * 0.8))
            .max(1.0)
            * (0.6 + 0.4 * entry);
        let mut core = if wt.speak > 0.5 || wt.work > 0.5 {
            hsl(60.0 * secs, 0.9, 0.7)
        } else {
            base
        };
        if let Some(tint) = tint {
            core = mix(core, tint, (exit_k * 3.0).min(1.0));
        }
        let fade = 1.0 - exit_k;
        let center = Point::from_xy(SIZE / 2.0, SIZE / 2.0);
        let glow = RadialGradient::new(
            center,
            0.0,
            center,
            radius * 4.0,
            vec![
                GradientStop::new(0.0, color(core, 0.4 * fade)),
                GradientStop::new(1.0, color(core, 0.0)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        );
        if let (Some(shader), Some(disc)) = (
            glow,
            PathBuilder::from_circle(center.x, center.y, radius * 4.0),
        ) {
            let paint = Paint {
                shader,
                anti_alias: true,
                ..Paint::default()
            };
            pixmap.fill_path(&disc, &paint, FillRule::Winding, transform, None);
        }
        if let Some(disc) = PathBuilder::from_circle(center.x, center.y, radius) {
            let paint = solid(core, fade);
            pixmap.fill_path(&disc, &paint, FillRule::Winding, transform, None);
        }
    }

    /// Deforms, rotates and projects the sphere at motion time `tm`, scaled by `size`.
    fn project(&self, tm: f32, size: f32) -> Vec<Projected> {
        let wt = self.weights;
        let base = 18.0 + (1.0 - wt.ready) * 12.0;
        let (amp, breathe) = if self.reduced_motion {
            (0.0, 0.0)
        } else {
            (
                wt.ready * 1.2
                    + wt.silent * 2.0
                    + wt.speak * (4.0 + 14.0 * self.voice.level)
                    + wt.work * 1.5,
                wt.ready * (2.0 * tm).sin() * 1.3,
            )
        };
        let life = WAVE_LIFE.as_secs_f32();
        let ages: Vec<f32> = self
            .waves
            .iter()
            .map(|start| self.now.saturating_sub(*start).as_secs_f32())
            .collect();
        let (sin_y, cos_y) = (0.45 * tm).sin_cos();
        let (sin_x, cos_x) = 0.4_f32.sin_cos();
        self.sphere
            .vertices
            .iter()
            .map(|&[x, y, z]| {
                let lobes = (x * 2.2 + tm * 1.6).sin() * (y * 2.4 - tm * 1.2).cos()
                    + 0.6 * (z * 3.1 + tm * 2.3).sin();
                // Waves start at the pole facing the viewer and travel across the surface.
                let d = z.clamp(-1.0, 1.0).acos();
                let wave: f32 = ages
                    .iter()
                    .map(|age| (-(d - age * 1.6).powi(2) * 10.0).exp() * (1.0 - age / life))
                    .sum();
                let r = (base + amp * lobes * 0.6 + wave * 7.0 + breathe) * size;
                let (px, py, pz) = (x * r, y * r, z * r);
                let x1 = px * cos_y + pz * sin_y;
                let z1 = -px * sin_y + pz * cos_y;
                let y1 = py * cos_x - z1 * sin_x;
                let z2 = py * sin_x + z1 * cos_x;
                Projected {
                    x: SIZE / 2.0 + x1,
                    y: SIZE / 2.0 + y1,
                    z: z2,
                    wave,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn starts_hidden() {
        assert!(!Scene::new(false).tick(ms(0), 0.0));
    }

    #[test]
    fn finished_hides_after_exit() {
        let mut scene = Scene::new(false);
        scene.set_phase(Phase::Listening, ms(0));
        scene.set_phase(Phase::Finished(Outcome::Inserted), ms(1000));
        assert!(scene.tick(ms(1299), 0.0));
        assert!(!scene.tick(ms(1301), 0.0));
    }

    #[test]
    fn new_phase_cancels_exit() {
        let mut scene = Scene::new(false);
        scene.set_phase(Phase::Listening, ms(0));
        scene.set_phase(Phase::Finished(Outcome::Failed), ms(1000));
        scene.set_phase(Phase::Opening, ms(1100));
        assert!(scene.tick(ms(1500), 0.0));
    }

    #[test]
    fn idle_while_visible_fades_out() {
        let mut scene = Scene::new(false);
        scene.set_phase(Phase::Listening, ms(0));
        scene.set_phase(Phase::Idle, ms(500));
        assert!(scene.tick(ms(700), 0.0));
        assert!(!scene.tick(ms(801), 0.0));
    }

    #[test]
    fn repeated_finished_does_not_restart_exit() {
        let mut scene = Scene::new(false);
        scene.set_phase(Phase::Listening, ms(0));
        scene.set_phase(Phase::Finished(Outcome::Inserted), ms(1000));
        scene.set_phase(Phase::Finished(Outcome::Inserted), ms(1200));
        assert!(!scene.tick(ms(1301), 0.0));
    }

    fn drawn(scene: &Scene) -> bool {
        let mut pixmap = tiny_skia::Pixmap::new(140, 140).unwrap();
        scene.draw(&mut pixmap);
        pixmap.pixels().iter().any(|p| p.alpha() > 0)
    }

    #[test]
    fn listening_draws_something() {
        let mut scene = Scene::new(false);
        scene.set_phase(Phase::Listening, ms(0));
        for frame in 0..10 {
            scene.tick(ms(frame * 16), 0.1);
        }
        assert!(drawn(&scene));
    }

    #[test]
    fn hidden_draws_nothing() {
        assert!(!drawn(&Scene::new(false)));
    }

    #[test]
    fn phase_survives_raw_round_trip() {
        for phase in [
            Phase::Idle,
            Phase::Opening,
            Phase::Listening,
            Phase::Transcribing,
            Phase::Finished(Outcome::Inserted),
            Phase::Finished(Outcome::NothingHeard),
            Phase::Finished(Outcome::NotPasted),
            Phase::Finished(Outcome::Failed),
        ] {
            assert_eq!(Phase::from_raw(phase.to_raw()), phase);
        }
    }
}
