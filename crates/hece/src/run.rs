//! `hece run`: dictation into any window. Hotkey and capture events feed the recording state
//! machine; its actions start and stop captures, run transcriptions and paste the results in
//! recording order. Status lines never contain transcript text.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use hece_audio::{AudioError, Capture, CaptureConfig, CaptureEvent, Recording};
use hece_core::recorder::{Action, Input, Mode, Recorder, RecordingId};
use hece_core::{Transcript, Vocabulary};
use hece_platform::{
    CANCEL_SHORTCUT, HotkeyEvent, Hotkeys, Inserted, RECORD_SHORTCUT, Shortcut, Window,
};
use hece_providers::{Provider, ProviderError, ProviderRequest};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::cli::{ModeArg, RunArgs};
use crate::dictate::setup;
use crate::output;

/// Recordings shorter than this (0.3 s at 16 kHz) count as "nothing heard".
const MIN_SAMPLES: usize = 4_800;

enum Msg {
    Hotkey(HotkeyEvent),
    Capture(RecordingId, CaptureEvent),
    Captured(RecordingId, Result<Recording, AudioError>),
    Transcribed(RecordingId, Result<Transcript, ProviderError>),
}

type Tx = mpsc::UnboundedSender<Msg>;

/// Owns one capture: opens the microphone, then stops (`true`) or discards (`false`) it on
/// command and reports the recording.
async fn capture(id: RecordingId, cfg: CaptureConfig, tx: Tx, command: oneshot::Receiver<bool>) {
    let events = tx.clone();
    let started = tokio::task::spawn_blocking(move || {
        Capture::start(&cfg, move |e| {
            let _ = events.send(Msg::Capture(id, e));
        })
    })
    .await;
    let capture = match started {
        Ok(Ok(capture)) => capture,
        Ok(Err(e)) => {
            let _ = tx.send(Msg::Captured(id, Err(e)));
            return;
        }
        Err(_) => {
            let _ = tx.send(Msg::Captured(id, Err(AudioError::Thread)));
            return;
        }
    };
    if command.await.unwrap_or(false) {
        let result = tokio::task::spawn_blocking(move || capture.stop())
            .await
            .unwrap_or(Err(AudioError::Thread));
        let _ = tx.send(Msg::Captured(id, result));
    } else {
        let _ = tokio::task::spawn_blocking(move || capture.cancel()).await;
    }
}

struct Daemon {
    provider: Arc<dyn Provider>,
    vocabulary: Option<Arc<Vocabulary>>,
    language: Option<String>,
    capture_cfg: CaptureConfig,
    tx: Tx,
    recorder: Recorder,
    /// The recording in progress; capture events of older ones are ignored.
    current: Option<RecordingId>,
    captures: HashMap<RecordingId, oneshot::Sender<bool>>,
    audio: HashMap<RecordingId, Vec<i16>>,
    targets: HashMap<RecordingId, Option<Window>>,
    tasks: HashMap<RecordingId, JoinHandle<()>>,
    results: HashMap<RecordingId, Transcript>,
    failures: HashMap<RecordingId, String>,
}

impl Daemon {
    fn input(&mut self, msg: Msg) -> Option<Input> {
        match msg {
            Msg::Hotkey(HotkeyEvent::Pressed(Shortcut::Record)) => Some(Input::Press),
            Msg::Hotkey(HotkeyEvent::Released(Shortcut::Record)) => Some(Input::Release),
            Msg::Hotkey(HotkeyEvent::Pressed(Shortcut::Cancel)) => Some(Input::Cancel),
            Msg::Capture(id, event) if self.current == Some(id) => match event {
                CaptureEvent::Live { latency } => {
                    output::status(&format!(
                        "● recording (mic live after {} ms)",
                        latency.as_millis()
                    ));
                    None
                }
                CaptureEvent::LimitReached => {
                    output::status("recording limit reached");
                    Some(Input::LimitReached)
                }
                CaptureEvent::StreamError { code } => {
                    output::status(&format!("microphone error ({code}); stopping"));
                    Some(Input::LimitReached)
                }
            },
            Msg::Hotkey(HotkeyEvent::Released(Shortcut::Cancel)) | Msg::Capture(..) => None,
            Msg::Captured(id, Ok(recording)) => {
                let samples = recording.samples.len();
                self.audio.insert(id, recording.samples);
                Some(Input::Captured { id, samples })
            }
            Msg::Captured(id, Err(e)) => {
                output::error(&format!("{e} ({})", e.code()));
                if self.current == Some(id) {
                    // The microphone never opened: end the recording.
                    return Some(Input::Cancel);
                }
                Some(Input::Captured { id, samples: 0 })
            }
            Msg::Transcribed(id, result) => {
                self.tasks.remove(&id);
                let ok = match result.map(Transcript::with_quality_layers) {
                    Ok(t) if t.text.is_empty() => {
                        self.failures.insert(id, "nothing heard".to_owned());
                        false
                    }
                    Ok(t) => {
                        self.results.insert(id, t);
                        true
                    }
                    Err(e) => {
                        self.failures.insert(id, format!("{e} ({})", e.code()));
                        false
                    }
                };
                Some(Input::Transcribed { id, ok })
            }
        }
    }

    async fn act(&mut self, action: Action) {
        match action {
            Action::StartCapture(id) => {
                self.current = Some(id);
                let (stop, command) = oneshot::channel();
                self.captures.insert(id, stop);
                tokio::spawn(capture(
                    id,
                    self.capture_cfg.clone(),
                    self.tx.clone(),
                    command,
                ));
            }
            Action::StopCapture(id) => {
                self.current = None;
                self.targets.insert(id, hece_platform::focused_window());
                if let Some(stop) = self.captures.remove(&id) {
                    let _ = stop.send(true);
                }
                output::status("transcribing…");
            }
            Action::DiscardCapture(id) => {
                self.current = None;
                if let Some(stop) = self.captures.remove(&id) {
                    let _ = stop.send(false);
                }
                output::status("cancelled");
            }
            Action::Transcribe(id) => self.transcribe(id),
            Action::Abort(id) => {
                if let Some(task) = self.tasks.remove(&id) {
                    task.abort();
                }
                self.audio.remove(&id);
                self.targets.remove(&id);
                output::status("cancelled");
            }
            Action::Insert(id) => self.insert(id).await,
            Action::Failed(id) => {
                self.targets.remove(&id);
                let why = self.failures.remove(&id).unwrap_or_default();
                output::status(&format!("not inserted: {why}"));
            }
            Action::NothingHeard(id) => {
                self.audio.remove(&id);
                self.targets.remove(&id);
                output::status("nothing heard");
            }
        }
    }

    fn transcribe(&mut self, id: RecordingId) {
        let Some(audio) = self.audio.remove(&id) else {
            return;
        };
        let provider = Arc::clone(&self.provider);
        let vocabulary = self.vocabulary.clone();
        let language = self.language.clone();
        let tx = self.tx.clone();
        let task = tokio::spawn(async move {
            let req = ProviderRequest {
                audio: &audio,
                language: language.as_deref(),
                vocabulary: vocabulary.as_deref(),
                prompt_prefix: hece_core::DEFAULT_PROMPT_PREFIX,
                prompt_tokens: hece_core::DEFAULT_PROMPT_TOKENS,
            };
            let result = provider.transcribe(req).await;
            let _ = tx.send(Msg::Transcribed(id, result));
        });
        self.tasks.insert(id, task);
    }

    async fn insert(&mut self, id: RecordingId) {
        let target = self.targets.remove(&id).flatten();
        let Some(transcript) = self.results.remove(&id) else {
            return;
        };
        let chars = transcript.text.chars().count();
        let low = transcript.low_confidence;
        let text = transcript.text;
        let result =
            tokio::task::spawn_blocking(move || hece_platform::insert(&text, target)).await;
        let note = if low { " (may be misheard)" } else { "" };
        match result {
            Ok(Ok(Inserted::Pasted)) => {
                output::status(&format!("inserted {chars} characters{note}"));
            }
            Ok(Ok(Inserted::FocusChanged)) => output::status(&format!(
                "focus changed, nothing pasted; the text is on the clipboard (Ctrl+V){note}"
            )),
            Ok(Err(e)) => output::error(&format!("{e} ({})", e.code())),
            Err(_) => output::error("insert failed"),
        }
    }
}

pub async fn execute(args: &RunArgs) -> anyhow::Result<()> {
    let setup = setup(&args.provider)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let hotkey_tx = tx.clone();
    let _hotkeys = Hotkeys::listen(move |e| {
        let _ = hotkey_tx.send(Msg::Hotkey(e));
    })?;
    if let Some(engine) = setup.engine.clone() {
        tokio::spawn(async move {
            output::status("loading the local model…");
            match engine.prewarm().await {
                Ok(()) => output::status("model loaded"),
                Err(e) => output::error(&format!("{e} ({})", e.code())),
            }
        });
    }
    let (mode, how) = match args.mode {
        ModeArg::Hold => (Mode::PushToTalk, "hold"),
        ModeArg::Toggle => (Mode::Toggle, "press"),
    };
    output::status(&format!(
        "ready: {how} {RECORD_SHORTCUT} to dictate, {CANCEL_SHORTCUT} cancels, Ctrl+C quits"
    ));
    let mut daemon = Daemon {
        provider: setup.provider,
        vocabulary: setup.vocabulary.map(Arc::new),
        language: args.provider.language.clone(),
        capture_cfg: CaptureConfig {
            device: args.device.clone(),
            max_duration: Duration::from_secs(args.max_secs),
            ..CaptureConfig::default()
        },
        tx,
        recorder: Recorder::new(mode, MIN_SAMPLES),
        current: None,
        captures: HashMap::new(),
        audio: HashMap::new(),
        targets: HashMap::new(),
        tasks: HashMap::new(),
        results: HashMap::new(),
        failures: HashMap::new(),
    };
    loop {
        let msg = tokio::select! {
            msg = rx.recv() => msg,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(msg) = msg else { break };
        if let Some(input) = daemon.input(msg) {
            for action in daemon.recorder.handle(input) {
                daemon.act(action).await;
            }
        }
    }
    if let Some(engine) = setup.engine {
        let _ = engine.shutdown().await;
    }
    Ok(())
}
