# hece design: Gece

Hece's visual language, for the website (`site/`) and the settings app (`apps/settings`). Direction **Gece** was
chosen for the website on 2026-11-03 from three candidates (Gece, Sahne, Sıcak; #57) and applied to the settings
app by decision of 2026-11-03 to match the site (#59), so the app people open after downloading looks like the
page they downloaded it from. Every screen follows this file, and new decisions are recorded here.

Reference: Raycast (dark canvas, surface ladder instead of shadows, white pill CTA, keycaps). Character comes from
the product: the recording orb is the hero, so the violet accent is the orb's colour, and *hece* means syllable,
so section titles keep the syllable dot.

History: the settings app first followed **Dizgi** (chosen 2026-11-02, PR #37): a light, typeset page with IBM
Plex Sans, Instrument Serif titles, square controls and hairlines only. Gece replaced it; the syllable-dot title
and the one-row-per-config-key layout carry over.

## Tokens

| Token | Value | Use |
| --- | --- | --- |
| `--canvas` | `#07080a` | page and window background |
| `--surface` / `--surface-2` / `--card` | `#0d0d0d` / `#101111` / `#121212` | depth ladder (no shadows): app sidebar / fields, banner, active nav / cards |
| `--hairline` | `#242728` | card borders, row rules, sidebar divider |
| `--hairline-strong` | `#3a3d40` (app) / `rgba(255,255,255,0.16)` (site) | field and ghost-button borders |
| `--ink` / `--body` / `--mute` | `#f4f4f6` / `#cdcdcd` / `#9c9c9d` | headings, labels / body / helper text (6.7:1 on `--card`) |
| `--violet` | `#9682ff` | syllable dots, wordmark dot, "saved" state, checked boxes, eyebrows (6.2:1 on `--card`) |
| `--focus` (site `--blue`) | `#57c1ff` | focus outline |
| `--danger` (site `--red`) | `#ff6161` | errors only (6.3:1 on `--card`) |
| `--green`, `--yellow` | `#59d499`, `#ffc533` | site only, as small dots and tags in feature illustrations |

One accent (violet). Text contrast is at least 4.5:1; control boundaries that carry meaning on their own (the
checkbox) at least 3:1 (`#6b6e72`).

## Type

- **Schibsted Grotesk** (variable, 400-700) for everything. Headings 600 with -0.02em tracking. App: section
  title 32 px, body and labels 14 px (labels 600), helper text 12.5 px, controls 13 px.
- **Instrument Serif** only for the `He·ce` wordmark (site header, app sidebar).
- Both are bundled (SIL OFL): `site/fonts` for the site, `apps/settings/src/fonts` plus `@fontsource/instrument-serif`
  for the app. No network font requests. No Inter or Geist. No uppercase or mono labels.

## Space and shape

- 4 px grid. App: content padding 30 px top, 32 px sides; rows 14 px top and bottom; 22 px between a section's
  lede and its card. Sidebar 180 px on `--surface` with a hairline on its right. Content column at most 720 px.
- Cards: 12 px radius, 1 px `--hairline`, `--card` fill. Fields 8 px radius. Buttons are pills.
- No shadows on chrome. Keycaps are the only gradient besides the orb and the site's hero glow.

## Components

- **Section title (app):** the section name split into syllables with violet middle dots (`Pro·vid·er`,
  `Sağ·la·yı·cı`), `aria-label` set to the plain name, then a one-sentence lede in `--mute`. The syllable split is
  precomputed per language in the strings file, not hyphenated at run time.
- **Settings card and row:** a section's rows sit in one card. A row is label (600, `--ink`) above one helper
  line (`--mute`), control on the right, a hairline between rows. One row is one config key.
- **Select / text input:** `--surface-2` fill, 1 px `--hairline-strong` border, 8 px radius, 7×10 px padding,
  selects at most 300 px wide (long device names are cut with an ellipsis). Hover lightens the border.
- **Buttons:** ghost pill (hairline border, ink text) by default; the white pill (`#fff`, text `#18191a`) is the
  primary action of a row or banner (Start, Download, Add key, Save). Disabled uses `--hairline` and `--mute`.
- **Focus:** a 2 px `--focus` outline offset by 2 px on every control.
- **Toggle:** an 18 px checkbox with 5 px radius; checked is violet with a dark check. No pill switches.
- **Key chip:** keycap — 7 px radius, `#1c1d1f → #0d0d0d` gradient, `#2c2e30` border, 1 px inner top highlight.
- **Shortcut field:** the shortcut as key chips, then a ghost "Change" pill (and "Reset" when it is not the
  default). "Change" turns the control into a field with a 1 px `--violet` border that shows the held
  modifiers as chips and "Esc keeps the current one" in `--mute`; the first non-modifier key saves.
- **Term list (vocabulary):** a full-width block inside the card, between rows: label and count on one line,
  then numbered rows of a text input plus 32 px round ghost buttons (↑, ↓, ×). A skipped term gets a
  `--danger` border and a reason line; a term past the length limit a `--mute` line. The whole list saves on
  every change. Long controls (a file path) go on their own line under the row, not in the row's control slot.
- **Status text:** "Stored ✓" or "Saved" in `--violet` (600). Errors in `--danger` with their stable code in
  parentheses.
- **Banner:** `--surface-2` card above the settings card, text in `--ink`, a white pill action on the right.
- **Navigation (app):** wordmark on top, then text links in `--mute`; the active one is `--ink` on `--surface-2`
  with a hairline ring, 8 px radius.

## Behaviour

- Changes save immediately to `config.toml` (no Save button). The row's helper line briefly shows "Saved".
  Settings that need a daemon restart say so.
- Secrets are never shown. A key row reads "Stored ✓" with "Change" / "Delete", or "Add key", which opens a
  masked input.
- The window is about 900×640 and resizable down to 720×520. Below 760 px wide, the sidebar collapses into a
  select at the top.
- Keyboard: every control is reachable with Tab. Ctrl+W and Esc close the window.
- The site honours `prefers-reduced-motion` by stopping every animation; the app has no animation.

## Language

- English is the default for the settings window, the tray menu and tooltip, and toasts. Turkish is used when
  the Windows display language is Turkish: the window reads `navigator.language` (WebView2 takes it from the
  display language), the daemon reads `GetUserDefaultUILanguage`. There is no in-app language switch.
- Every text lives in one table per language: `apps/settings/src/lib/strings.ts` (`en`, `tr`),
  `crates/hece-platform/src/i18n.rs` (tray) and `crates/hece/src/i18n.rs` (toasts). A new text is added to both.
- Error codes (`invalid_config`, `paste_failed`, ...) are never translated. Error details, CLI output and logs
  stay English.
- The window title and the Start menu shortcut are "Hece Settings" in every language.

## Recording overlay

The animated sphere in the top-right corner while recording (look "A3", chosen on 2026-11-02; spec in
`docs/superpowers/specs/2026-11-02-recording-overlay-design.md`). It is a deliberate exception to the single-accent
and no-gradient rules: it lives on the user's desktop, not in a window, and its job is to be noticed.

| Use | Colour |
| --- | --- |
| Entry (mic opening) | light blue `#5ab4e6` mixed into violet |
| Base mesh, speaking | violet `#9682ff` |
| Silent | silver `#a8acb6` |
| Syllable waves, transcribing | rainbow (hue from the edge's angle, turning with time) |
| Failed or cancelled | red `#b3261e` on the core |
| Not pasted (focus changed) | amber `#f5a524` on the core |

140×140 logical px, 16 px from the work area's top-right corner, click-through, never focused. Exits take 300 ms.
With Windows animation effects off it stops rotating and deforming; colour still follows the state.

## Website

- **Orb:** the overlay look (A3) in CSS: rainbow ring blurred behind a violet core with two syllable waves. It
  spins slowly.
- **Copy:** facts must match `.agents/product-marketing.md`. No invented numbers, testimonials or logos. Examples
  are synthetic and labelled as illustrative.

## Avoid

Light sections or a light theme, a second primary colour, shadows on chrome, pastel tints, Inter/Geist, stock
photos, emoji in the UI, and gradients other than the keycaps, the orb and the site's hero glow.
