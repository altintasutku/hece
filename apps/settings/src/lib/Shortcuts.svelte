<script lang="ts">
  import HotkeyField from "./HotkeyField.svelte";
  import Row from "./Row.svelte";
  import { setValues, type Snapshot, type Value } from "./api";
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

  const HOTKEYS = {
    record: { field: "recording.hotkey", fallback: "Ctrl+Shift+Space" },
    cancel: { field: "recording.cancel_hotkey", fallback: "Ctrl+Shift+Backspace" },
  } as const;
  const hotkey = (row: keyof typeof HOTKEYS) =>
    (snap.values[HOTKEYS[row].field] as string | null) ?? HOTKEYS[row].fallback;

  function show(row: string, note: Note) {
    notes[row] = note;
    setTimeout(() => (notes[row] = null), 4000);
  }

  /** Saves a shortcut (`null` = default). If another app owns it, the daemon keeps the old pair
   *  and this puts the old value back in config.toml so the next start does not fail. */
  async function setHotkey(row: keyof typeof HOTKEYS, combo: string | null) {
    const { field, fallback } = HOTKEYS[row];
    const before = snap.values[field] ?? null;
    if ((combo ?? fallback) === hotkey(row)) return;
    try {
      const answer = await setValues({ [field]: combo });
      if (answer === "hotkey_in_use") {
        await setValues({ [field]: before });
        show(row, { text: s.inUse(combo ?? fallback), error: true });
        return;
      }
      snap.values[field] = combo;
      show(row, t.savedNote(answer));
    } catch (e) {
      show(row, { text: s.hotkeyError(String(e)), error: true });
    }
  }

  function pick(row: keyof typeof HOTKEYS, combo: string | null) {
    if (combo === null) show(row, { text: s.unsupportedKey, error: true });
    else setHotkey(row, combo);
  }
</script>

{#each [["record", s.record, s.recordHelp], ["cancel", s.cancel, s.cancelHelp]] as const as [row, label, help] (row)}
  <Row {label} {help} note={notes[row]}>
    <HotkeyField value={hotkey(row)} action={label} isDefault={hotkey(row) === HOTKEYS[row].fallback}
      onpick={(combo) => pick(row, combo)} onreset={() => setHotkey(row, null)} />
  </Row>
{/each}

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

<Row label={s.maxLength} help={s.maxLengthHelp} note={notes.max}>
  <select value={minutes}
    onchange={(e) => change("max", { "recording.max_seconds": Number(e.currentTarget.value) * 60 })}>
    {#each [1, 2, 5, 10, 11] as m (m)}
      <option value={m}>{s.minutes(m)}</option>
    {/each}
  </select>
</Row>
