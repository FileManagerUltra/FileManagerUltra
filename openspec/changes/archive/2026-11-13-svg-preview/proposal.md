## Why

`.svg` files open as XML source in Preview, so you cannot see what a drawing, icon or diagram looks like without leaving the file manager. The preview-pane spec already lists svg among rendered images, but the filename defaults send `*.svg` to XML and nothing renders it. A renderer (`resvg`) is already bundled for Markdown images, so the cost is small.

## What Changes

- SVG files render as a picture in Preview, with the same Fit / 100% controls as other images and a checkerboard behind transparent areas, so dark artwork stays visible in the dark theme.
- A **Preview** toggle in the header (the one Markdown uses) switches between the picture and the highlighted XML source, with Pretty and wrap as for XML. The choice resets when another file is selected, like Markdown's.
- An SVG that cannot be read shows a short reason and falls back to the source view.
- Preferences > Preview > File types gains an **SVG** type (default `*.svg`), placed before XML. Existing settings migrate: the SVG type is inserted just above XML and the default `*.svg` pattern is taken out of XML; custom patterns are left alone.
- Files matched by no pattern that look like SVG (an `<svg` element near the start) are detected as SVG.
- Text in SVGs is drawn with the installed system fonts; fonts named but not installed (Helvetica, Segoe UI, Consolas…) fall back to an installed font of the same kind.
- Rendering keeps today's safety rules: no external or embedded resources are loaded, sizes and pixel counts are capped, and the work runs on the preview worker, never the UI thread. This also closes a hole where `<image href="/some/path">` in an SVG (including SVG images in Markdown) made the renderer read that local file.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `preview-pane`: "Supported previews" gains SVG rendering with a source toggle; "Configurable preview filename filters" gains the SVG type, its default and the settings migration; "Markdown images" now draws SVG text; a new "SVG preview" requirement describes the view, fonts, limits and failures.

## Impact

- `fm-preview`: SVG kind in sniffing, an SVG preview model (rendered image plus source text), rasterizing at preview resolution (reusing `svg.rs`).
- `fm-engine`: `PreviewType::Svg` with defaults and prefs migration; the preview worker builds SVG previews; `preview.toggle_source` is enabled for SVG as well as Markdown.
- `fm-ui`: SVG view in `preview.rs` (picture or source, header toggle, checkerboard from theme tokens), snapshots in both themes.
- Docs: CHANGELOG, `docs/development.md` manual checks.
- Dependencies: `resvg` features `text` and `system-fonts` (pure-Rust font crates).
