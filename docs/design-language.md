# Taste Engine — Design Language

One visual system across the web app and the Android app
(`com.niknair.tasteengine`). This document is the source of truth for cohesion.
Any new UI, or any evaluation of existing UI, is judged against it.

## Palette

Two shell modes share one token family:

| Token | Crimson field | Charcoal shell |
| --- | --- | --- |
| Field / bg | `#c80044` (the hero accent) | `#151110` warm charcoal |
| Field deep / bg2 | `#a80038` | `#1d1817` (inputs) |
| Cards / surfaces | `#1a1718` (`--surface`) | `#221c1a` (`--card`) |
| Surface 2 | `#272124` | `#2a2321` |
| Surface 3 | `#302a2d` | — |
| Text | `#fdf6f6` | `#fdf6f6` |
| Muted | `#e8b8c8` | `#a49385` |
| Accent (fills/rings) | `#c80044` | `#c80044` |
| Accent strong (marks/hover/overlay) | `#e4628b` rose | `#e4628b` |
| Opinions on field | `--field-muted #ffdde6`, `--field-line rgba(255,255,255,.34)` | same |
| Danger | `#ef5b57` | `#ef5b57` |
| OK | `#7dd87d` | `#7dd87d` |
| Warn | `#d8a657` | `#d8a657` |

Rules: the field IS the page background on the vault and on full-page headers.
Never paint the field color on `html` (dark canvas below). Shadow never, except
the sanctioned filter-drawer slab below.

## Typography

- UI font: system stack (Segoe UI / Roboto / system-ui). The web app also
  self-hosts a grotesque for brand moments. No serif display anywhere.
- Section labels, chips, badges, toolbars: UPPERCASE micro-labels, 10–10.5px,
  letter-spacing `.08em`–`.1em`.
- Body 12–13px; captions 11px.
- No text shadows, no outlines on text.

## Corner radius

ZERO. All corners `0`: buttons, cards, chips, sheets, switches, inputs and
dialogs. Android enforces `RoundedCornerShape(0)` on every component and square
switch/track geometry. If you see a rounded corner in any Taste Engine UI, it is
a bug.

## Iconography

- Line icons, stroke width 2, round caps/joins, `18px` / `18dp` glyph inside
  `48–52px` hit areas.
- Default icon color = `currentColor`; active or selected affordances may fill.
- The circular-arrow is the restore glyph. Do not use a tray-with-up-arrow
  glyph for share or upload.

## Buttons

- Ghost/action buttons: square `48–52px`, transparent fill, `1px` border
  (`--line` family), text color. Hover: border and text go accent-strong.
  Danger variants use the danger color, with a danger border on hover.
- Chips/filter pills: rectangles (radius 0), uppercase micro text, dark
  print-pill on the crimson field (surface fill + text). ACTIVE chip = white
  background + crimson text on field; accent-strong text on work surfaces.
- FAB (web mobile only): obsidian disc, `2px` white ring. No floating elements
  anywhere else.
- Destructive actions always confirm (dialog or browser confirm).

## Controls

- Square switch: `46×26` track, `18×18` square knob, crimson fill when on,
  obsidian fill + muted knob when off. Identical metrics on web and Android.

## Surfaces & cards

- `1px` borders (`--line-strong` family), flat fills, radius 0.
- Cards float on the crimson field as obsidian slabs. Work views use the warm
  charcoal shell with `#221c1a` cards.
- NO shadows and NO glows. The single exception: the sticky filter/sort drawer
  under the toolbar may use `0 4px 14px rgba(0,0,0,.4)`.

## Effects

- Dithering: white 4px checkerboard at `.08` opacity over the field
  (`data-style="dithering"`). Overlay veils = obsidian base + rose checkerboard
  stipple.
- NO blur, no glassmorphism. Every element is a flat fill.
- Hover = flat color or border shift, never glow or shadow.

## Animation

- Transition budget: ≤200ms, `ease-out` or `cubic-bezier(.2,.8,.2,1)`.
- NO springs, no bounces, except the shimmer pulse.
- Loading = shimmer skeleton (Android `ShimmerBox` mirrors the web grid), never
  spinners.
- Drag interactions: previews scale `0.82 → 1.0` over 200ms on release.

## Scroll

- Thin themed scrollbars: `~10px` web track painted to match the shell, thumb
  `--line-strong`.
- Android: pull-to-refresh everywhere. Web: explicit refresh affordances.

## Density & layout

- Vault grid: 1–4 columns; thumb density 92 / 64 / 44 / 36dp (Android).
- Header strips must be the same height on every Android tab (~178–179px incl.
  crimson strip). Count/status text lives at the BOTTOM bar, never in headers.
- Section headers = uppercase micro text. Screens start at the content, no
  decorative titles.
- Web status bars ~34px: queue/count text left, activity right.

## Platform notes

- Android: full-page headers use the crimson field. Navigation is icons-only,
  with no badges and no counts on icons; counts appear as plain text where
  approved.
- Web: the field covers the vault and full-page headers; the shell below stays
  charcoal.

## History of reversals (do not re-propose)

- Circular scrubber knob: removed on web and Android.
- Center preview card while scrubbing: removed; scrub scrolls instead.
- Dither textures inside controls: rejected. Dither stays as the page-wide
  overlay, not inside controls.
- Rounded corners: removed everywhere.
- Glow and blur effects: removed everywhere.
