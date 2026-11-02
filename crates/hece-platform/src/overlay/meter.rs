//! Turns the capture RMS into the overlay's level, speaking flag and syllable onsets.

/// Quietest level shown, in dBFS.
const FLOOR_DB: f32 = -50.0;
/// Loudest level shown, in dBFS.
const CEIL_DB: f32 = -15.0;
/// Smoothing per frame when the level rises.
const ATTACK: f32 = 0.35;
/// Smoothing per frame when the level falls.
const RELEASE: f32 = 0.08;
/// Above this smoothed level the user counts as speaking.
const SPEAKING: f32 = 0.12;
/// A rise past this level is a syllable onset...
const ONSET: f32 = 0.35;
/// ...once the level has dipped below this since the last one.
const REARM: f32 = 0.2;

/// What the meter hears in one frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Voice {
    /// Smoothed level, 0..1.
    pub level: f32,
    /// Whether the user is speaking.
    pub speaking: bool,
    /// Whether a syllable started in this frame.
    pub onset: bool,
}

/// Smooths the RMS and detects syllable onsets; fed once per frame.
#[derive(Debug, Default)]
pub struct Meter {
    smooth: f32,
    previous: f32,
    /// An onset fired and the level has not dipped since.
    spent: bool,
}

impl Meter {
    /// Feeds the RMS (linear) of the latest audio block.
    pub fn update(&mut self, rms: f32) -> Voice {
        let db = 20.0 * (rms.max(0.0) + 1e-6).log10();
        let raw = ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0);
        let rate = if raw > self.smooth { ATTACK } else { RELEASE };
        self.smooth += (raw - self.smooth) * rate;
        let onset = !self.spent && self.smooth > ONSET && self.smooth > self.previous;
        if onset {
            self.spent = true;
        }
        if self.smooth < REARM {
            self.spent = false;
        }
        self.previous = self.smooth;
        Voice {
            level: self.smooth,
            speaking: self.smooth > SPEAKING,
            onset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(meter: &mut Meter, rms: f32, frames: usize) -> (Voice, usize) {
        let mut onsets = 0;
        let mut last = meter.update(rms);
        onsets += usize::from(last.onset);
        for _ in 1..frames {
            last = meter.update(rms);
            onsets += usize::from(last.onset);
        }
        (last, onsets)
    }

    #[test]
    fn silence_is_not_speaking() {
        let (voice, onsets) = run(&mut Meter::default(), 0.0, 30);
        assert!(voice.level < 0.01);
        assert!(!voice.speaking);
        assert_eq!(onsets, 0);
    }

    #[test]
    fn speech_maps_db_to_level() {
        // 0.1 RMS is -20 dBFS: (-20 + 50) / 35 of the way from -50 to -15 dBFS.
        let (voice, _) = run(&mut Meter::default(), 0.1, 60);
        assert!((voice.level - 30.0 / 35.0).abs() < 0.02, "{}", voice.level);
        assert!(voice.speaking);
    }

    #[test]
    fn onset_fires_once_per_syllable() {
        let mut meter = Meter::default();
        let (_, first) = run(&mut meter, 0.1, 60);
        assert_eq!(first, 1);
        let (_, during_pause) = run(&mut meter, 0.0, 60);
        assert_eq!(during_pause, 0);
        let (_, second) = run(&mut meter, 0.1, 60);
        assert_eq!(second, 1);
    }
}
