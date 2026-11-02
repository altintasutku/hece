<script lang="ts">
  import Row from "./Row.svelte";
  import type { Snapshot } from "./api";
  import { t } from "./strings";

  let { snap }: { snap: Snapshot } = $props();
</script>

<Row label="Hece" help={t.daemonText(snap.daemon)}>
  <span class="muted">{t.about.status}</span>
</Row>
<Row label={t.about.configFile} help={snap.config}>
  <span class="muted">{t.about.commentsKept}</span>
</Row>

<p class="quote">{t.about.quote}</p>

<style>
  .muted { color: var(--mute); font-size: 12.5px; }
  .quote { padding: 14px 0 16px; border-top: 1px solid var(--hairline); color: var(--body); }
</style>
