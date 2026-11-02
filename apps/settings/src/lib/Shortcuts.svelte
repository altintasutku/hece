<script lang="ts">
  import Row from "./Row.svelte";
  import type { Snapshot, Value } from "./api";
  import type { Note } from "./save";

  interface Props {
    snap: Snapshot;
    save: (changes: Record<string, Value>) => Promise<Note>;
  }
  let { snap, save }: Props = $props();

  let notes: Record<string, Note | null> = $state({});
  async function change(row: string, changes: Record<string, Value>) {
    notes[row] = await save(changes);
    setTimeout(() => (notes[row] = null), 3000);
  }
  const minutes = $derived(Math.round(((snap.values["recording.max_seconds"] as number | null) ?? 600) / 60));
</script>

<Row label="Kaydet" help="Windows'un kullanmadığı bir kısayol; şimdilik değiştirilemez">
  <span class="chip">Ctrl</span><span class="chip">Shift</span><span class="chip">Space</span>
</Row>

<Row label="Vazgeç" help="Süren kaydı ya da son dikteyi iptal eder">
  <span class="chip">Ctrl</span><span class="chip">Shift</span><span class="chip">Backspace</span>
</Row>

<Row label="Kayıt biçimi" help="Basılı tut: bırakınca biter. Bas-bas: ikinci basışta biter." note={notes.mode}>
  <select value={snap.values["recording.mode"] ?? "hold"}
    onchange={(e) => change("mode", { "recording.mode": e.currentTarget.value })}>
    <option value="hold">Basılı tut</option>
    <option value="toggle">Bas-bas</option>
  </select>
</Row>

<Row label="Mikrofon" help="Varsayılan, Windows'un seçtiği giriş aygıtıdır" note={notes.device}>
  <select value={snap.values["recording.device"] ?? ""}
    onchange={(e) => change("device", { "recording.device": e.currentTarget.value || null })}>
    <option value="">Windows varsayılanı</option>
    {#each snap.devices as device (device)}
      <option value={device}>{device}</option>
    {/each}
  </select>
</Row>

<Row label="Kayıt göstergesi" help="Kayıt sırasında ekranın sağ üstünde sese tepki veren küre" note={notes.overlay}>
  <input type="checkbox" checked={snap.values["ui.overlay"] !== false}
    onchange={(e) => change("overlay", { "ui.overlay": e.currentTarget.checked })} />
</Row>

<Row label="Sesli uyarı" help="Kayıt başlarken, biterken ve iptal edilince kısa, yumuşak bir ses" note={notes.sounds}>
  <input type="checkbox" checked={snap.values["feedback.sounds"] !== false}
    onchange={(e) => change("sounds", { "feedback.sounds": e.currentTarget.checked })} />
</Row>

<Row label="En uzun kayıt" help="Bu süreden sonra kayıt kendiliğinden biter (en çok 11 dk)" note={notes.max}>
  <select value={minutes}
    onchange={(e) => change("max", { "recording.max_seconds": Number(e.currentTarget.value) * 60 })}>
    {#each [1, 2, 5, 10, 11] as m (m)}
      <option value={m}>{m} dakika</option>
    {/each}
  </select>
</Row>

<style>
  .chip { font: 500 12px/1 var(--sans); border: 1px solid var(--ink); padding: 3px 7px; }
</style>
