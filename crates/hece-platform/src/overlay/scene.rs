//! The overlay's state over time: which phase shows, the exit animation, the eased colour
//! weights and the syllable waves. Drawing reads this state; nothing here touches a window.

use std::time::Duration;

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
