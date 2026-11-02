# hece settings: DESIGN draft

Status: **draft, direction not chosen**. `direction-board.html` shows the same screen part (Provider + shortcut) in
three directions. Once one is chosen, its tokens below move to the root `DESIGN.md` and the other two are deleted.

## Shared rules (all directions)

- **Window:** one settings window, about 900x640, resizable down to 720x520. Left navigation (or pills in B) and one
  content column of at most 640 px. Opens only when launched from the tray "Settings" item; closing it frees it.
- **Sections:** Sağlayıcı, Kısayollar, Sözlük, Temizleme, Yapıştırma, Geçmiş, API, Hakkında. Each section is a title,
  one sentence, then rows.
- **Row:** label (600), one helper line (muted), control on the right. One row = one config key. No nested cards.
- **Saving:** changes write `config.toml` right away (no Save button). A row that needs a daemon restart says so in
  its helper line.
- **Secrets:** keys are never shown. A row reads "Kayıtlı ✓" or offers "Anahtar ekle", which opens a masked input;
  the key goes to the Credential Manager.
- **Shortcut chips:** one chip per key, the same chip style everywhere (tray tooltip, README screenshots).
- **Text:** Turkish first, English UI later via one strings file. Sentence case, no all-caps labels.
- **Accessibility:** body text at least 13 px, contrast at least 4.5:1 for text, visible focus ring, full keyboard
  navigation. The Windows high-contrast theme falls back to system colors.
- **The hece signature:** each direction carries one syllable mark (A: dot, B: beat bar, C: red hyphen). Use it only
  in section titles and in the "last dictation" sample, never as decoration elsewhere.
- **Avoid:** dark graphite with a single neon accent, pastel cards, Inter/Geist, grain, mono uppercase labels.

## A · Dizgi (reference: WIRED)

| Token | Value |
| --- | --- |
| Display | Instrument Serif 400, 44/1, -0.5px |
| UI / body | IBM Plex Sans 400/500/600, 13–14 px |
| ink / muted / hairline / canvas | `#111111` / `#757575` / `#e0e0e0` / `#fbfaf7` |
| accent (links, syllable dot, success) | `#057dbc` |
| Radius | 0 (controls), 10 (window only) |
| Spacing | row padding 14 px, section gap 22 px, content padding 26/30 |
| Controls | 1 px ink border, white fill; hover inverts to ink fill |

## B · Ritim (reference: Mastercard)

| Token | Value |
| --- | --- |
| Display | Unbounded 700, 30/1.05, -0.5px |
| UI / body | Hanken Grotesk 400/500/600, 13–15 px |
| ink / muted / canvas / surface | `#141413` / `#77736f` / `#f3f0ee` / `#ffffff` |
| accent (primary, beat) | `#cf4a1a` (text on it: white, 4.6:1) |
| success | `#2f7a3b` |
| Radius | 28 (cards), 999 (pills, chips, controls) |
| Spacing | card padding 18/20, card gap 12 px |
| Beat bar | 12 px pills, accent and ink alternating; live while recording (segment per detected syllable burst) |

## C · Defter (reference: Notion structure + school syllable notebook)

| Token | Value |
| --- | --- |
| Display | Literata 700, 32/1, ink-blue |
| UI / body | Atkinson Hyperlegible 400/700, 13–14 px |
| ink / ink-blue / muted / paper | `#1d1b17` / `#1f3a93` / `#6b665a` / `#fffdf6` |
| margin red (hyphen, margin line) | `#e0605a` (decoration only, never text below 18 px) |
| ruled lines | `#dfe7f2`, every 28 px |
| Radius | 8 (rows), 6 (controls), 5 (key chips with 2 px bottom border) |
| Spacing | row padding 12/14, row gap 10 px, content left 46 px (after the margin line) |

## Open questions (for Utku)

1. Which direction (A, B or C), or a mix (for example A's layout with C's syllable hyphen)?
2. How the app talks to the daemon. Proposal: the app writes `config.toml` and sends `reload` over the existing
   control pipe (one new command). Status and history are read from the files the daemon already writes.
3. Tauri 2 brings a frontend toolchain into the repo (node/bun + a small framework). Proposal: `apps/settings` with
   Vite + plain TypeScript or Svelte, versions pinned in `mise.toml`.
