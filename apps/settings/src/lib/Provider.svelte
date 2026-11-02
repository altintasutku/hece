<script lang="ts">
  import KeyRow from "./KeyRow.svelte";
  import Row from "./Row.svelte";
  import { cancelDownload, downloadModel, type Snapshot, type Value } from "./api";
  import type { Note } from "./save";

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
  const languages: [string, string][] = [["tr", "Türkçe"], ["en", "İngilizce"], ["", "Otomatik algıla"]];

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
      ? `İndiriliyor… %${percent}`
      : failure
        ? `İndirilemedi (${failure}); tekrar denersen kaldığı yerden devam eder`
        : selected && !selected.installed
          ? `Bir kez indirilir (${(selected.size_mb / 1000).toFixed(1)} GB), sonra internetsiz çalışır`
          : "Konuşmayı bu bilgisayarda, ekran kartında yazıya çevirir",
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

<Row label="Motor" help="Yerel: Türkçe Whisper, ekran kartında. Bulut: anahtar gerekir." note={notes.kind}>
  <select value={v("provider.kind") ?? "local"} onchange={(e) => change("kind", kindChanges(e.currentTarget.value))}>
    <option value="local">Yerel (GPU)</option>
    <option value="openai">OpenAI uyumlu</option>
    <option value="deepgram">Deepgram</option>
  </select>
</Row>

{#if (v("provider.kind") ?? "local") === "local"}
  <Row label="Model" help={modelHelp} note={notes.model}>
    <select value={chosen} disabled={percent !== null}
      onchange={(e) => pick(e.currentTarget.value)}>
      {#each snap.models as m (m.id)}
        <option value={m.id}>{m.name}{m.installed ? "" : ` · ${(m.size_mb / 1000).toFixed(1)} GB`}</option>
      {/each}
      <option value="custom">Kendi klasörün…</option>
    </select>
    {#if percent !== null}
      <button onclick={cancelDownload}>Vazgeç</button>
    {:else if selected && !selected.installed}
      <button onclick={() => download(selected.id)}>İndir</button>
    {/if}
  </Row>
  {#if chosen === "custom"}
    <Row label="Model klasörü" help="CTranslate2 Whisper modeli (içinde model.bin olan klasör)" note={notes.dir}>
      <input class="path" value={fromConfig === "custom" ? text("provider.model_dir") : ""} spellcheck="false"
        placeholder="C:\models\my-model"
        onchange={(e) => change("dir", { "provider.model_dir": e.currentTarget.value.trim() || null })} />
    </Row>
  {/if}
{/if}

{#if v("provider.kind") === "openai"}
  <Row label="Adres" help="OpenAI, Groq ya da yerel bir sunucu (…/v1)" note={notes.url}>
    <input class="path" value={text("provider.base_url")} spellcheck="false"
      onchange={(e) => change("url", { "provider.base_url": e.currentTarget.value.trim() })} />
  </Row>
  <Row label="Model" help="Sağlayıcının model adı, örneğin whisper-large-v3" note={notes.modelName}>
    <input value={text("provider.model")} spellcheck="false"
      onchange={(e) => change("modelName", { "provider.model": e.currentTarget.value.trim() })} />
  </Row>
  <Row label="Ad" help="Mesajlarda ve anahtar kaydında görünür" note={notes.name}>
    <input value={text("provider.name")} placeholder="openai" spellcheck="false"
      onchange={(e) => change("name", { "provider.name": e.currentTarget.value.trim() || null })} />
  </Row>
{/if}

{#if v("provider.kind") !== "local" && v("provider.kind") !== null}
  <KeyRow section="provider" stored={snap.keys.provider ?? false} onchange={refresh} />
{/if}

<Row label="Dil" help="Konuştuğun dil; otomatik algılama daha yavaş ve hatalı olabilir" note={notes.lang}>
  <select value={text("transcription.language")}
    onchange={(e) => change("lang", { "transcription.language": e.currentTarget.value || null })}>
    {#each languages as [code, name] (code)}
      <option value={code}>{name}</option>
    {/each}
  </select>
</Row>

<Row label="Buluta yedekle" help="Yerel motor çalışmazsa bulut yedeğine geçmeye izin ver" note={notes.fallback}>
  <input type="checkbox" checked={v("transcription.allow_cloud_fallback") === true}
    onchange={(e) => change("fallback", { "transcription.allow_cloud_fallback": e.currentTarget.checked })} />
</Row>

<style>
  .path { width: 300px; }
</style>
