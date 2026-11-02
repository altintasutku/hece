<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    label: string;
    help: string;
    /** Shown instead of the helper line after a save, then cleared. */
    note?: { text: string; error: boolean } | null;
    children: Snippet;
  }
  let { label, help, note = null, children }: Props = $props();
</script>

<div class="row">
  <div>
    <div class="label">{label}</div>
    {#if note}
      <div class="help" class:ok={!note.error} class:error={note.error} role="status">{note.text}</div>
    {:else}
      <div class="help">{help}</div>
    {/if}
  </div>
  <div class="control">{@render children()}</div>
</div>

<style>
  .row {
    display: grid; grid-template-columns: 1fr auto; align-items: center; gap: 16px;
    padding: 14px 0; border-top: 1px solid var(--hairline);
  }
  .row:first-child { border-top: 0; }
  .label { font-weight: 600; color: var(--ink); }
  .help { color: var(--mute); font-size: 12.5px; margin-top: 2px; }
  .help.ok { color: var(--violet); font-weight: 600; }
  .help.error { color: var(--danger); }
  .control { display: flex; gap: 6px; align-items: center; }
</style>
