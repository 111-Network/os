# Dashboard fonts

Self-hosted woff2 under `ui/public/fonts/` (copied into `dist/fonts/` by Vite). The dashboard makes no
runtime network request for fonts (spec section 2). All four faces are licensed under the SIL Open Font
License 1.1; the license texts ship next to the files and are copied into `dist/`.

| Family | File | Weight | Bytes | Role |
|---|---|---|---|---|
| Geist Mono | `GeistMono-Variable.woff2` | variable, `100 900` | 71004 | Body and all UI text. **Preloaded.** |
| Geist Pixel Line | `GeistPixel-Line.woff2` | 500 | 18192 | View titles (`font-pixel-line`). |
| Silkscreen | `Silkscreen-400-latin.woff2` | 400 | 8404 | Logo and pixel badges (`font-pixel`). |
| Silkscreen | `Silkscreen-700-latin.woff2` | 700 | 7524 | Same, bold. |
| Licenses | `LICENSE-geist.txt` (4368), `LICENSE-silkscreen.txt` (4394) | | | OFL 1.1 texts. |

Total font payload if every face loads: 105124 bytes (woff2 is already compressed, so this is also the
transfer size). Fonts are not counted in the JS+CSS budgets (256 KB eager + 256 KB lazy, D21); `pnpm budget` lists them separately.

## Provenance

- **Geist Mono, Geist Pixel Line.** Copied byte-for-byte from the `geist` npm package, version 1.7.0
  (`website/node_modules/geist/dist/fonts/geist-mono/GeistMono-Variable.woff2` and
  `geist-pixel/GeistPixel-Line.woff2`). Copyright (c) 2023 Vercel, in collaboration with basement.studio.
  License text: the package's `LICENSE.txt`, copied as `LICENSE-geist.txt`.
- **Silkscreen 400 and 700.** Downloaded 2026-10-13 from Google Fonts. The CSS at
  `https://fonts.googleapis.com/css2?family=Silkscreen:wght@400;700&display=swap` (fetched with a Chrome
  user agent) listed these woff2 files, which were then fetched from `fonts.gstatic.com`:
  - 400: `https://fonts.gstatic.com/s/silkscreen/v6/m8JXjfVPf62XiF7kO-i9YLNlaw.woff2`
  - 700: `https://fonts.gstatic.com/s/silkscreen/v6/m8JUjfVPf62XiF7kO-i9aAhAfmyi2A.woff2`

  Copyright 2001 The Silkscreen Project Authors (https://github.com/googlefonts/silkscreen). License text:
  `https://raw.githubusercontent.com/google/fonts/main/ofl/silkscreen/OFL.txt`, saved as
  `LICENSE-silkscreen.txt`.

SHA-256 of the shipped woff2 files:

```
5f687a5dd4c87da13deaff9f6b9503d5e62249ff501265a96b134565f9aa8c87  GeistMono-Variable.woff2
3a22f02388d8b30780ec260eb064e9701f75a1f9bac7f9e79d5efdf96f8d4259  GeistPixel-Line.woff2
e6c72ea4702249202bcdd79d3343057e4e25ef1f04e3fcffd8602ab53b40b4cc  Silkscreen-400-latin.woff2
5741b7f01a127f40ed70281fa2f686e8fc2cfe241930a120e4d1a1cebe78f995  Silkscreen-700-latin.woff2
```

## Subsetting note

Google serves Silkscreen as two unicode-range subsets (`latin`, `latin-ext`). Only `latin` is vendored
because the logo and pixel badges are ASCII. A glyph outside it falls back to Geist Mono through the
`--font-pixel` stack. To add latin-ext, download the `latin-ext` URLs from the CSS above, add `@font-face`
rules with the matching `unicode-range` in `ui/src/styles/globals.css`, and update this table.

## Preload rule

Only Geist Mono is preloaded (`<link rel="preload" as="font" type="font/woff2" crossorigin>` in
`index.html`): it is the body face, so every first paint needs it. Pixel Line and Silkscreen are not
preloaded; with `font-display: swap` they load on first use and the Geist Mono fallback shows meanwhile.
Preloading more would delay first paint on slow phone links for text that is not on screen yet.
The `crossorigin` attribute is required even for same-origin fonts, otherwise the browser fetches the file twice.

## Updating

Replace the file, keep the filename or update `globals.css` and `index.html`, then refresh the byte sizes
and hashes above (`wc -c` and `sha256sum` over `ui/public/fonts/*.woff2`).
