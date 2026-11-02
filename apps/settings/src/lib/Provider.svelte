<script lang="ts">
  import KeyRow from "./KeyRow.svelte";
  import Row from "./Row.svelte";
  import { cancelDownload, downloadModel, type Snapshot, type Value } from "./api";
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

  // OpenAI-compatible needs a URL and model; start from OpenAI's own so the switch is valid.
  function kindChanges(kind: string): Record<string, Value> {
    if (kind !== "openai") return { "provider.kind": kind };
    return {
      "provider.kind": kind,
      "provider.base_url": text("provider.base_url") || "https://api.openai.com/v1",
      "provider.model": text("provider.model") || "whisper-1",
    };
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
      if (code !== "model_download_cancelled") failure = code;
    } finally {
      percent = null;
    }
  }
</script>

<Row label={t.provider.engine} help={t.provider.engineHelp} note={notes.kind}>
  <select value={v("provider.kind") ?? "local"} onchange={(e) => change("kind", kindChanges(e.currentTarget.value))}>
    <option value="local">{t.provider.local}</option>
    <option value="openai">{t.provider.openai}</option>
    <option value="deepgram">Deepgram</option>
  </select>
</Row>

{#if (v("provider.kind") ?? "local") === "local"}
  <Row label={t.provider.model} help={modelHelp} note={notes.model}>
    <select value={chosen} disabled={percent !== null}
      onchange={(e) => pick(e.currentTarget.value)}>
      {#each snap.models as m (m.id)}
        <option value={m.id}>{m.name}{m.installed ? "" : ` · ${(m.size_mb / 1000).toFixed(1)} GB`}</option>
      {/each}
      {#if chosen === "custom"}
        <option value="custom">{t.provider.customFolder}</option>
      {/if}
    </select>
    {#if percent !== null}
      <button onclick={cancelDownload}>{t.provider.cancel}</button>
    {:else if selected && !selected.installed}
      <button class="primary" onclick={() => download(selected.id)}>{t.provider.download}</button>
    {/if}
  </Row>
{/if}

{#if v("provider.kind") === "openai"}
  <Row label={t.provider.url} help={t.provider.urlHelp} note={notes.url}>
    <input class="path" value={text("provider.base_url")} spellcheck="false"
      onchange={(e) => change("url", { "provider.base_url": e.currentTarget.value.trim() })} />
  </Row>
  <Row label={t.provider.model} help={t.provider.modelNameHelp} note={notes.modelName}>
    <input value={text("provider.model")} spellcheck="false"
      onchange={(e) => change("modelName", { "provider.model": e.currentTarget.value.trim() })} />
  </Row>
  <Row label={t.provider.name} help={t.provider.nameHelp} note={notes.name}>
    <input value={text("provider.name")} placeholder="openai" spellcheck="false"
      onchange={(e) => change("name", { "provider.name": e.currentTarget.value.trim() || null })} />
  </Row>
{/if}

{#if v("provider.kind") !== "local" && v("provider.kind") !== null}
  <KeyRow section="provider" stored={snap.keys.provider ?? false} onchange={refresh} />
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
