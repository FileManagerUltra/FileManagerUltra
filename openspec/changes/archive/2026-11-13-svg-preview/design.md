## Context

- `fm-preview::svg::rasterize` already turns SVG bytes into an RGBA `Thumb` for Markdown images. `resvg` is built with default features off, so text, embedded raster images and external files are not rendered; nothing can be read from disk or the network.
- Preview routing: filename rules (`preview_types.rs`, user-editable) first, then content sniffing (`sniff.rs`). Defaults put `*.svg` under XML, so SVGs become `Kind::Structured(Xml)`.
- `PreviewSlot::source` and the `preview.toggle_source` command already give Markdown a rendered/source switch; the header shows it as a selectable **Preview** button.
- Image previews (`Preview::Image`) already support Fit / 100% and pane zoom.

## Goals / Non-Goals

**Goals:**
- Render SVG files in Preview, switchable to highlighted source with one click.
- Keep every existing limit and the no-external-resources rule.
- Migrate existing File types settings so SVG rendering works without the user editing anything.

**Non-Goals:**
- Editing SVGs, animation (SMIL/CSS), scripts, or interactivity.
- Re-rasterizing on every zoom step (vector-sharp zoom); a single raster at preview resolution is used.
- `.svgz` (gzip-compressed SVG).
- SVG thumbnails in directory thumbnail grids (can follow later).

## Decisions

1. **Its own preview type, before XML.** A separate `Svg` type keeps the File types list honest ("which files render as pictures") and lets users send some `.svg` files back to XML by moving patterns. Alternative: add `*.svg` to Images. Rejected: Images implies raster decoding and thumbnails, and the source toggle would not belong there.

2. **One preview model with both views.** The worker reads the file once (text read limit), rasterizes it, and keeps the source text, so toggling is instant and needs no re-read. The source view reuses the XML highlighting and Pretty path.

3. **Raster size.** Render so the longest edge is the larger of the SVG's own size and 2048 px, capped by the existing image edge and pixel limits. Fit then looks sharp in large panes, and 100% means the SVG's declared size. Alternative: re-render to the pane size on resize/zoom. Rejected for now: more worker churn for little gain.

4. **Checkerboard background** drawn from theme tokens (`bg` / `panel` squares), so transparent artwork with black strokes is visible in dark mode, and no colour literal is added.

5. **Migration.** When stored groups have no `svg` entry, insert `{svg: ["*.svg"]}` directly above the XML group and remove only the exact default `*.svg` from XML. If XML is missing, append SVG at the end. Restore defaults gives the new default order.

6. **Failure path.** Parse or size errors show the reason above the source view rather than a blank pane.

7. **Text with system fonts (Open Question 1, approved).** `resvg` features `text` + `system-fonts` (not `memmap-fonts`: font files are read, not mapped). The font database loads once per process on the first SVG, on a worker. Serif / sans-serif / monospace point at installed fonts; a named font that is not installed falls back by name to monospace, serif or sans-serif before usvg's final serif fallback.

## Risks / Trade-offs

- [Text depends on installed fonts] → generic families and absent named fonts map to installed fonts (decision 7); text is tested in `fm-preview`, and UI snapshots avoid text so they don't vary by machine.
- [A span mixing Latin with characters its font lacks (e.g. CJK) is re-shaped entirely in the fallback font, losing italics] → upstream usvg behaviour; accepted.
- [usvg's default `<image href>` resolver reads any local path, which also affected Markdown images before this change] → resolvers that refuse everything, with a test that a referenced local SVG is not drawn.
- [Very large or deeply nested SVGs take long to parse] → existing read-size cap, usvg's own limits, worker cancellation on selection change; tests with a hostile SVG.
- [Users who prefer source for SVG] → one click on Preview, or move `*.svg` back to XML in File types.

## Migration Plan

Automatic on first load of older preferences (decision 5); no rollback is needed because older builds ignore unknown types and use defaults for them.

## Open Questions

None. Text rendering (formerly Open Question 1) was approved on 2026-11-13 ("We use text in SVG a lot"). New crates: `fontdb`, `fontconfig-parser`, `unicode-bidi`, `unicode-vo`, `xmlwriter`, `roxmltree` 0.20 (all pure Rust).
