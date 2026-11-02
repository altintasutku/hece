<script lang="ts">
  import About from "./lib/About.svelte";
  import Provider from "./lib/Provider.svelte";
  import Shortcuts from "./lib/Shortcuts.svelte";
  import { snapshot, type Snapshot, type Value } from "./lib/api";
  import { save } from "./lib/save";
  import { sections, type Section } from "./lib/strings";

  let snap: Snapshot | null = $state(null);
  let failure: string | null = $state(null);
  let current: Section = $state(sections[0]);

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
    return snap ? save(snap, changes) : { text: "Ayarlar henüz okunmadı", error: true };
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" || (e.ctrlKey && e.key.toLowerCase() === "w")) window.close();
  }
</script>

<svelte:window {onkeydown} />

<div class="layout">
  <nav aria-label="Bölümler">
    {#each sections as s (s.id)}
      <a href={`#${s.id}`} class:on={s.id === current.id} aria-current={s.id === current.id ? "page" : undefined}
        onclick={(e) => { e.preventDefault(); current = s; }}>{s.nav}</a>
    {/each}
  </nav>
  <select class="nav-select" aria-label="Bölüm" value={current.id}
    onchange={(e) => (current = sections.find((s) => s.id === e.currentTarget.value) ?? current)}>
    {#each sections as s (s.id)}
      <option value={s.id}>{s.nav}</option>
    {/each}
  </select>

  <main>
    <h1>
      {#each current.syllables as part, i (i)}{#if i > 0}<span class="dot">·</span>{/if}{part}{/each}
    </h1>
    <p class="lede">{current.lede}</p>

    {#if failure}
      <p class="error" role="alert">Ayarlar okunamadı: {failure}</p>
    {:else if snap}
      {#if current.id === "provider"}
        <Provider {snap} save={saveTo} {refresh} />
      {:else if current.id === "shortcuts"}
        <Shortcuts {snap} save={saveTo} />
      {:else}
        <About {snap} />
      {/if}
    {/if}
  </main>
</div>

<style>
  .layout { display: grid; grid-template-columns: 150px 1fr; min-height: 100vh; }
  nav { border-right: 1px solid var(--hairline); padding: 26px 0 0 18px; }
  nav a { display: block; padding: 7px 0; color: var(--muted); text-decoration: none; }
  nav a.on { color: var(--ink); font-weight: 600; box-shadow: inset -2px 0 0 var(--ink); }
  .nav-select { display: none; }
  main { padding: 26px 30px; max-width: 700px; }
  h1 { font: 400 44px/1 var(--serif); letter-spacing: -0.5px; }
  .dot { color: var(--accent); }
  .lede { color: var(--muted); margin: 8px 0 22px; }
  .error { color: var(--danger); }

  @media (max-width: 760px) {
    .layout { grid-template-columns: 1fr; }
    nav { display: none; }
    .nav-select { display: block; margin: 16px 30px 0; justify-self: start; }
  }
</style>
