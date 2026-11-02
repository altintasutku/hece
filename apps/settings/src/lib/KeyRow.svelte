<script lang="ts">
  import { deleteKey, storeKey } from "./api";
  import Row from "./Row.svelte";
  import { t } from "./strings";

  interface Props {
    section: string;
    stored: boolean;
    onchange: () => void;
    /** Where to get the key; the default only says where it is kept. */
    help?: string;
  }
  let { section, stored, onchange, help = t.key.help }: Props = $props();

  let editing = $state(false);
  let secret = $state("");
  let note: { text: string; error: boolean } | null = $state(null);

  async function run(action: () => Promise<string>) {
    try {
      note = t.savedNote(await action());
      editing = false;
      secret = "";
      onchange();
    } catch (e) {
      note = { text: String(e), error: true };
    }
  }
</script>

<Row label={t.key.label} {help} {note}>
  {#if editing}
    <input type="password" autocomplete="off" placeholder={t.key.placeholder} bind:value={secret}
      onkeydown={(e) => e.key === "Enter" && secret && run(() => storeKey(section, secret))} />
    <button disabled={!secret} onclick={() => run(() => storeKey(section, secret))}>{t.key.save}</button>
    <button onclick={() => { editing = false; secret = ""; }}>{t.key.cancel}</button>
  {:else if stored}
    <span class="stored">{t.key.stored}</span>
    <button onclick={() => (editing = true)}>{t.key.change}</button>
    <button onclick={() => run(() => deleteKey(section))}>{t.key.remove}</button>
  {:else}
    <button onclick={() => (editing = true)}>{t.key.add}</button>
  {/if}
</Row>

<style>
  .stored { color: var(--accent); font-weight: 600; margin-right: 6px; }
  input { width: 220px; }
</style>
