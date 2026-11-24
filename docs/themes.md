# Themes

A theme restyles the browser chrome and, if you want, the pages you visit. It is one CSS file:
`:root` custom properties set the shell's tokens and `@tarn-site` blocks style web pages. A theme
cannot run code, reach the network, or hide or fake security UI (origin chip, "Not Secure",
Shield, prompts and warnings use a fixed palette no token reaches). Parsing and all rules live in
`browser_core::theme`; the shells only apply the result.

Sources: built-in themes, the official catalog (signed index), a GitHub release asset named
`theme.css`, a local file, the colour editor or your own CSS in Settings › Appearance. Themes from
GitHub or a file are marked Unverified. Every installed theme is checked against a revocation list
shipped with the filter bundle.

## Authoring reference

A theme is one UTF-8 `.css` file of at most 256 KiB (hosts may be quoted, and must be when a label
starts with a digit, as in `"www.163.com"`; public suffixes such as `*.com` are refused):

```css
@tarn-theme { name: "Glacier"; author: "You"; version: "1.0"; scheme: dark; }
:root { --tarn-accent: #1f75b0; }
@tarn-site github.com, *.example.org { body { font-family: "Inter"; } }
```

Tokens (defaults are Tarn's dark theme; a missing or invalid one keeps its default):

| Token | Value |
|---|---|
| `--tarn-backdrop`, `--tarn-sidebar` | colour or `linear-gradient(<angle>deg, <colour>, <colour>)` |
| `--tarn-sidebar-edge`, `--tarn-space-label`, `--tarn-surface`, `--tarn-surface-edge`, `--tarn-page`, `--tarn-panel`, `--tarn-panel-nav`, `--tarn-panel-row`, `--tarn-separator`, `--tarn-control`, `--tarn-raised`, `--tarn-group`, `--tarn-hover`, `--tarn-pressed`, `--tarn-hairline`, `--tarn-text`, `--tarn-muted`, `--tarn-faint`, `--tarn-accent`, `--tarn-fog-1`, `--tarn-fog-2`, `--tarn-fog-3`, `--tarn-fog-4` | `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`, `hsl()`, `hsla()` (numbers only), a CSS colour name or `transparent` |
| `--tarn-radius-small`, `--tarn-radius-row`, `--tarn-radius-group`, `--tarn-radius-tile`, `--tarn-radius-panel`, `--tarn-radius-card` | `0` to `24px` |
| `--tarn-shadow` | `0` to `1.5` |
| `--tarn-fog` | `on`, `still` or `off` |
| `--tarn-search-effect` | `none`, `glow`, `flames`, `aurora` or `sparkle`: drawn around the search field on the start page and in the command bar (one still frame with Reduce Motion); Settings › Appearance › Search bar effect overrides it |
| `--tarn-search-color` | a colour for the effect (its alpha is ignored); unset means the accent |
| `--tarn-font` | an installed font family |

A translucent `--tarn-backdrop` blurs the desktop through the window only for the built-in Liquid
Glass themes, whose text is checked over a white and a black desktop; any other theme draws on an
opaque window.
`--tarn-danger`, `--tarn-warning` and `--tarn-success` are fixed for security and ignored.
Website styles: `@media`, `@supports` and `@keyframes` (not in `*`); selectors with `:not`, `:is`,
`:where`, `:has` (not nested, own and official themes only), `:nth-*`, `:lang`, `:dir`; values
without `url()` or any function that loads or reads something; text `color` a hex, numeric
`rgb`/`hsl` or CSS colour name at least 60 % opaque, or `currentcolor` and the CSS-wide keywords
(no system colours). Themes installed from GitHub or a file may only set colours and
fonts on sites (`color`, `background-color`, the `border-*-color` properties, `outline-color`,
`accent-color`, `caret-color`, `text-decoration-color`, `column-rule-color`, `font-family`,
`scrollbar-color`), and so may `@tarn-site *` in every theme. To publish on
GitHub, attach the file as `theme.css` to each release.

## Where website styles apply

- Only the top frame of `http`/`https` pages with a host; never `about:`, `data:`, `file:`,
  internal pages or under Lockdown. Hosts are names; IP addresses and public suffixes are refused.
- Order in the injected sheet: the theme's `*` block, the theme's host blocks, dark mode, then
  your own per-site CSS, which always wins.
- Per profile, "Theme styles on websites" is on for ordinary profiles and off in private spaces
  and burner tabs.
- A theme's website styles total at most 64 KiB (`store::MAX_SITE_CSS`), on top of your own CSS
  for a site. The CSS is tokenized and re-serialised from the accepted tokens, so the engine parses
  exactly what was checked.

## Safety nets

- **Contrast:** text, muted text, the space name and placeholders are moved until they are
  readable on every background they sit on; if no colour works, text and backgrounds return to the
  defaults. Settings lists what was adjusted.
- **Safe mode:** hold Shift while Tarn opens its first window to use the built-in theme without
  website styles for that run.
- **Bad stored theme:** a theme that no longer parses falls back to the built-in theme of the same
  scheme and shows a notice; the stored theme is kept.
- **Withdrawn themes:** a theme on the revocation list is switched to the default and cannot be
  selected again.
