# hece design: Dizgi

The settings app's visual language. Direction **A · Dizgi** was chosen on 2026-11-02 from three candidates; see
the PR #37 history for the board. Character comes from the name: *hece* means syllable, so the app reads like a
typeset page and the syllable dot marks rhythm. Every screen follows this file, and new decisions are recorded here.

Reference: WIRED (editorial duet of display serif + clean sans, hairlines, almost no chrome). From it we take the
hierarchy, hairline rules, square controls and colour restraint. The palette and fonts are our own.

## Type

| Role | Font | Size / line / tracking | Weight |
| --- | --- | --- | --- |
| Section title | Instrument Serif | 44 / 1.0 / -0.5px | 400 |
| Sub-title, quotes | Instrument Serif italic | 19 / 1.35 | 400 |
| Body, labels, controls | IBM Plex Sans | 14 / 1.4 | 400, 500, 600 |
| Helper text | IBM Plex Sans | 12.5 / 1.4 | 400 |
| Key chips | IBM Plex Sans | 12 / 1 | 500 |

Fonts are bundled with the app (SIL OFL), never loaded from the network. No Inter or Geist. No uppercase or mono
labels.

## Colour

| Token | Hex | Use |
| --- | --- | --- |
| `--paper` | `#fbfaf7` | window and content background |
| `--ink` | `#111111` | text, control borders, active nav marker |
| `--muted` | `#757575` | helper text, inactive nav (4.6:1 on paper) |
| `--hairline` | `#e0e0e0` | row rules, nav divider |
| `--accent` | `#057dbc` | syllable dot, links, "saved" state, focus ring (4.5:1 on paper) |
| `--danger` | `#b3261e` | destructive actions and errors only |
| `--field` | `#ffffff` | control fill |

There is one accent, used sparingly: never as a background block, never for decoration.

## Space and shape

- 4 px grid. Content padding 26 px top and 30 px sides. Rows 14 px top and bottom. 22 px between a section's
  lede and its first row.
- Navigation column 150 px with a 1 px hairline on its right. Content column at most 640 px.
- Radius is 0 on every control. Only the OS window itself is rounded.
- No shadows. Depth comes from hairlines.

## Components

- **Section title:** the section name split into syllables with accent middle dots (`Sağ·la·yı·cı`), followed
  by a one-sentence lede in `--muted`. The syllable split is precomputed in the strings file, not hyphenated
  at run time.
- **Row:** label (600) above one helper line (`--muted`), control on the right, a hairline above it, and a
  hairline below the last row. One row is one config key. Never put rows inside cards.
- **Select / text input / button:** 1 px `--ink` border, `--field` fill, 7×10 px padding, square, selects at
  most 300 px wide (long device names are cut with an ellipsis). Hover
  inverts (ink fill, paper text). Focus shows a 2 px `--accent` outline offset by 2 px. Disabled uses
  `--hairline` border and `--muted` text.
- **Toggle:** a square checkbox styled as a 1 px box with an ink check. No pill switches.
- **Key chip:** one bordered box per key, 3×7 px, 4 px apart.
- **Status text:** "Kayıtlı ✓" or "Kaydedildi" in `--accent` (600). Errors in `--danger` with their stable code
  in parentheses.
- **Quote block:** the last dictation sample in Instrument Serif italic with a 2 px `--accent` rule on the left.
  Shown only where it explains a setting (cleanup, history).
- **Navigation:** text links. The active one is `--ink` 600 with a 2 px ink bar on the column's right edge.

## Behaviour

- Changes save immediately to `config.toml` (no Save button). The row's helper line briefly shows "Kaydedildi".
  Settings that need a daemon restart say so.
- Secrets are never shown. A key row reads "Kayıtlı ✓" with "Değiştir" / "Sil", or "Anahtar ekle", which opens
  a masked input.
- The window is about 900×640 and resizable down to 720×520. Below 760 px wide, the navigation collapses into a
  select at the top.
- Keyboard: every control is reachable with Tab. Ctrl+W and Esc close the window.

## Avoid

Cards, pastel tints, gradients, pill buttons, shadows, a second accent colour, emoji in the UI, and dark-graphite
+ neon (no dark theme until it is designed here).

## Website: Gece

The public site in `site/` has its own direction, **Gece**, chosen on 2026-11-03 from three candidates (Gece,
Sahne, Sıcak; #57). Reference: Raycast (dark canvas, surface ladder instead of shadows, white pill CTA, keycaps).
Its character comes from the product: the recording orb is the hero, so the first thing a visitor sees is what
they will see on their own desktop. The settings app keeps Dizgi until #59 decides otherwise; the "no dark theme"
rule above applies to the app, not the site.

| Token | Value | Use |
| --- | --- | --- |
| `--canvas` | `#07080a` | page background |
| `--surface` / `--surface-2` / `--card` | `#0d0d0d` / `#101111` / `#121212` | depth ladder (no shadows) |
| `--hairline` | `#242728` | card borders, rules |
| `--ink` / `--body` / `--mute` | `#f4f4f6` / `#cdcdcd` / `#9c9c9d` | headings / body / helper text |
| `--violet` | `#9682ff` | eyebrows, logo dot, list markers, caret (from the orb) |
| `--green`, `--blue`, `--yellow`, `--red` | `#59d499`, `#57c1ff`, `#ffc533`, `#ff6161` | only as small dots and tags in feature illustrations |

- **Type:** Schibsted Grotesk (variable, 400-700) for everything; headings 600 with negative tracking. Instrument
  Serif only for the `He·ce` wordmark. Both self-hosted in `site/fonts` (SIL OFL); no third-party requests.
- **Shape:** cards 12 px radius with a 1 px hairline; buttons are pills (white primary, hairline ghost). Keycaps
  are the only gradient besides the orb and the hero glow.
- **Orb:** the overlay look (A3) in CSS: rainbow ring blurred behind a violet core with two syllable waves. It
  spins slowly; `prefers-reduced-motion` stops every animation.
- **Copy:** facts must match `.agents/product-marketing.md`. No invented numbers, testimonials or logos. Examples
  are synthetic and labelled as illustrative.
- **Avoid:** light sections, a second primary colour, shadows on chrome, Inter/Geist, stock photos.
