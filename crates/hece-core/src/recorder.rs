//! Recording state machine: hotkey and capture events in, actions for the daemon out.
//!
//! A new recording may start while earlier ones are still being transcribed; results are
//! inserted in recording order. The daemon locks the target window when it executes
//! [`Action::StopCapture`], i.e. at stop time.

use std::collections::VecDeque;

/// Recording identifier, increasing in recording order.
pub type RecordingId = u64;

/// How the hotkey drives recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Press starts, the next press stops.
    Toggle,
    /// Press starts, release stops; repeated presses while held are ignored.
    PushToTalk,
}

/// Something that happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The record hotkey went down.
    Press,
    /// The record hotkey went up.
    Release,
    /// The cancel shortcut.
    Cancel,
    /// The capture hit its length limit.
    LimitReached,
    /// A stopped capture finished; `samples` is its length at 16 kHz.
    Captured {
        /// Recording.
        id: RecordingId,
        /// Number of samples.
        samples: usize,
    },
    /// Transcription finished.
    Transcribed {
        /// Recording.
        id: RecordingId,
        /// Whether it produced a transcript.
        ok: bool,
    },
}

/// What the daemon must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the microphone and record.
    StartCapture(RecordingId),
    /// Stop recording (after the post-roll) and lock the target window now.
    StopCapture(RecordingId),
    /// Close the microphone and drop the audio.
    DiscardCapture(RecordingId),
    /// Transcribe the captured audio.
    Transcribe(RecordingId),
    /// Abandon an in-flight transcription.
    Abort(RecordingId),
    /// Insert the transcript.
    Insert(RecordingId),
    /// Tell the user the transcription failed.
    Failed(RecordingId),
    /// Tell the user nothing was heard.
    NothingHeard(RecordingId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Capturing,
    Transcribing,
    Done { ok: bool },
}

/// The state machine.
#[derive(Debug)]
pub struct Recorder {
    mode: Mode,
    min_samples: usize,
    recording: Option<RecordingId>,
    next_id: RecordingId,
    /// Stopped recordings in order, until their result is inserted.
    jobs: VecDeque<(RecordingId, Stage)>,
}

impl Recorder {
    /// Recordings shorter than `min_samples` count as "nothing heard".
    #[must_use]
    pub fn new(mode: Mode, min_samples: usize) -> Self {
        Self {
            mode,
            min_samples,
            recording: None,
            next_id: 1,
            jobs: VecDeque::new(),
        }
    }

    /// Whether the microphone is recording.
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Applies `input` and returns the actions to run, in order.
    pub fn handle(&mut self, input: Input) -> Vec<Action> {
        match input {
            Input::Press => match (self.recording, self.mode) {
                (None, _) => {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.recording = Some(id);
                    vec![Action::StartCapture(id)]
                }
                (Some(_), Mode::Toggle) => self.stop(),
                (Some(_), Mode::PushToTalk) => Vec::new(),
            },
            Input::Release if self.mode == Mode::PushToTalk => self.stop(),
            Input::Release => Vec::new(),
            Input::LimitReached => self.stop(),
            Input::Cancel => self.cancel(),
            Input::Captured { id, samples } => {
                let min_samples = self.min_samples;
                let Some(stage) = self.stage_mut(id) else {
                    return Vec::new();
                };
                if samples >= min_samples {
                    *stage = Stage::Transcribing;
                    return vec![Action::Transcribe(id)];
                }
                self.jobs.retain(|(j, _)| *j != id);
                let mut out = vec![Action::NothingHeard(id)];
                out.extend(self.flush());
                out
            }
            Input::Transcribed { id, ok } => {
                if let Some(stage) = self.stage_mut(id) {
                    *stage = Stage::Done { ok };
                }
                self.flush()
            }
        }
    }

    fn stop(&mut self) -> Vec<Action> {
        let Some(id) = self.recording.take() else {
            return Vec::new();
        };
        self.jobs.push_back((id, Stage::Capturing));
        vec![Action::StopCapture(id)]
    }

    /// Cancels the recording in progress, else the most recent dictation not yet inserted.
    fn cancel(&mut self) -> Vec<Action> {
        if let Some(id) = self.recording.take() {
            return vec![Action::DiscardCapture(id)];
        }
        let Some((id, stage)) = self.jobs.pop_back() else {
            return Vec::new();
        };
        let mut out = match stage {
            Stage::Transcribing => vec![Action::Abort(id)],
            // A finished capture result for it is ignored; a done result is just dropped.
            Stage::Capturing | Stage::Done { .. } => Vec::new(),
        };
        out.extend(self.flush());
        out
    }

    fn stage_mut(&mut self, id: RecordingId) -> Option<&mut Stage> {
        self.jobs.iter_mut().find(|(j, _)| *j == id).map(|(_, s)| s)
    }

    /// Emits results that are next in recording order.
    fn flush(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        while let Some(&(id, Stage::Done { ok })) = self.jobs.front() {
            self.jobs.pop_front();
            out.push(if ok {
                Action::Insert(id)
            } else {
                Action::Failed(id)
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Action::*;
    use super::*;

    const MIN: usize = 4_800;

    fn run(r: &mut Recorder, inputs: &[Input]) -> Vec<Action> {
        inputs.iter().flat_map(|i| r.handle(*i)).collect()
    }

    fn captured(id: RecordingId) -> Input {
        Input::Captured { id, samples: MIN }
    }

    fn done(id: RecordingId) -> Input {
        Input::Transcribed { id, ok: true }
    }

    #[test]
    fn toggle_double_press_records_and_inserts() {
        let mut r = Recorder::new(Mode::Toggle, MIN);
        let out = run(
            &mut r,
            &[
                Input::Press,
                Input::Release,
                Input::Press,
                captured(1),
                done(1),
            ],
        );
        assert_eq!(
            out,
            [StartCapture(1), StopCapture(1), Transcribe(1), Insert(1)]
        );
    }

    #[test]
    fn push_to_talk_ignores_key_repeat() {
        let mut r = Recorder::new(Mode::PushToTalk, MIN);
        let out = run(
            &mut r,
            &[Input::Press, Input::Press, Input::Press, Input::Release],
        );
        assert_eq!(out, [StartCapture(1), StopCapture(1)]);
    }

    #[test]
    fn results_are_inserted_in_recording_order() {
        let mut r = Recorder::new(Mode::Toggle, MIN);
        run(&mut r, &[Input::Press, Input::Press, captured(1)]);
        run(&mut r, &[Input::Press, Input::Press, captured(2)]);
        assert_eq!(r.handle(done(2)), []);
        assert_eq!(
            r.handle(Input::Transcribed { id: 1, ok: false }),
            [Failed(1), Insert(2)]
        );
    }

    #[test]
    fn cancel_discards_recording_or_aborts_latest_dictation() {
        let mut r = Recorder::new(Mode::Toggle, MIN);
        assert_eq!(
            run(&mut r, &[Input::Press, Input::Cancel]),
            [StartCapture(1), DiscardCapture(1)]
        );
        assert!(!r.is_recording());
        run(&mut r, &[Input::Press, Input::Press, captured(2)]);
        run(&mut r, &[Input::Press, Input::Press, captured(3)]);
        assert_eq!(r.handle(Input::Cancel), [Abort(3)]);
        // A late result of the cancelled dictation is ignored; the earlier one still lands.
        assert_eq!(run(&mut r, &[done(3), done(2)]), [Insert(2)]);
    }

    #[test]
    fn limit_stops_and_processes_normally() {
        let mut r = Recorder::new(Mode::PushToTalk, MIN);
        let out = run(
            &mut r,
            &[
                Input::Press,
                Input::LimitReached,
                Input::Release,
                captured(1),
            ],
        );
        assert_eq!(out, [StartCapture(1), StopCapture(1), Transcribe(1)]);
    }

    #[test]
    fn empty_recording_is_nothing_heard_and_unblocks_later_results() {
        let mut r = Recorder::new(Mode::Toggle, MIN);
        run(&mut r, &[Input::Press, Input::Press]);
        run(&mut r, &[Input::Press, Input::Press, captured(2), done(2)]);
        assert_eq!(
            r.handle(Input::Captured {
                id: 1,
                samples: MIN - 1
            }),
            [NothingHeard(1), Insert(2)]
        );
    }
}
