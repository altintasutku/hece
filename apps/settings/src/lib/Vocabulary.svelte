<script lang="ts">
  import Row from "./Row.svelte";
  import {
    importVocabulary, saveVocabulary, useVocabularyFile, vocabulary,
    type VocabularyCheck, type VocabularyView,
  } from "./api";
  import type { Note } from "./save";
  import { t } from "./strings";

  const s = t.vocabulary;

  let view: VocabularyView | null = $state(null);
  let failure: string | null = $state(null);
  let entries: string[] = $state([]);
  let check: VocabularyCheck = $state({ statuses: [], terms: 0, fit: 0 });
  let draft = $state("");
  let otherPath = $state("");
  let choosing = $state(false);
  let notes: Record<string, Note | null> = $state({});

  async function load() {
    try {
      view = await vocabulary();
      entries = [...view.entries];
      check = view.check;
      failure = null;
    } catch (e) {
      failure = s.failed(String(e));
    }
  }
  load();

  function show(row: string, note: Note) {
    notes[row] = note;
    setTimeout(() => (notes[row] = null), 3000);
  }

  /** Saves the whole list; the daemon reloads it. */
  async function save(next: string[]) {
    entries = next;
    try {
      const [result, answer] = await saveVocabulary(next);
      check = result;
      show("list", t.savedNote(answer));
    } catch (e) {
      show("list", { text: s.failed(String(e)), error: true });
    }
  }

  function add(e: SubmitEvent) {
    e.preventDefault();
    const term = draft.trim();
    if (!term) return;
    draft = "";
    save([...entries, term]);
  }

  function move(i: number, by: number) {
    const next = [...entries];
    [next[i], next[i + by]] = [next[i + by], next[i]];
    save(next);
  }

  function edit(i: number, value: string) {
    if (value === entries[i]) return;
    const next = [...entries];
    if (value.trim()) next[i] = value;
    else next.splice(i, 1);
    save(next);
  }

  /** Index among usable terms, to tell which ones the length limit leaves out. */
  const rank = $derived.by(() => {
    let n = 0;
    return check.statuses.map((st) => (st === "ok" ? n++ : -1));
  });

  function hint(i: number): { text: string; error: boolean } | null {
    const st = check.statuses[i];
    if (st === "duplicate") return { text: s.duplicate, error: true };
    if (st === "invalid") return { text: s.invalid, error: true };
    if (rank[i] >= check.fit) return { text: s.overBudget, error: false };
    return null;
  }

  async function useOther() {
    try {
      const answer = await useVocabularyFile(otherPath.trim());
      choosing = false;
      await load();
      show("file", t.savedNote(answer));
    } catch (e) {
      show("file", { text: s.failed(String(e)), error: true });
    }
  }

  async function importOther() {
    try {
      const terms = await importVocabulary(otherPath.trim());
      choosing = false;
      await save([...entries, ...terms]);
      show("file", { text: s.imported(terms.length), error: false });
    } catch (e) {
      show("file", { text: s.failed(String(e)), error: true });
    }
  }

  async function backToDefault() {
    try {
      const answer = await useVocabularyFile(null);
      await load();
      show("file", t.savedNote(answer));
    } catch (e) {
      show("file", { text: s.failed(String(e)), error: true });
    }
  }
</script>

{#if failure}
  <p class="failed" role="alert">{failure}</p>
{:else if view}
  <Row label={s.add} help={s.addHelp}>
    <form class="add" onsubmit={add}>
      <input bind:value={draft} placeholder={s.placeholder} aria-label={s.add} maxlength="80" />
      <button class="primary" type="submit" disabled={!draft.trim()}>{s.addButton}</button>
    </form>
  </Row>

  <div class="list">
    <div class="head">
      <span class="label">{s.terms}</span>
      {#if notes.list}
        <span class="help" class:ok={!notes.list.error} class:error={notes.list.error} role="status">{notes.list.text}</span>
      {:else}
        <span class="help">{s.count(check.terms, check.fit)}</span>
      {/if}
    </div>
    {#if entries.length === 0}
      <p class="empty">{s.empty}</p>
    {:else}
      <ol>
        {#each entries as entry, i (i)}
          {@const h = hint(i)}
          <li>
            <span class="n">{i + 1}</span>
            <div class="term">
              <input value={entry} aria-label={s.termLabel(i + 1)} class:skipped={h?.error}
                onchange={(e) => edit(i, e.currentTarget.value)} />
              {#if h}<span class="hint" class:error={h.error}>{h.text}</span>{/if}
            </div>
            <button class="icon" aria-label={s.moveUp} title={s.moveUp} disabled={i === 0} onclick={() => move(i, -1)}>↑</button>
            <button class="icon" aria-label={s.moveDown} title={s.moveDown} disabled={i === entries.length - 1}
              onclick={() => move(i, 1)}>↓</button>
            <button class="icon" aria-label={s.remove} title={s.remove}
              onclick={() => save(entries.filter((_, j) => j !== i))}>×</button>
          </li>
        {/each}
      </ol>
    {/if}
  </div>

  <Row label={s.file} help={s.fileHelp(view.path)} note={notes.file}>
    {#if choosing}
      <button onclick={() => (choosing = false)}>{s.cancel}</button>
    {:else}
      {#if view.custom}<button onclick={backToDefault}>{s.backToDefault}</button>{/if}
      <button onclick={() => (choosing = true)}>{s.other}</button>
    {/if}
  </Row>
  {#if choosing}
    <div class="other">
      <input bind:value={otherPath} placeholder={s.pathPlaceholder} aria-label={s.other} />
      <button class="primary" disabled={!otherPath.trim()} onclick={useOther}>{s.use}</button>
      <button disabled={!otherPath.trim()} onclick={importOther}>{s.import}</button>
    </div>
  {/if}
{/if}

<style>
  .failed { color: var(--danger); padding: 14px 0; }
  .add { display: flex; gap: 6px; }
  .other { display: flex; gap: 6px; padding: 0 0 14px; }
  .other input { flex: 1; }
  .add input { width: 200px; }
  .list { padding: 14px 0; border-top: 1px solid var(--hairline); }
  .head { display: flex; justify-content: space-between; align-items: baseline; gap: 16px; margin-bottom: 10px; }
  .label { font-weight: 600; color: var(--ink); }
  .help { color: var(--mute); font-size: 12.5px; }
  .help.ok { color: var(--violet); font-weight: 600; }
  .help.error { color: var(--danger); }
  .empty { color: var(--mute); font-size: 12.5px; }
  ol { list-style: none; padding: 0; display: grid; gap: 6px; }
  li { display: grid; grid-template-columns: 24px 1fr auto auto auto; gap: 6px; align-items: start; }
  .n { color: var(--mute); font-size: 12.5px; text-align: right; padding-top: 8px; font-variant-numeric: tabular-nums; }
  .term { display: grid; gap: 3px; }
  .term input { width: 100%; }
  .term input.skipped { border-color: var(--danger); }
  .hint { color: var(--mute); font-size: 12px; }
  .hint.error { color: var(--danger); }
  .icon { width: 32px; height: 32px; padding: 0; font: 600 14px/1 var(--sans); }
</style>
