## 1. SVGZ

- [x] 1.1 `fm-preview`: capped gzip decoding (16 MiB) for SVG data; `.svgz` named gzip data sniffs as SVG and configured SVG associations accept gzip; tests incl. a decompression bomb
- [x] 1.2 `fm-engine`: SVG default patterns `*.svg; *.svgz`; tests

## 2. Thumbnails

- [x] 2.1 `fm-preview`: `ThumbKind::Svg` (classify `svg`/`svgz`), thumbnails via the safe SVG rasterizer within the requested edge; tests
- [x] 2.2 `fm-engine`: configured SVG associations request SVG thumbnails; test

## 3. File types keyboard

- [x] 3.1 `fm-ui`: Up/Down move the pattern selection while a pattern row has focus; test

## 4. Wrap-up

- [x] 4.1 CHANGELOG, docs/development.md, devlog; clippy, fmt, layers, brand-check green
