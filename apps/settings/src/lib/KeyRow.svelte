<script lang="ts">
  import { deleteKey, storeKey } from "./api";
  import Row from "./Row.svelte";
  import { savedNote } from "./strings";

  interface Props {
    section: string;
    stored: boolean;
    onchange: () => void;
  }
  let { section, stored, onchange }: Props = $props();

  let editing = $state(false);
  let secret = $state("");
  let note: { text: string; error: boolean } | null = $state(null);

  async function run(action: () => Promise<string>) {
    try {
      note = savedNote(await action());
      editing = false;
      secret = "";
      onchange();
    } catch (e) {
      note = { text: String(e), error: true };
    }
  }
</script>

<Row label="API anahtarı" help="Windows Kimlik Bilgisi Yöneticisi'nde saklanır, burada gösterilmez" {note}>
  {#if editing}
    <input type="password" autocomplete="off" placeholder="Anahtarı yapıştır" bind:value={secret}
      onkeydown={(e) => e.key === "Enter" && secret && run(() => storeKey(section, secret))} />
    <button disabled={!secret} onclick={() => run(() => storeKey(section, secret))}>Kaydet</button>
    <button onclick={() => { editing = false; secret = ""; }}>Vazgeç</button>
  {:else if stored}
    <span class="stored">Kayıtlı ✓</span>
    <button onclick={() => (editing = true)}>Değiştir</button>
    <button onclick={() => run(() => deleteKey(section))}>Sil</button>
  {:else}
    <button onclick={() => (editing = true)}>Anahtar ekle</button>
  {/if}
</Row>

<style>
  .stored { color: var(--accent); font-weight: 600; margin-right: 6px; }
  input { width: 220px; }
</style>
