//! Renders the recording overlay offline for videos: one dictation driven by a voice recording,
//! written to stdout as raw RGBA frames (straight alpha) for ffmpeg. The sphere opens, listens
//! for the length of the WAV, transcribes for a moment and collapses as if the text was pasted.
//!
//! ```sh
//! cargo run --release -p hece-platform --example overlay_render -- voice.wav 1440 \
//!   | ffmpeg -f rawvideo -pix_fmt rgba -s 1440x1440 -r 60 -i - -c:v png orb/%04d.png
//! ```
//!
//! Arguments: `<voice.wav> [size_px = 1440] [fps = 60] [lead_s = 0.4] [work_s = 0.8]`. The voice
//! starts `lead_s` after the sphere appears, so frame `lead_s * fps` lines up with the WAV's start.

use std::error::Error;
use std::io::{BufWriter, Write};
use std::time::Duration;

use hece_platform::overlay::{EXIT, Outcome, Phase, Scene};
use tiny_skia::Pixmap;

/// The capture publishes the RMS of its latest audio block; blocks are about this long.
const BLOCK_S: f64 = 0.01;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(wav) = args.first() else {
        return Err("usage: overlay_render <voice.wav> [size_px] [fps] [lead_s] [work_s]".into());
    };
    let size: u32 = arg(&args, 1, 1440)?;
    let fps: u32 = arg(&args, 2, 60)?;
    let lead: f64 = arg(&args, 3, 0.4)?;
    let work: f64 = arg(&args, 4, 0.8)?;

    let (samples, rate) = read_mono(wav)?;
    #[allow(
        clippy::cast_precision_loss,
        reason = "a voice clip has far fewer samples than f64 can count"
    )]
    let voice_end = lead + samples.len() as f64 / f64::from(rate);
    let finish = voice_end + work;
    let end = finish + EXIT.as_secs_f64() + 0.1;

    let mut scene = Scene::new(false);
    let mut pixmap = Pixmap::new(size, size).ok_or("size must be above zero")?;
    let mut out = BufWriter::new(std::io::stdout().lock());
    let mut frame = Vec::with_capacity(pixmap.data().len());
    for n in 0u32.. {
        let t = f64::from(n) / f64::from(fps);
        if t >= end {
            break;
        }
        let phase = if t < lead {
            Phase::Opening
        } else if t < voice_end {
            Phase::Listening
        } else if t < finish {
            Phase::Transcribing
        } else {
            Phase::Finished(Outcome::Inserted)
        };
        let now = Duration::from_secs_f64(t);
        scene.set_phase(phase, now);
        let rms = if phase == Phase::Listening {
            block_rms(&samples, rate, t - lead)
        } else {
            0.0
        };
        scene.tick(now, rms);
        scene.draw(&mut pixmap);
        frame.clear();
        for px in pixmap.pixels() {
            let c = px.demultiply();
            frame.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        out.write_all(&frame)?;
    }
    out.flush()?;
    Ok(())
}

/// The positional argument at `i`, or `default` when it is missing.
fn arg<T: std::str::FromStr>(args: &[String], i: usize, default: T) -> Result<T, Box<dyn Error>>
where
    T::Err: Error + 'static,
{
    match args.get(i) {
        Some(s) => Ok(s.parse()?),
        None => Ok(default),
    }
}

/// The WAV as mono f32 samples in -1..1, with its sample rate.
fn read_mono(path: &str) -> Result<(Vec<f32>, u32), Box<dyn Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            #[allow(
                clippy::cast_precision_loss,
                reason = "audio sample scale, at most 2^31"
            )]
            let full = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| {
                    s.map(|v| {
                        #[allow(clippy::cast_precision_loss, reason = "audio samples fit in f32")]
                        let v = v as f32;
                        v / full
                    })
                })
                .collect::<Result<_, _>>()?
        }
    };
    let channels = usize::from(spec.channels.max(1));
    #[allow(clippy::cast_precision_loss, reason = "channel count is tiny")]
    let mono = interleaved
        .chunks(channels)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}

/// RMS of the audio block that ends at `t` seconds into the voice.
fn block_rms(samples: &[f32], rate: u32, t: f64) -> f32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "t is non-negative and within the clip"
    )]
    let (end, len) = (
        ((t * f64::from(rate)) as usize).min(samples.len()),
        (BLOCK_S * f64::from(rate)) as usize,
    );
    let block = &samples[end.saturating_sub(len)..end];
    if block.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "a block has a few hundred samples"
    )]
    let mean = block.iter().map(|s| s * s).sum::<f32>() / block.len() as f32;
    mean.sqrt()
}
