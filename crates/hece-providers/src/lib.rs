//! Provider trait and adapters (local-ct2 client, OpenAI-compatible, Deepgram) plus API key
//! handling and whole-dictation fallback.

pub mod deepgram;
pub mod engine_client;
pub mod fallback;
mod http;
pub mod keys;
pub mod local;
pub mod openai;
pub mod provider;
pub mod vocab_hint;

pub use deepgram::{DeepgramConfig, DeepgramProvider};
pub use fallback::{FallbackNotice, Outcome, Providers};
pub use keys::{ApiKey, KeySource, KeyringStore, SecretStore, StoreError};
pub use local::LocalProvider;
pub use openai::{OpenAiConfig, OpenAiProvider};
pub use provider::{
    BoxFuture, Capabilities, Provider, ProviderError, ProviderRequest, VocabularyStyle,
};
pub use vocab_hint::{VocabularyHint, render};
