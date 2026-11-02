<script lang="ts">
  import Row from "./Row.svelte";
  import type { Snapshot, Value } from "./api";
  import type { Note } from "./save";
  import { daemonText, updates } from "./strings";

  interface Props {
    snap: Snapshot;
    save: (changes: Record<string, Value>) => Promise<Note>;
  }
  let { snap, save }: Props = $props();

  let note: Note | null = $state(null);
  async function toggleUpdates(check: boolean) {
    note = await save({ "updates.check": check });
    setTimeout(() => (note = null), 3000);
  }
</script>

<Row label="Hece" help={daemonText(snap.daemon)}>
  <span class="muted">Durum</span>
</Row>
<Row label="Ayar dosyası" help={snap.config}>
  <span class="muted">Yorumlar korunur</span>
</Row>
<Row label={updates.label} help={updates.help} {note}>
  <input type="checkbox" checked={snap.values["updates.check"] !== false}
    onchange={(e) => toggleUpdates(e.currentTarget.checked)} />
</Row>

<p class="quote">Her ayar hemen kaydedilir; Hece çalışıyorsa yeniden yüklenir.</p>

<style>
  .muted { color: var(--muted); font-size: 12.5px; }
  .quote {
    margin-top: 22px; padding-left: 12px; border-left: 2px solid var(--accent);
    font: italic 400 19px/1.35 var(--serif);
  }
</style>
