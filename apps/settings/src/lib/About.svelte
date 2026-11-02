<script lang="ts">
  import Row from "./Row.svelte";
  import type { Snapshot } from "./api";
  import { daemonText } from "./strings";

  let { snap }: { snap: Snapshot } = $props();
</script>

<Row label="Hece" help={daemonText(snap.daemon)}>
  <span class="muted">Durum</span>
</Row>
<Row label="Ayar dosyası" help={snap.config}>
  <span class="muted">Yorumlar korunur</span>
</Row>

<p class="quote">Her ayar hemen kaydedilir; Hece çalışıyorsa yeniden yüklenir.</p>

<style>
  .muted { color: var(--muted); font-size: 12.5px; }
  .quote {
    margin-top: 22px; padding-left: 12px; border-left: 2px solid var(--accent);
    font: italic 400 19px/1.35 var(--serif);
  }
</style>
