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
  const keyHelp: Record<string, string> = {
    groq: "console.groq.com/keys adresinden al; Kimlik Bilgisi Yöneticisi'nde saklanır",
    openai: "platform.openai.com/api-keys adresinden al; Kimlik Bilgisi Yöneticisi'nde saklanır",
    deepgram: "console.deepgram.com adresinden al; Kimlik Bilgisi Yöneticisi'nde saklanır",
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

<Row label="Motor" help="Yerel: ekran kartında, ses bilgisayardan çıkmaz. Bulut: anahtar gerekir, ses sağlayıcıya gider." note={notes.kind}>
  <select value={choice} onchange={(e) => choose(e.currentTarget.value)}>
    <option value="local">Yerel (GPU)</option>
    <option value="groq">Groq (bulut)</option>
    <option value="openai">OpenAI (bulut)</option>
    <option value="deepgram">Deepgram (bulut)</option>
    <option value="custom">OpenAI uyumlu sunucu</option>
  </select>
</Row>

{#if (v("provider.kind") ?? "local") === "local"}
  <Row label="Model" help={modelHelp} note={notes.model}>
    <select value={chosen} disabled={percent !== null}
      onchange={(e) => pick(e.currentTarget.value)}>
      {#each snap.models as m (m.id)}
        <option value={m.id}>{m.name}{m.installed ? "" : ` · ${(m.size_mb / 1000).toFixed(1)} GB`}</option>
      {/each}
      {#if chosen === "custom"}
        <option value="custom">Kendi klasörün</option>
      {/if}
    </select>
    {#if percent !== null}
      <button onclick={cancelDownload}>Vazgeç</button>
    {:else if selected && !selected.installed}
      <button onclick={() => download(selected.id)}>İndir</button>
    {/if}
  </Row>
{/if}

{#if choice === "groq"}
  <Row label="Model" help="large-v3 daha doğru, turbo daha hızlı ve ucuz" note={notes.modelName}>
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
    <Row label="Adres" help="Sunucunun …/v1 adresi (speaches, whisper.cpp, LocalAI)" note={notes.url}>
      <input class="path" value={text("provider.base_url")} spellcheck="false"
        onchange={(e) => change("url", { "provider.base_url": e.currentTarget.value.trim() })} />
    </Row>
  {/if}
  <Row label="Model" help="Sağlayıcının model adı, örneğin whisper-1" note={notes.modelName}>
    <input value={text("provider.model")} spellcheck="false"
      onchange={(e) => change("modelName", modelChanges(e.currentTarget.value.trim()))} />
  </Row>
  {#if choice === "custom"}
    <Row label="Ad" help="Mesajlarda ve anahtar kaydında görünür" note={notes.name}>
      <input value={text("provider.name")} placeholder="openai" spellcheck="false"
        onchange={async (e) => { await change("name", { "provider.name": e.currentTarget.value.trim() || null }); refresh(); }} />
    </Row>
  {/if}
{/if}

{#if choice !== "local"}
  <KeyRow section="provider" stored={snap.keys.provider ?? false} onchange={refresh}
    help={keyHelp[choice]} />
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
