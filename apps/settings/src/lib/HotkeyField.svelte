<script lang="ts">
  import { tick } from "svelte";
  import { t } from "./strings";

  const s = t.shortcuts;

  interface Props {
    /** The shortcut as config.toml spells it, e.g. `Ctrl+Shift+Space`. */
    value: string;
    /** The action's name, for the button's accessible label. */
    action: string;
    isDefault: boolean;
    /** A pressed combo in config spelling, or `null` for a key that cannot be a shortcut. */
    onpick: (combo: string | null) => void;
    onreset: () => void;
  }
  let { value, action, isDefault, onpick, onreset }: Props = $props();

  // KeyboardEvent.code -> config name; the same keys hece_core::hotkey accepts. Punctuation is
  // left out: its place depends on the keyboard layout.
  const NAMED: Record<string, string> = {
    Space: "Space", Enter: "Enter", Tab: "Tab", Backspace: "Backspace", Delete: "Delete", Insert: "Insert",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", ArrowUp: "Up", ArrowDown: "Down",
    ArrowLeft: "Left", ArrowRight: "Right", Pause: "Pause",
  };
  const MODIFIER_CODES = /^(Control|Shift|Alt|Meta|OS)(Left|Right)?$/;

  function keyName(code: string): string | null {
    if (code in NAMED) return NAMED[code];
    const m = /^Key([A-Z])$/.exec(code) ?? /^Digit([0-9])$/.exec(code) ?? /^(F(?:[1-9]|1[0-9]|2[0-4]))$/.exec(code);
    return m ? m[1] : null;
  }

  function modifiers(e: KeyboardEvent): string[] {
    return [
      e.ctrlKey && "Ctrl",
      e.altKey && "Alt",
      e.shiftKey && "Shift",
      e.metaKey && "Win",
    ].filter((m): m is string => !!m);
  }

  let capturing = $state(false);
  let held: string[] = $state([]);
  let field: HTMLButtonElement | undefined = $state();

  async function begin() {
    held = [];
    capturing = true;
    // Wait for the capture field to render, then take the keyboard.
    await tick();
    field?.focus();
  }

  function onkeydown(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    const mods = modifiers(e);
    if (e.key === "Escape" && mods.length === 0) {
      capturing = false;
      return;
    }
    held = mods;
    if (MODIFIER_CODES.test(e.code)) return;
    capturing = false;
    const key = keyName(e.code);
    onpick(key ? [...mods, key].join("+") : null);
  }

  function onkeyup(e: KeyboardEvent) {
    held = modifiers(e);
  }
</script>

{#if capturing}
  <button bind:this={field} class="capture" aria-live="polite" {onkeydown} {onkeyup}
    onblur={() => (capturing = false)}>
    {#if held.length}
      {#each held as k (k)}<kbd>{k}</kbd>{/each}<span class="plus">+</span>
    {:else}
      <span class="prompt">{s.press}</span>
    {/if}
    <span class="hint">{s.pressHelp}</span>
  </button>
{:else}
  <span class="keys" aria-label={value}>
    {#each value.split("+") as k, i (i)}<kbd>{k}</kbd>{/each}
  </span>
  <button onclick={begin} aria-label={s.changeLabel(action)}>{s.change}</button>
  {#if !isDefault}
    <button onclick={onreset}>{s.reset}</button>
  {/if}
{/if}

<style>
  .keys { display: inline-flex; gap: 4px; margin-right: 6px; }
  .capture {
    display: inline-flex; align-items: center; gap: 4px; min-width: 260px;
    padding: 5px 10px; border-radius: 8px; cursor: default;
    background: var(--surface-2); border: 1px solid var(--violet);
  }
  .prompt { color: var(--ink); font-weight: 500; }
  .plus { color: var(--mute); }
  .hint { color: var(--mute); font-weight: 400; font-size: 12px; margin-left: auto; padding-left: 12px; }
</style>
