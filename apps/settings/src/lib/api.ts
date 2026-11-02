import { Channel, invoke } from "@tauri-apps/api/core";

export type Value = string | number | boolean | null;

export interface Snapshot {
  config: string;
  values: Record<string, Value>;
  /** Section -> whether its provider has a key stored. */
  keys: Record<string, boolean>;
  devices: string[];
  models: ModelInfo[];
  daemon: string;
}

export const snapshot = () => invoke<Snapshot>("snapshot");
/** Saves values together; resolves to the daemon's reload answer. */
export const setValues = (changes: Record<string, Value>) => invoke<string>("set_values", { changes });
export const storeKey = (section: string, secret: string) =>
  invoke<string>("store_key", { section, secret });
export const deleteKey = (section: string) => invoke<string>("delete_key", { section });
export const startDaemon = () => invoke<void>("start_daemon");

export interface ModelInfo {
  id: string;
  name: string;
  size_mb: number;
  /** Folder it is (or will be) downloaded to. */
  path: string;
  installed: boolean;
}

/** Downloads a model, reporting the percentage; resolves to the daemon's reload answer. */
export function downloadModel(id: string, onPercent: (percent: number) => void) {
  const progress = new Channel<number>();
  progress.onmessage = onPercent;
  return invoke<string>("download_model", { id, progress });
}
export const cancelDownload = () => invoke<void>("cancel_download");
