//! Microphone capture (cpal) with resampling to 16 kHz mono `i16`.

pub mod capture;
pub mod resample;

pub use capture::{Capture, CaptureConfig, CaptureEvent, Level, Recording, input_devices};

/// Capture failures. [`AudioError::code`] is the stable user-facing code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// There is no default input device.
    #[error("no microphone found")]
    NoDevice,
    /// The configured device does not exist.
    #[error("the configured microphone was not found")]
    DeviceNotFound,
    /// The device disappeared or the audio system is not running.
    #[error("the microphone is not available")]
    Unavailable,
    /// Another application holds the device.
    #[error("the microphone is busy")]
    Busy,
    /// The OS refused access (privacy settings).
    #[error("access to the microphone was denied")]
    PermissionDenied,
    /// The device's sample format is not supported.
    #[error("the microphone's sample format is not supported")]
    UnsupportedFormat,
    /// Any other audio backend failure.
    #[error("audio error")]
    Backend,
    /// Resampling failed.
    #[error("audio conversion failed")]
    Resample,
    /// The capture thread failed.
    #[error("the capture thread failed")]
    Thread,
}

impl AudioError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoDevice => "mic_missing",
            Self::DeviceNotFound => "mic_not_found",
            Self::Unavailable => "mic_unavailable",
            Self::Busy => "mic_busy",
            Self::PermissionDenied => "mic_permission_denied",
            Self::UnsupportedFormat => "mic_unsupported_format",
            Self::Backend => "audio_backend",
            Self::Resample => "audio_resample",
            Self::Thread => "audio_thread",
        }
    }

    pub(crate) fn from_cpal(e: &cpal::Error) -> Self {
        use cpal::ErrorKind as K;
        match e.kind() {
            K::DeviceNotAvailable | K::HostUnavailable | K::StreamInvalidated => Self::Unavailable,
            K::DeviceBusy => Self::Busy,
            K::PermissionDenied => Self::PermissionDenied,
            K::UnsupportedConfig => Self::UnsupportedFormat,
            _ => Self::Backend,
        }
    }

    /// Whether a stream error of this kind leaves the stream running.
    pub(crate) fn is_transient(e: &cpal::Error) -> bool {
        matches!(
            e.kind(),
            cpal::ErrorKind::DeviceChanged
                | cpal::ErrorKind::Xrun
                | cpal::ErrorKind::RealtimeDenied
        )
    }
}
