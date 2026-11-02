//! Where to cut a long recording into chunks while it is still being recorded: at a natural
//! pause once a chunk is long enough, or at the quietest moment near the cap. Works on mono
//! samples at any rate, so the capture can cut before resampling.

/// Chunking limits, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkPolicy {
    /// No cut before this much audio.
    pub min_secs: f32,
    /// A chunk never grows past this.
    pub max_secs: f32,
    /// Silence at least this long counts as a pause.
    pub pause_secs: f32,
}

impl Default for ChunkPolicy {
    /// 15 s minimum, 30 s cap (Whisper's window), 0.5 s pauses.
    fn default() -> Self {
        Self {
            min_secs: 15.0,
            max_secs: 30.0,
            pause_secs: 0.5,
        }
    }
}

/// Analysis frame length.
const FRAME_SECS: f32 = 0.02;
/// Where to look for the quietest frame when no pause came before the cap.
const FALLBACK_WINDOW_SECS: f32 = 5.0;
/// Frames below this RMS are silent regardless of the speech level (~ -50 dBFS).
const ABS_SILENCE: f32 = 0.003;
/// Frames below this fraction of the median frame RMS are silent.
const REL_SILENCE: f32 = 0.1;

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "positive durations times sample rates, far below 2^24"
)]
fn samples(secs: f32, rate: u32) -> usize {
    (secs * rate as f32) as usize
}

#[expect(clippy::cast_precision_loss, reason = "frame lengths are tiny")]
fn rms(frame: &[f32]) -> f32 {
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt()
}

/// The end of the next chunk in `audio` (mono at `rate` Hz, starting at the previous cut), or
/// `None` while no cut is due yet.
#[must_use]
pub fn next_cut(audio: &[f32], rate: u32, policy: &ChunkPolicy) -> Option<usize> {
    let frame = samples(FRAME_SECS, rate).max(1);
    let min = samples(policy.min_secs, rate);
    let max = samples(policy.max_secs, rate);
    if audio.len() < min {
        return None;
    }
    let energies: Vec<f32> = audio.chunks_exact(frame).map(rms).collect();
    let mut sorted = energies.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let threshold = (median * REL_SILENCE).max(ABS_SILENCE);
    let pause_frames = (samples(policy.pause_secs, rate) / frame).max(1);

    // First pause that starts at or after the minimum length; cut in its middle.
    let first = min / frame;
    let mut run = 0;
    for (i, e) in energies.iter().enumerate() {
        run = if *e < threshold { run + 1 } else { 0 };
        let start = i + 1 - run;
        if run >= pause_frames && start >= first && start * frame < max {
            return Some((start + run / 2) * frame);
        }
    }
    if audio.len() < max {
        return None;
    }
    // No pause before the cap: the quietest frame in the last few seconds before it.
    let last = max / frame;
    let from = last.saturating_sub(samples(FALLBACK_WINDOW_SECS, rate) / frame);
    let quietest =
        (from..last.min(energies.len())).min_by(|a, b| energies[*a].total_cmp(&energies[*b]))?;
    Some(quietest * frame + frame / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1_000;

    /// `secs` of "speech" (a loud alternating signal) or silence.
    fn sound(secs: f32, loud: bool) -> Vec<f32> {
        let n = samples(secs, RATE);
        (0..n)
            .map(|i| match (loud, i % 2) {
                (true, 0) => 0.5,
                (true, _) => -0.5,
                (false, _) => 0.0,
            })
            .collect()
    }

    fn concat(parts: &[(f32, bool)]) -> Vec<f32> {
        parts.iter().flat_map(|(s, l)| sound(*s, *l)).collect()
    }

    #[test]
    fn no_cut_before_the_minimum_or_without_a_pause_before_the_cap() {
        let p = ChunkPolicy::default();
        assert_eq!(
            next_cut(&concat(&[(10.0, true), (1.0, false)]), RATE, &p),
            None
        );
        assert_eq!(next_cut(&sound(20.0, true), RATE, &p), None);
    }

    #[test]
    fn cuts_in_the_middle_of_the_first_pause_after_the_minimum() {
        let p = ChunkPolicy::default();
        // A pause at 5 s is too early; the one at 18 s (1 s long) is used.
        let audio = concat(&[
            (5.0, true),
            (1.0, false),
            (12.0, true),
            (1.0, false),
            (4.0, true),
        ]);
        let cut = next_cut(&audio, RATE, &p).unwrap();
        assert!((18_200..=18_800).contains(&cut), "cut at {cut}");
    }

    #[test]
    fn without_a_pause_cuts_at_the_quietest_moment_before_the_cap() {
        let p = ChunkPolicy::default();
        // A short dip (shorter than a pause) at 27 s.
        let audio = concat(&[(27.0, true), (0.2, false), (5.0, true)]);
        let cut = next_cut(&audio, RATE, &p).unwrap();
        assert!((27_000..=27_200).contains(&cut), "cut at {cut}");
    }
}
