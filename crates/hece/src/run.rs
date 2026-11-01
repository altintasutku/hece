//! The daemon: dictation into any window. Shortcut, tray, control-pipe and capture events feed
//! the recording state machine; its actions start and stop captures, run transcriptions and
//! paste the results in recording order. The tray icon shows the state; every failure is also
//! a toast. Status lines, notifications and logs never contain transcript text.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hece_audio::{AudioError, Capture, CaptureConfig, CaptureEvent, Recording};
use hece_core::recorder::{Action, Input, Mode, Recorder, RecordingId};
use hece_core::{Transcript, Vocabulary};
use hece_platform::{
    CANCEL_SHORTCUT, ControlListener, ControlRequest, Desktop, DesktopEvent, HotkeyEvent, Inserted,
    MenuAction, RECORD_SHORTCUT, Shortcut, TrayState, Window,
};
use hece_providers::{Outcome, ProviderError, ProviderRequest, Providers};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config::{Config, RecordMode};
use crate::history::{self, Entry, History};
use crate::output;
use crate::providers::setup;

/// Recordings shorter than this (0.3 s at 16 kHz) count as "nothing heard".
const MIN_SAMPLES: usize = 4_800;
/// How long a warning or error colour stays on the tray icon.
const FLAG_FOR: Duration = Duration::from_secs(6);
/// Time for the tray menu to close and focus to return before a re-paste.
const MENU_CLOSE: Duration = Duration::from_millis(300);

enum Msg {
    Desktop(DesktopEvent),
    Control(ControlRequest),
    Capture(RecordingId, CaptureEvent),
    Captured(RecordingId, Result<Recording, AudioError>),
    Transcribed(RecordingId, Result<Outcome, ProviderError>),
    RefreshTray,
}

type Tx = mpsc::UnboundedSender<Msg>;

/// Prints `message` and shows it as a toast.
fn toast(message: &str) {
    output::status(message);
    let message = message.to_owned();
    tokio::task::spawn_blocking(move || hece_platform::notify("Hece", &message));
}

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
    providers: Arc<Providers>,
    vocabulary: Option<Arc<Vocabulary>>,
    language: Option<String>,
    prompt_tokens: usize,
    mode: Mode,
    capture_cfg: CaptureConfig,
    history: Option<History>,
    config_file: PathBuf,
    state_dir: PathBuf,
    desktop: Desktop,
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
    /// A warning/error colour shown on the tray until the given time.
    flag: Option<(TrayState, Instant)>,
    quit: bool,
}

impl Daemon {
    fn pending(&self) -> usize {
        // A target is kept from stop until the dictation is inserted, failed or cancelled.
        self.targets.len()
    }

    fn status(&self) -> String {
        match (self.recorder.is_recording(), self.pending()) {
            (true, _) => "recording".to_owned(),
            (false, 0) => "idle".to_owned(),
            (false, n) => format!("processing {n}"),
        }
    }

    fn refresh_tray(&mut self) {
        let flagged = self.flag.filter(|(_, until)| Instant::now() < *until);
        self.flag = flagged;
        let state = if self.recorder.is_recording() {
            TrayState::Recording
        } else if self.pending() > 0 {
            TrayState::Processing
        } else {
            flagged.map_or(TrayState::Idle, |(state, _)| state)
        };
        self.desktop.set_state(state);
    }

    /// Toast plus a temporary warning/error colour on the tray.
    fn alert(&mut self, state: TrayState, message: &str) {
        toast(message);
        self.flag = Some((state, Instant::now() + FLAG_FOR));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FLAG_FOR).await;
            let _ = tx.send(Msg::RefreshTray);
        });
    }

    fn fail(&mut self, message: &str) {
        self.alert(TrayState::Error, message);
    }

    /// Start/stop regardless of the hotkey mode.
    fn toggle(&self) -> Input {
        match (self.recorder.is_recording(), self.mode) {
            (true, Mode::PushToTalk) => Input::Release,
            _ => Input::Press,
        }
    }

    async fn control(&self, request: ControlRequest) -> Option<Input> {
        match request.command.as_str() {
            "status" => {
                let status = self.status();
                request.reply(&status).await;
                None
            }
            "toggle" => {
                let input = self.toggle();
                request.reply("ok").await;
                Some(input)
            }
            "cancel" => {
                request.reply("ok").await;
                Some(Input::Cancel)
            }
            _ => {
                request.reply("unknown command").await;
                None
            }
        }
    }

    async fn menu(&mut self, action: MenuAction) -> Option<Input> {
        match action {
            MenuAction::Toggle => return Some(self.toggle()),
            MenuAction::Repaste | MenuAction::RepasteRaw => {
                self.repaste(action == MenuAction::RepasteRaw).await;
            }
            MenuAction::Settings => {
                let opened = std::process::Command::new("notepad.exe")
                    .arg(&self.config_file)
                    .spawn();
                if opened.is_err() {
                    self.fail("Could not open the config file.");
                }
            }
            MenuAction::Logs => {
                if hece_platform::open(&self.state_dir).is_err() {
                    self.fail("Could not open the log folder.");
                }
            }
            MenuAction::Quit => self.quit = true,
        }
        None
    }

    async fn repaste(&mut self, raw: bool) {
        let Some(history) = self.history.clone() else {
            self.fail("History is disabled, so there is nothing to paste again.");
            return;
        };
        let entries = tokio::task::spawn_blocking(move || history.entries())
            .await
            .unwrap_or_default();
        let Some(last) = entries.last() else {
            self.fail("History is empty.");
            return;
        };
        let text = if raw {
            last.raw.clone()
        } else {
            last.text().to_owned()
        };
        tokio::time::sleep(MENU_CLOSE).await;
        let pasted = tokio::task::spawn_blocking(move || hece_platform::insert(&text, None)).await;
        if !matches!(pasted, Ok(Ok(Inserted::Pasted))) {
            self.fail("Could not paste again.");
        }
    }

    async fn input(&mut self, msg: Msg) -> Option<Input> {
        match msg {
            Msg::Desktop(DesktopEvent::Hotkey(HotkeyEvent::Pressed(Shortcut::Record))) => {
                Some(Input::Press)
            }
            Msg::Desktop(DesktopEvent::Hotkey(HotkeyEvent::Released(Shortcut::Record))) => {
                Some(Input::Release)
            }
            Msg::Desktop(DesktopEvent::Hotkey(HotkeyEvent::Pressed(Shortcut::Cancel))) => {
                Some(Input::Cancel)
            }
            Msg::Desktop(DesktopEvent::Menu(action)) => self.menu(action).await,
            Msg::Control(request) => self.control(request).await,
            Msg::Capture(id, event) if self.current == Some(id) => match event {
                CaptureEvent::Live { latency } => {
                    tracing::info!(latency_ms = latency.as_millis(), "mic live");
                    output::status("● recording");
                    None
                }
                CaptureEvent::LimitReached => {
                    self.alert(
                        TrayState::Warning,
                        "Recording limit reached; transcribing what was recorded.",
                    );
                    Some(Input::LimitReached)
                }
                CaptureEvent::StreamError { code } => {
                    self.fail(&format!("Microphone error ({code}); stopping."));
                    Some(Input::LimitReached)
                }
            },
            Msg::Desktop(DesktopEvent::Hotkey(HotkeyEvent::Released(Shortcut::Cancel)))
            | Msg::Capture(..)
            | Msg::RefreshTray => None,
            Msg::Captured(id, Ok(recording)) => {
                let samples = recording.samples.len();
                self.audio.insert(id, recording.samples);
                Some(Input::Captured { id, samples })
            }
            Msg::Captured(id, Err(e)) => {
                tracing::warn!(code = e.code(), "capture failed");
                self.fail(&format!("{e} ({})", e.code()));
                if self.current == Some(id) {
                    // The microphone never opened: end the recording.
                    return Some(Input::Cancel);
                }
                Some(Input::Captured { id, samples: 0 })
            }
            Msg::Transcribed(id, result) => {
                self.tasks.remove(&id);
                let ok = self.transcribed(id, result);
                Some(Input::Transcribed { id, ok })
            }
        }
    }

    fn transcribed(&mut self, id: RecordingId, result: Result<Outcome, ProviderError>) -> bool {
        match result {
            Ok(outcome) => {
                if let Some(notice) = &outcome.fell_back {
                    tracing::warn!(from = %notice.from, to = %notice.to, reason = %notice.reason, "fell back");
                    self.alert(
                        TrayState::Warning,
                        &format!(
                            "{} failed ({}); {} transcribed this dictation.",
                            notice.from, notice.reason, notice.to
                        ),
                    );
                }
                let t = outcome.transcript.with_quality_layers();
                if t.text.is_empty() {
                    self.failures.insert(id, "Nothing heard.".to_owned());
                    false
                } else {
                    self.results.insert(id, t);
                    true
                }
            }
            Err(e) => {
                tracing::warn!(code = e.code(), "transcription failed");
                self.failures
                    .insert(id, format!("Not inserted: {e} ({})", e.code()));
                false
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
            Action::Failed(id) | Action::NothingHeard(id) => {
                self.audio.remove(&id);
                self.targets.remove(&id);
                let why = self
                    .failures
                    .remove(&id)
                    .unwrap_or_else(|| "Nothing heard.".to_owned());
                self.fail(&why);
            }
        }
    }

    fn transcribe(&mut self, id: RecordingId) {
        let Some(audio) = self.audio.remove(&id) else {
            return;
        };
        let providers = Arc::clone(&self.providers);
        let vocabulary = self.vocabulary.clone();
        let language = self.language.clone();
        let prompt_tokens = self.prompt_tokens;
        let tx = self.tx.clone();
        let task = tokio::spawn(async move {
            let req = ProviderRequest {
                audio: &audio,
                language: language.as_deref(),
                vocabulary: vocabulary.as_deref(),
                prompt_prefix: hece_core::DEFAULT_PROMPT_PREFIX,
                prompt_tokens,
            };
            let result = providers.transcribe(req).await;
            let _ = tx.send(Msg::Transcribed(id, result));
        });
        self.tasks.insert(id, task);
    }

    async fn insert(&mut self, id: RecordingId) {
        let target = self.targets.remove(&id).flatten();
        let Some(transcript) = self.results.remove(&id) else {
            return;
        };
        if let Some(history) = self.history.clone() {
            let entry = Entry {
                time: history::now(),
                raw: transcript.text.clone(),
                cleaned: None,
                provider: transcript.provider.clone(),
                low_confidence: transcript.low_confidence,
            };
            tokio::task::spawn_blocking(move || {
                if let Err(e) = history.append(entry) {
                    tracing::warn!(kind = ?e.kind(), "history write failed");
                }
            });
        }
        let chars = transcript.text.chars().count();
        let low = transcript.low_confidence;
        let text = transcript.text;
        let result =
            tokio::task::spawn_blocking(move || hece_platform::insert(&text, target)).await;
        match result {
            Ok(Ok(Inserted::Pasted)) if low => {
                self.alert(TrayState::Warning, "Inserted, but it may be misheard.");
            }
            Ok(Ok(Inserted::Pasted)) => output::status(&format!("inserted {chars} characters")),
            Ok(Ok(Inserted::FocusChanged)) => self.alert(
                TrayState::Warning,
                "Focus changed, so nothing was pasted. The text is on the clipboard (Ctrl+V).",
            ),
            Ok(Err(e)) => {
                tracing::warn!(code = e.code(), "insert failed");
                self.fail(&format!("{e} ({})", e.code()));
            }
            Err(_) => self.fail("Insert failed."),
        }
    }
}

/// Paths the daemon works with.
pub struct Paths {
    pub config_file: PathBuf,
    pub state_dir: PathBuf,
    pub control: PathBuf,
}

/// Runs the daemon until Ctrl+C or "Quit" in the tray.
pub async fn execute(cfg: Config, paths: Paths) -> anyhow::Result<()> {
    let listener = ControlListener::bind(&paths.control)?;
    let setup = setup(&cfg)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let desktop_tx = tx.clone();
    let desktop = Desktop::start(move |e| {
        let _ = desktop_tx.send(Msg::Desktop(e));
    })?;
    let control_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let request = listener.next().await;
            if control_tx.send(Msg::Control(request)).is_err() {
                break;
            }
        }
    });
    if let Some(engine) = setup.engine.clone() {
        tokio::spawn(async move {
            output::status("loading the local model…");
            match engine.prewarm().await {
                Ok(()) => output::status("model loaded"),
                Err(e) => toast(&format!(
                    "The local model could not be loaded: {e} ({})",
                    e.code()
                )),
            }
        });
    }
    let (mode, how) = match cfg.recording.mode {
        RecordMode::Hold => (Mode::PushToTalk, "hold"),
        RecordMode::Toggle => (Mode::Toggle, "press"),
    };
    output::status(&format!(
        "ready: {how} {RECORD_SHORTCUT} to dictate, {CANCEL_SHORTCUT} cancels, Ctrl+C quits"
    ));
    tracing::info!(mode = how, "daemon ready");
    let mut daemon = Daemon {
        providers: Arc::new(setup.providers),
        vocabulary: setup.vocabulary,
        language: setup.language,
        prompt_tokens: setup.prompt_tokens,
        mode,
        capture_cfg: CaptureConfig {
            device: cfg.recording.device.clone(),
            max_duration: Duration::from_secs(cfg.recording.max_seconds),
            ..CaptureConfig::default()
        },
        history: cfg
            .history
            .enabled
            .then(|| History::new(&paths.state_dir, &cfg.history)),
        config_file: paths.config_file,
        state_dir: paths.state_dir,
        desktop,
        tx,
        recorder: Recorder::new(mode, MIN_SAMPLES),
        current: None,
        captures: HashMap::new(),
        audio: HashMap::new(),
        targets: HashMap::new(),
        tasks: HashMap::new(),
        results: HashMap::new(),
        failures: HashMap::new(),
        flag: None,
        quit: false,
    };
    while !daemon.quit {
        let msg = tokio::select! {
            msg = rx.recv() => msg,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(msg) = msg else { break };
        if let Some(input) = daemon.input(msg).await {
            for action in daemon.recorder.handle(input) {
                daemon.act(action).await;
            }
        }
        daemon.refresh_tray();
    }
    tracing::info!("daemon stopping");
    if let Some(engine) = setup.engine {
        let _ = engine.shutdown().await;
    }
    Ok(())
}
