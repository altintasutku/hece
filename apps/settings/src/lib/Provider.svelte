<script lang="ts">
  import KeyRow from "./KeyRow.svelte";
  import Row from "./Row.svelte";
  import type { Snapshot, Value } from "./api";
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
</script>

<Row label="Motor" help="Yerel: Türkçe Whisper, ekran kartında. Bulut: anahtar gerekir." note={notes.kind}>
  <select value={v("provider.kind") ?? "local"} onchange={(e) => change("kind", kindChanges(e.currentTarget.value))}>
    <option value="local">Yerel (GPU)</option>
    <option value="openai">OpenAI uyumlu</option>
    <option value="deepgram">Deepgram</option>
  </select>
</Row>

<Row label="Yerel model klasörü" help="prepare_model.py çıktısı; içinde model.bin olmalı" note={notes.model}>
  <input class="path" value={text("provider.model_dir")} spellcheck="false"
    onchange={(e) => change("model", { "provider.model_dir": e.currentTarget.value.trim() || null })} />
</Row>

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
