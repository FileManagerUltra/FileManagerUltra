## Why

The SVG preview shipped with two non-goals that users notice quickly: folder thumbnail grids show no picture for `.svg` files, and compressed `.svgz` drawings open as a hex dump. The redesigned File types list also lacks arrow-key navigation in its pattern list, unlike the type list beside it.

## What Changes

- SVG and SVGZ files get thumbnails in folder thumbnail grids, following the SVG file type's patterns, with the same caching and limits as image thumbnails and the same safety rules as SVG previews.
- `.svgz` (gzip-compressed SVG) previews like `.svg`: the drawing plus its decompressed source. Decompression is capped at 16 MiB so a small file cannot expand without bound. The SVG type's default patterns become `*.svg` and `*.svgz`.
- In Preferences > Preview > File types, Up and Down move the selection in the pattern list when it has keyboard focus.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `preview-pane`: "SVG preview" covers `.svgz` and thumbnails; "Configurable preview filename filters" gains the SVG default `*.svgz` and arrow-key navigation in the pattern list.

## Impact

- `fm-preview`: `ThumbKind::Svg`, SVG thumbnails, capped gzip decoding for SVG previews and thumbnails, sniffing of gzip data named `.svgz`.
- `fm-engine`: SVG associations request SVG thumbnails; `*.svgz` default.
- `fm-ui`: arrow keys in the pattern list.
- No new dependencies (`flate2` is already used for `.gz` logs); `resvg`'s own `svgz` feature stays off because it decompresses without a limit.
