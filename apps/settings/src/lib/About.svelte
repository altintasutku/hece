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
  .muted { color: var(--muted); font-size: 12.5px; }
  .quote {
    margin-top: 22px; padding-left: 12px; border-left: 2px solid var(--accent);
    font: italic 400 19px/1.35 var(--serif);
  }
</style>
