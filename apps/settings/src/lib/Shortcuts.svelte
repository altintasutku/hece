<script lang="ts">
  import Row from "./Row.svelte";
  import type { Snapshot, Value } from "./api";
  import type { Note } from "./save";
  import { t } from "./strings";

  const s = t.shortcuts;

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

<Row label={s.record} help={s.recordHelp}>
  <kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>Space</kbd>
</Row>

<Row label={s.cancel} help={s.cancelHelp}>
  <kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>Backspace</kbd>
</Row>

<Row label={s.mode} help={s.modeHelp} note={notes.mode}>
  <select value={snap.values["recording.mode"] ?? "hold"}
    onchange={(e) => change("mode", { "recording.mode": e.currentTarget.value })}>
    <option value="hold">{s.hold}</option>
    <option value="toggle">{s.toggle}</option>
  </select>
</Row>

<Row label={s.microphone} help={s.microphoneHelp} note={notes.device}>
  <select value={snap.values["recording.device"] ?? ""}
    onchange={(e) => change("device", { "recording.device": e.currentTarget.value || null })}>
    <option value="">{s.windowsDefault}</option>
    {#each snap.devices as device (device)}
      <option value={device}>{device}</option>
    {/each}
  </select>
</Row>

<Row label={s.overlay} help={s.overlayHelp} note={notes.overlay}>
  <input type="checkbox" checked={snap.values["ui.overlay"] !== false}
    onchange={(e) => change("overlay", { "ui.overlay": e.currentTarget.checked })} />
</Row>

<Row label={s.sounds} help={s.soundsHelp} note={notes.sounds}>
  <input type="checkbox" checked={snap.values["feedback.sounds"] !== false}
    onchange={(e) => change("sounds", { "feedback.sounds": e.currentTarget.checked })} />
</Row>

<Row label={s.maxLength} help={s.maxLengthHelp} note={notes.max}>
  <select value={minutes}
    onchange={(e) => change("max", { "recording.max_seconds": Number(e.currentTarget.value) * 60 })}>
    {#each [1, 2, 5, 10, 11] as m (m)}
      <option value={m}>{s.minutes(m)}</option>
    {/each}
  </select>
</Row>
