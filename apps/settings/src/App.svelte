<script lang="ts">
  import About from "./lib/About.svelte";
  import Provider from "./lib/Provider.svelte";
  import Shortcuts from "./lib/Shortcuts.svelte";
  import Vocabulary from "./lib/Vocabulary.svelte";
  import { snapshot, startDaemon, type Snapshot, type Value } from "./lib/api";
  import { save } from "./lib/save";
  import { t, type Section } from "./lib/strings";

  const sections = t.sections;

  let snap: Snapshot | null = $state(null);
  let failure: string | null = $state(null);
  let current: Section = $state(sections.find((s) => `#${s.id}` === location.hash) ?? sections[0]);

  async function refresh() {
    try {
      snap = await snapshot();
      failure = null;
    } catch (e) {
      failure = String(e);
    }
  }
  refresh();

  async function saveTo(changes: Record<string, Value>) {
    return snap ? save(snap, changes) : { text: t.notLoaded, error: true };
  }

  let starting = $state(false);
  async function start() {
    starting = true;
    try {
      await startDaemon();
      // Give the daemon a moment to open its pipe before asking how it is.
      await new Promise((r) => setTimeout(r, 2500));
      await refresh();
    } catch (e) {
      failure = String(e);
    } finally {
      starting = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    // A shortcut capture field took this key.
    if (e.defaultPrevented) return;
    if (e.key === "Escape" || (e.ctrlKey && e.key.toLowerCase() === "w")) window.close();
  }
</script>

<svelte:window {onkeydown} />

<div class="layout">
  <aside>
    <p class="wordmark" aria-hidden="true">He<span class="dot">·</span>ce</p>
    <nav aria-label={t.navLabel}>
      {#each sections as s (s.id)}
        <a href={`#${s.id}`} class:on={s.id === current.id} aria-current={s.id === current.id ? "page" : undefined}
          onclick={(e) => { e.preventDefault(); current = s; }}>{s.nav}</a>
      {/each}
    </nav>
  </aside>
  <select class="nav-select" aria-label={t.navSelectLabel} value={current.id}
    onchange={(e) => (current = sections.find((s) => s.id === e.currentTarget.value) ?? current)}>
    {#each sections as s (s.id)}
      <option value={s.id}>{s.nav}</option>
    {/each}
  </select>

  <main>
    <h1 aria-label={current.nav}>
      {#each current.syllables as part, i (i)}{#if i > 0}<span class="dot">·</span>{/if}{part}{/each}
    </h1>
    <p class="lede">{current.lede}</p>

    {#if snap?.daemon === "not_running"}
      <div class="banner" role="status">
        <span>{t.notRunning}</span>
        <button class="primary" onclick={start} disabled={starting}>{starting ? t.starting : t.start}</button>
      </div>
    {/if}

    {#if failure}
      <p class="error" role="alert">{t.readFailed(failure)}</p>
    {:else if snap}
      <div class="card">
        {#if current.id === "provider"}
          <Provider {snap} save={saveTo} {refresh} />
        {:else if current.id === "shortcuts"}
          <Shortcuts {snap} save={saveTo} />
        {:else if current.id === "vocabulary"}
          <Vocabulary />
        {:else}
          <About {snap} />
        {/if}
      </div>
    {/if}
  </main>
</div>

<style>
  .layout { display: grid; grid-template-columns: 180px 1fr; min-height: 100vh; }
  aside { background: var(--surface); border-right: 1px solid var(--hairline); padding: 22px 12px; }
  .wordmark { font: 400 28px/1 var(--serif); color: #fff; padding: 0 10px 22px; }
  nav a {
    display: block; padding: 8px 10px; margin-bottom: 2px; border-radius: 8px;
    color: var(--mute); text-decoration: none; font-weight: 500;
  }
  nav a:hover { color: var(--ink); }
  nav a.on { color: var(--ink); background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--hairline); }
  .nav-select { display: none; }
  main { padding: 30px 32px; max-width: 720px; }
  h1 { font: 600 32px/1.1 var(--sans); letter-spacing: -0.02em; color: var(--ink); }
  .dot { color: var(--violet); }
  .lede { color: var(--mute); margin: 8px 0 22px; }
  .error { color: var(--danger); }
  .card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius); padding: 0 18px; }
  .banner {
    display: flex; justify-content: space-between; align-items: center; gap: 16px;
    padding: 12px 12px 12px 16px; margin-bottom: 16px; color: var(--ink);
    background: var(--surface-2); border: 1px solid var(--hairline); border-radius: var(--radius);
  }

  @media (max-width: 760px) {
    .layout { grid-template-columns: 1fr; align-content: start; }
    aside { display: none; }
    .nav-select { display: block; margin: 16px 32px 0; justify-self: start; }
  }
</style>
