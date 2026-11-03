// Dev-only stand-in for the Tauri backend, so `bun run dev` renders in a plain browser for
// screenshots. Loaded only when Vite serves the app outside Tauri; never part of the build.
// `?daemon=not_running` shows the start banner, `?kind=openai` or `?kind=groq` the cloud rows,
// `?hotkey=Alt+F13` a changed record shortcut, `?conflict=1` a shortcut another app owns.
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
    "recording.hotkey": query.get("hotkey"),
    "recording.cancel_hotkey": null,
  },
  keys: { provider: query.get("kind") === "openai" },
  devices: ["Microphone (USB Audio Device)", "Headset Microphone (Wireless)"],
  models: [
    { id: "large-v3-turbo", name: "Whisper large-v3 turbo", size_mb: 1620, path: "C:\\models\\turbo", installed: false },
    { id: "large-v3-turkish", name: "Whisper large-v3 Turkish", size_mb: 3090, path: "C:\\models\\tr", installed: true },
  ],
  daemon: query.get("daemon") ?? "idle",
};

// Synthetic vocabulary. `?vocab=empty` shows the empty list, `?vocab=long` the length limit.
const words = ["Kubernetes", "PostgreSQL", "Tauri", "Svelte", "Alpha Bravo", "postgresql", "a, b", "CTranslate2"];
let entries = query.get("vocab") === "empty" ? [] : query.get("vocab") === "long" ? [...words, "Grafana", "Zigbee"] : words;
const fitLimit = query.get("vocab") === "long" ? 6 : Infinity;

/** A rough stand-in for hece::vocabulary::check. */
function check(list: string[]) {
  const seen = new Set<string>();
  const statuses = list.map((raw) => {
    const term = raw.trim();
    if (!term || term.length > 40 || term.includes(",") || /^[#!]/.test(term)) return "invalid";
    if (seen.has(term.toLowerCase())) return "duplicate";
    seen.add(term.toLowerCase());
    return "ok";
  });
  return { statuses, terms: seen.size, fit: Math.min(seen.size, fitLimit) };
}

let conflicts = query.get("conflict") === "1";
mockIPC((cmd, args) => {
  if (cmd === "snapshot") return structuredClone(snap);
  if (cmd === "vocabulary") {
    return { path: "C:\\Users\\you\\AppData\\Roaming\\hece\\vocabulary.txt", custom: false, entries, check: check(entries) };
  }
  if (cmd === "save_vocabulary") {
    entries = (args as { entries: string[] }).entries;
    return [check(entries), "ok"];
  }
  if (cmd === "import_vocabulary") return ["Grafana", "Zigbee"];
  if (cmd === "set_values" && conflicts) {
    // The first save is refused, the restore that follows succeeds.
    conflicts = false;
    return "hotkey_in_use";
  }
  return "ok";
});
