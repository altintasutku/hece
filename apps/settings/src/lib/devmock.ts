// Dev-only stand-in for the Tauri backend, so `bun run dev` renders in a plain browser for
// screenshots. Loaded only when Vite serves the app outside Tauri; never part of the build.
// `?daemon=not_running` shows the start banner, `?kind=openai` or `?kind=groq` the cloud rows.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Snapshot } from "./api";

const query = new URLSearchParams(location.search);

const snap: Snapshot = {
  config: "C:\\Users\\you\\AppData\\Roaming\\hece\\config.toml",
  values: {
    "provider.kind": query.get("kind") === "groq" ? "openai" : (query.get("kind") ?? "local"),
    "provider.model_dir": "",
    "provider.base_url": query.get("kind") === "groq" ? "https://api.groq.com/openai/v1" : "https://api.openai.com/v1",
    "provider.model": query.get("kind") === "groq" ? "whisper-large-v3" : "whisper-1",
    "provider.name": null,
    "transcription.language": null,
    "transcription.allow_cloud_fallback": false,
    "recording.mode": "hold",
    "recording.device": null,
    "recording.max_seconds": 600,
    "ui.overlay": true,
  },
  keys: { provider: query.get("kind") === "openai" },
  devices: ["Microphone (USB Audio Device)", "Headset Microphone (Wireless)"],
  models: [
    { id: "large-v3-turbo", name: "Whisper large-v3 turbo", size_mb: 1620, path: "C:\\models\\turbo", installed: false },
    { id: "large-v3-turkish", name: "Whisper large-v3 Turkish", size_mb: 3090, path: "C:\\models\\tr", installed: true },
  ],
  daemon: query.get("daemon") ?? "idle",
};

mockIPC((cmd) => {
  if (cmd === "snapshot") return structuredClone(snap);
  return "ok";
});
