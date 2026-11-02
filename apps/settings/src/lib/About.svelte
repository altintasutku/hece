<script lang="ts">
  import Row from "./Row.svelte";
  import type { Snapshot, Value } from "./api";
  import type { Note } from "./save";
  import { t } from "./strings";

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

<Row label="Hece" help={t.daemonText(snap.daemon)}>
  <span class="muted">{t.about.status}</span>
</Row>
<Row label={t.about.configFile} help={snap.config}>
  <span class="muted">{t.about.commentsKept}</span>
</Row>
<Row label={t.about.updates} help={t.about.updatesHelp} {note}>
  <input type="checkbox" checked={snap.values["updates.check"] !== false}
    onchange={(e) => toggleUpdates(e.currentTarget.checked)} />
</Row>

<p class="quote">{t.about.quote}</p>

<style>
  .muted { color: var(--mute); font-size: 12.5px; }
  .quote { padding: 14px 0 16px; border-top: 1px solid var(--hairline); color: var(--body); }
</style>
