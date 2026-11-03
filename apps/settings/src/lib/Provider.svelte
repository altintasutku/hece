<script lang="ts">
  import KeyRow from "./KeyRow.svelte";
  import Row from "./Row.svelte";
  import { cancelDownload, downloadCuda, downloadModel, type Snapshot, type Value } from "./api";
  import type { Note } from "./save";
  import { t } from "./strings";

  interface Props {
    snap: Snapshot;
    save: (changes: Record<string, Value>) => Promise<Note>;
    refresh: () => void;
  }
  let { snap, save, refresh }: Props = $props();

  let notes: Record<string, Note | null> = $state({});
  const v = (key: string) => snap.values[key];
  const text = (key: string) => (v(key) as string | null) ?? "";

  async function change(row: string, changes: Record<string, Value>) {
    notes[row] = await save(changes);
    setTimeout(() => (notes[row] = null), 3000);
  }

  // Engine choices. Cloud presets fill URL, model and name so nobody types a URL; every choice
  // also clears the fields it does not use (a Groq base_url left behind would send Deepgram
  // requests to Groq). `segments` (verbose_json) only for Whisper models, which support it.
  type Choice = "local" | "groq" | "openai" | "deepgram" | "custom";
  const GROQ_URL = "https://api.groq.com/openai/v1";
  const OPENAI_URL = "https://api.openai.com/v1";
  const GROQ_MODELS = ["whisper-large-v3", "whisper-large-v3-turbo"];
  const unused = {
    "provider.name": null,
    "provider.base_url": null,
    "provider.model": null,
    "provider.segments": null,
  };
  const presets: Record<Exclude<Choice, "custom">, Record<string, Value>> = {
    local: { ...unused, "provider.kind": "local" },
    groq: {
      "provider.kind": "openai",
      "provider.name": "groq",
      "provider.base_url": GROQ_URL,
      "provider.model": GROQ_MODELS[0],
      "provider.segments": true,
    },
    openai: {
      "provider.kind": "openai",
      "provider.name": null,
      "provider.base_url": OPENAI_URL,
      "provider.model": "whisper-1",
      "provider.segments": true,
    },
    deepgram: { ...unused, "provider.kind": "deepgram" },
  };

  const choice: Choice = $derived.by(() => {
    const kind = v("provider.kind") ?? "local";
    if (kind === "local" || kind === "deepgram") return kind;
    const url = text("provider.base_url").replace(/\/+$/, "");
    if (url === GROQ_URL) return "groq";
    if (url === OPENAI_URL) return "openai";
    return "custom";
  });

  async function choose(next: string) {
    const changes =
      next === "custom"
        ? {
            "provider.kind": "openai",
            "provider.base_url": "http://127.0.0.1:8000/v1",
            "provider.model": text("provider.model") || "whisper-1",
            "provider.segments": null,
          }
        : presets[next as keyof typeof presets];
    await change("kind", changes);
    // The keyring entry is named after the provider, so the stored-key state may differ.
    refresh();
  }

  function modelChanges(model: string): Record<string, Value> {
    const changes: Record<string, Value> = { "provider.model": model };
    if (choice !== "custom") changes["provider.segments"] = model.startsWith("whisper") || null;
    return changes;
  }

  // The model in use: the catalog entry whose folder model_dir points at, else the first
  // downloaded one (what the daemon picks when model_dir is empty), else a custom folder.
  let wanted: string | null = $state(null);
  const fromConfig = $derived.by(() => {
    const dir = text("provider.model_dir");
    if (!dir) return (snap.models.find((m) => m.installed) ?? snap.models[0])?.id ?? "custom";
    return snap.models.find((m) => m.path === dir)?.id ?? "custom";
  });
  const chosen = $derived(wanted ?? fromConfig);
  const selected = $derived(snap.models.find((m) => m.id === chosen));
  let percent: number | null = $state(null);
  let failure: string | null = $state(null);
  const modelHelp = $derived(
    percent !== null
      ? t.provider.downloading(percent)
      : failure
        ? t.provider.downloadFailed(failure)
        : selected && !selected.installed
          ? t.provider.downloadOnce((selected.size_mb / 1000).toFixed(1))
          : t.provider.localHelp,
  );

  async function pick(id: string) {
    wanted = id;
    failure = null;
    const m = snap.models.find((x) => x.id === id);
    if (m?.installed) await change("model", { "provider.model_dir": m.path });
  }

  async function download(id: string) {
    failure = null;
    percent = 0;
    try {
      await downloadModel(id, (p) => (percent = p));
      wanted = null;
      refresh();
    } catch (e) {
      const code = String(e);
      if (code !== "download_cancelled") failure = code;
    } finally {
      percent = null;
    }
  }

  // The CUDA runtime row: only when the runtime is missing.
  let cudaPercent: number | null = $state(null);
  let cudaFailure: string | null = $state(null);
  const busy = $derived(percent !== null || cudaPercent !== null);
  const cudaHelp = $derived(
    !snap.cuda.gpu
      ? t.cuda.noGpu
      : cudaPercent !== null
        ? t.provider.downloading(cudaPercent)
        : cudaFailure
          ? t.provider.downloadFailed(cudaFailure)
          : t.cuda.missing(snap.cuda.size_mb),
  );

  async function downloadRuntime() {
    cudaFailure = null;
    cudaPercent = 0;
    try {
      notes.cuda = t.savedNote(await downloadCuda((p) => (cudaPercent = p)));
      setTimeout(() => (notes.cuda = null), 3000);
      refresh();
    } catch (e) {
      const code = String(e);
      if (code !== "download_cancelled") cudaFailure = code;
    } finally {
      cudaPercent = null;
    }
  }
</script>

<Row label={t.provider.engine} help={t.provider.engineHelp} note={notes.kind}>
  <select value={choice} onchange={(e) => choose(e.currentTarget.value)}>
    <option value="local">{t.provider.local}</option>
    <option value="groq">{t.provider.groq}</option>
    <option value="openai">{t.provider.openai}</option>
    <option value="deepgram">{t.provider.deepgram}</option>
    <option value="custom">{t.provider.custom}</option>
  </select>
</Row>

{#if (v("provider.kind") ?? "local") === "local"}
  <Row label={t.provider.model} help={modelHelp} note={notes.model}>
    <select value={chosen} disabled={percent !== null}
      onchange={(e) => pick(e.currentTarget.value)}>
      {#each snap.models as m (m.id)}
        <option value={m.id}>{m.name}{m.installed ? "" : ` · ${(m.size_mb / 1000).toFixed(1)} GB`}</option>
      {/each}
      <option value="custom">{t.provider.customFolder}</option>
    </select>
    {#if percent !== null}
      <button onclick={cancelDownload}>{t.provider.cancel}</button>
    {:else if selected && !selected.installed}
      <button class="primary" disabled={busy} onclick={() => download(selected.id)}>{t.provider.download}</button>
    {/if}
  </Row>
  {#if chosen === "custom"}
    <Row label={t.provider.modelDir} help={t.provider.modelDirHelp} note={notes.dir}>
      <input class="path" value={fromConfig === "custom" ? text("provider.model_dir") : ""} spellcheck="false"
        placeholder="C:\models\my-model"
        onchange={(e) => change("dir", { "provider.model_dir": e.currentTarget.value.trim() || null })} />
    </Row>
  {/if}
  {#if snap.cuda.state === "missing"}
    <Row label={t.cuda.label} help={cudaHelp} note={notes.cuda}>
      {#if cudaPercent !== null}
        <button onclick={cancelDownload}>{t.provider.cancel}</button>
      {:else if snap.cuda.gpu}
        <button class="primary" disabled={busy} onclick={downloadRuntime}>{t.provider.download}</button>
      {/if}
    </Row>
  {/if}
{/if}

{#if choice === "groq"}
  <Row label={t.provider.model} help={t.provider.groqModelHelp} note={notes.modelName}>
    <select value={text("provider.model")} onchange={(e) => change("modelName", modelChanges(e.currentTarget.value))}>
      {#each GROQ_MODELS as m (m)}
        <option value={m}>{m}</option>
      {/each}
      {#if !GROQ_MODELS.includes(text("provider.model"))}
        <option value={text("provider.model")}>{text("provider.model")}</option>
      {/if}
    </select>
  </Row>
{:else if choice === "openai" || choice === "custom"}
  {#if choice === "custom"}
    <Row label={t.provider.url} help={t.provider.urlHelp} note={notes.url}>
      <input class="path" value={text("provider.base_url")} spellcheck="false"
        onchange={(e) => change("url", { "provider.base_url": e.currentTarget.value.trim() })} />
    </Row>
  {/if}
  <Row label={t.provider.model} help={t.provider.modelNameHelp} note={notes.modelName}>
    <input value={text("provider.model")} spellcheck="false"
      onchange={(e) => change("modelName", modelChanges(e.currentTarget.value.trim()))} />
  </Row>
  {#if choice === "custom"}
    <Row label={t.provider.name} help={t.provider.nameHelp} note={notes.name}>
      <input value={text("provider.name")} placeholder="openai" spellcheck="false"
        onchange={async (e) => { await change("name", { "provider.name": e.currentTarget.value.trim() || null }); refresh(); }} />
    </Row>
  {/if}
{/if}

{#if choice !== "local"}
  <KeyRow section="provider" stored={snap.keys.provider ?? false} onchange={refresh}
    help={t.provider.keyHelp[choice]} />
{/if}

<Row label={t.provider.language} help={t.provider.languageHelp} note={notes.lang}>
  <select value={text("transcription.language")}
    onchange={(e) => change("lang", { "transcription.language": e.currentTarget.value || null })}>
    {#each t.provider.languages as [code, name] (code)}
      <option value={code}>{name}</option>
    {/each}
  </select>
</Row>

<Row label={t.provider.fallback} help={t.provider.fallbackHelp} note={notes.fallback}>
  <input type="checkbox" checked={v("transcription.allow_cloud_fallback") === true}
    onchange={(e) => change("fallback", { "transcription.allow_cloud_fallback": e.currentTarget.checked })} />
</Row>

<style>
  .path { width: 300px; }
</style>
