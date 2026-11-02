# Cloud providers

No NVIDIA GPU? Hece can send each dictation to a cloud speech-to-text provider instead of the
local engine. Three are built in: **Groq** and **OpenAI** (both through the OpenAI-compatible
adapter) and **Deepgram**.

**Privacy:** with a cloud provider, the recorded audio of every dictation (and your vocabulary,
sent as a prompt or key terms) leaves your computer and goes to that provider, under its terms.
Hece itself never logs audio, transcripts or keys. With the local engine nothing leaves the
machine.

## Quick start (settings window)

1. Get an API key (see the provider below).
2. Open **Hece Settings** → **Provider**.
3. **Engine**: pick **Groq (cloud)**, **OpenAI (cloud)** or **Deepgram (cloud)**. The address,
   model and name are filled in for you.
4. **API key** → **Add key**, paste the key, **Save**. It is stored in the Windows
   Credential Manager (entry named after the provider: `groq`, `openai`, `deepgram`) and never
   shown again or written to `config.toml`.
5. **Language**: leave **Auto-detect** to let the provider detect the language, or pin one
   (pinning is faster and more accurate for short sentences).
6. Every change reloads the running daemon. If Hece is not running, start it from the Start menu.
   Hold **Ctrl+Shift+Space**, speak, release.

## Groq

- Key: <https://console.groq.com/keys>
- Models: `whisper-large-v3` (default, more accurate) and `whisper-large-v3-turbo` (faster,
  cheaper). Pick one under **Model**.
- Limits (Groq docs, checked 2026-11-03): 25 MB per file on the free tier (100 MB on the dev
  tier). Hece sends 16 kHz mono WAV, about 1.9 MB per minute, so the 11-minute maximum is about
  21 MB and fits the free tier. Groq bills requests shorter than 10 s as 10 s. Rate limits depend
  on your plan; see the Groq console.

```toml
[provider]
kind = "openai"
name = "groq"
base_url = "https://api.groq.com/openai/v1"
model = "whisper-large-v3"
key = "keyring"            # `hece key set groq`
segments = true            # verbose_json: lets the confidence layer flag misheard dictations
```

## OpenAI

- Key: <https://platform.openai.com/api-keys>
- Model: `whisper-1` (default). The `gpt-4o-transcribe` models also work but only return plain
  `json`, so set `segments = false` for them (the settings window does this when the model name
  does not start with `whisper`). Unverified: OpenAI's 25 MB upload limit was confirmed only from
  third-party docs.

```toml
[provider]
kind = "openai"
base_url = "https://api.openai.com/v1"
model = "whisper-1"
key = "keyring"            # `hece key set openai`
segments = true
```

## Deepgram

- Key: <https://console.deepgram.com> (create a project key).
- Model: `nova-3` (default). With no language set, Hece asks Deepgram to detect it
  (`detect_language=true`; Turkish is on Deepgram's detection list). Unverified: whether
  `nova-3` accepts `language = "tr"` on your account; if Turkish is rejected
  (`provider_rejected`), set `model = "nova-2"`.
- Vocabulary terms are sent as Deepgram key terms (at most 50).

```toml
[provider]
kind = "deepgram"
key = "keyring"            # `hece key set deepgram`
# model = "nova-3"
```

## Command line instead of the settings window

```powershell
hece config                # prints the config.toml path; edit [provider] as above
hece key set groq          # prompts for the key without echo; the name must match [provider] name
hece key delete groq
```

The keyring entry is named after `[provider] name`, or after the kind when `name` is unset
(`openai`, `deepgram`). For CI or a sandbox, `key = "env:GROQ_API_KEY"` reads the key from that
environment variable instead. Never put the key itself in `config.toml`; Hece refuses it.

A self-hosted OpenAI-compatible server (speaches, whisper.cpp server, LocalAI) is the **OpenAI
uyumlu sunucu** choice. A key is sent only over https or to localhost; for a server without
authentication set `key = "none"` in `config.toml`.

## Fallback

`[fallback]` takes the same fields as `[provider]`. If the primary fails, the whole dictation is
transcribed again by the fallback (never mixed per chunk) and the tray shows
`groq failed (provider_network); deepgram transcribed this dictation.` A local primary with a
cloud fallback also needs `allow_cloud_fallback = true` under `[transcription]` (the
**Buluta yedekle** box), because it sends audio off the machine.

```toml
[fallback]
kind = "deepgram"
key = "keyring"
```

When both fail, the code is `all_providers_failed` and the message names both causes.

## Troubleshooting

A failed dictation is not inserted. The tray/toast shows `Not inserted: <provider>: <reason>
(<code>)`; the log (`%LOCALAPPDATA%\hece\hece.log`) has the HTTP status but never the key, audio or
text.

| Code | Meaning | What to do |
| --- | --- | --- |
| `provider_key_missing` | No key stored under the provider's name | Add the key in settings, or `hece key set <name>`; check that `name` matches |
| `provider_unauthorized` | 401/403: the key was rejected | Create a new key and store it again; the next dictation re-reads it |
| `provider_rate_limited` | 429: too many requests or quota used up | Wait a moment; check your plan and billing in the provider console |
| `provider_server_error` | 5xx from the provider | Provider outage; retry later or configure a `[fallback]` |
| `provider_timeout` | No answer in time (60 s plus 1 s per 10 s of audio) | Slow network or provider; retry |
| `provider_network` | Could not connect: offline, DNS failure, firewall or proxy | Check the internet connection |
| `audio_too_long` | Over 11 minutes, or 413 from the provider | Dictate shorter pieces; lower `max_seconds` |
| `provider_rejected` | Other 4xx, usually a wrong model name or an unsupported option | Check `model`; for non-Whisper models set `segments = false` |
| `provider_bad_response` | The answer was not understood | Wrong `base_url` (it must end in `/v1` for OpenAI-compatible servers) |
| `provider_config` | Invalid address, or a key would go over plain http | Use https (or localhost) |
| `keyring_locked`, `keyring_error` | Credential Manager refused access | Unlock the session; store the key again |

## Live test checklist (real key)

Run with the release build and a real Groq key; repeat the first block for OpenAI/Deepgram if you
have keys.

1. Fresh state: **Engine** → **Groq (cloud)**. Check `config.toml`: `kind = "openai"`,
   `name = "groq"`, the Groq `base_url`, `model = "whisper-large-v3"`, `segments = true`, no
   `language` line.
2. **Add key** → paste → **Save** shows "Saved". Credential Manager has a `groq`
   entry; `hece.log` does not contain the key.
3. Dictate a Turkish sentence (e.g. "Yarın saat üçte toplantı var, notları paylaşır mısın?")
   into Notepad: inserted, Turkish characters correct.
4. Dictate an English sentence ("Please send the report by Friday.") with **Language** on
   **Auto-detect**: inserted in English, not translated.
5. Note the latency of the first and later dictations (the 2026-11-02 sandbox run measured
   2.76 s, then 0.73 s).
6. Bad key: store `gsk_invalid` → dictate → `Not inserted: groq: the API key was rejected
   (provider_unauthorized)`. Store the real key again → the next dictation works without
   restarting Hece.
7. Offline: turn off Wi-Fi/Ethernet → dictate → `provider_network` (or `provider_timeout`) toast,
   nothing inserted. Reconnect → works again.
8. Switch **Engine** to **Deepgram (cloud)** and back to **Groq**: `config.toml` has no stale
   `base_url`/`name` from the other provider, and the key row shows the right stored state.
