## Context

`fm-preview::markdown` parses documents with pulldown-cmark into `MdBlock`s; a fenced block keeps its info string as `CodeBlock::language` and its text, highlighted on the preview worker. `fm-ui::preview_markdown` draws blocks, including the code panel's copy control (`preview.code.copy`). Markdown images follow a pattern this change copies: the UI asks `MdImageService::state` what it may draw, the `preview.md_images.want` command queues what is missing, work runs on a bounded pool with caps on bytes, count, time and cache memory, and SVG images are rasterized by `fm_preview::svg::rasterize` (resvg, system fonts, no external resources).

The spike (scratchpad, 2026-11-16) rendered the two target documents (`it-architecture.md`: TB flowchart with four subgraphs, chained, labelled and dotted edges; `ecommerce-landscape.md`: LR flowchart with 20 labelled edges and `<-->`) with five pure-Rust crates. Only `merman` 0.10 matched Mermaid's own layout; with its `SvgPipeline::resvg_safe()` output every label, subgraph title and edge label renders through resvg, in 13 to 16 ms. Its default output keeps Mermaid's HTML labels (`<foreignObject>`), which resvg cannot draw. It rejected garbage with a clear parse error and refused a 3000-edge chain through its own resource limit in 1 ms. No candidate panicked. In the dark theme a few edge labels were clipped because `merman` measured text with built-in metrics while resvg drew with a different system font.

## Goals / Non-Goals

**Goals:**
- Mermaid code blocks in rendered Markdown are drawn as diagrams that read well in the app's current theme, contrast and accent settings.
- Untrusted diagram source can never block a frame, crash the app, fetch anything or use unbounded memory.
- Nothing is lost when a diagram cannot be drawn: the code block stays, with a reason.

**Non-Goals:**
- Previewing standalone `.mmd` / `.mermaid` files (a follow-up can reuse the same service).
- Interactive diagrams: clicks, tooltips, `click` callbacks, links inside diagrams.
- Searching or selecting text inside a drawn diagram.
- Pixel parity with mermaid.js.
- Diagrams in other places that show Markdown-like text (log previews, terminal).

## Decisions

1. **Renderer: `merman` with the resvg-safe pipeline.** Pure Rust, MIT OR Apache-2.0, built-in resource limits, cancellation (`OperationControl`), many diagram types. Alternatives from the spike: `mermaid-svg` (subgraphs merged, clipped titles), `mermaid-rs-renderer` (missing edges, 24 s on a large graph), `ariel-rs` (literal `<br/>` and quotes), `flowmaid` (GPL-3.0, incompatible with Apache-2.0). A Node/mermaid-cli helper or a webview contradicts the Rust-only rule. `merman` is used only behind `fm_preview::mermaid`, so a later swap touches one module. Built with `default-features = false` and `svg` plus the diagram features listed in the proposal; the layout engines (`layout-elk`, `layout-cytoscape`) stay off unless a listed type needs one, which task 1.1 measures. A type that would pull in a large layout engine is dropped from the list rather than accepted silently.

2. **Pipeline: source → SVG → bitmap, all on a worker.** `fm_preview::mermaid::render(source, &Palette, &DiagramLimits, &cancel) -> Result<Thumb, DiagramError>`: bounded source check, `merman` render with the palette as Mermaid `base` theme variables and `resvg_safe()`, SVG size check, then the existing `svg::rasterize` at the requested scale with the existing edge and pixel caps. The UI draws the bitmap like a Markdown image: scaled down to the document width, never up past natural size. Kept as a bitmap rather than re-rasterizing SVG per frame, as images already are.

3. **Theme from `Tokens`, passed as validated data.** `fm-ui` derives a `Palette` (background, node fill, node border, text, line, cluster fill, cluster border, edge-label background, note colours, font family) from the active `Tokens`, so dark/light, high contrast and accent settings apply. `Tokens` live in L4; the engine and `fm-preview` receive plain `#rrggbb` strings and a font family name bounded to 64 characters, validated in the command (malformed values → `Err`). A document's own `%%{init}%%` theme settings (`theme`, `themeVariables`, `themeCSS`) are overridden by the palette so diagrams stay readable for low-vision users. Other init settings pass through only if `merman` supports them; they change layout, not colours.

4. **Text measurement matches drawing.** Clipped labels come from measuring and drawing with different fonts. Task 1.2 enables `merman`'s `native-fonts` measurement with the same system font family that resvg draws with, and checks the spike's clipped labels in both themes. If that is not enough or too heavy, the fallback is extra label padding from the palette (`themeVariables` padding) and a bounded label-width allowance; the choice is recorded in this file.

5. **Engine service `md_diagrams` modelled on `md_images`.** Key: (hash of source, palette hash, scale bucket). Scale bucket = preview zoom × pixels-per-point rounded to 0.25, so resizing never re-renders and zoom steps re-render at most once per step. State: `Pending | Ready(Arc<Thumb>) | Failed(reason)`. `preview.md_diagrams.want {pane?, blocks: [index], palette, scale}` queues missing keys for the pane's loaded document (blocks addressed like `preview.code.copy`). One worker thread, a queue of at most 32 jobs; jobs for a document that is no longer shown are dropped, and a generation counter discards stale results. The cache holds at most 64 MiB of bitmaps (least recently drawn evicted) and 256 entries.

6. **Limits** (constants in `fm_preview::mermaid`, tested): source at most 64 KiB per diagram; at most 50 diagrams drawn per document (later ones stay code blocks with the reason "Too many diagrams in this document"); `merman`'s default resource limits plus a 2 s deadline through `OperationControl`; SVG output at most 8 MiB; raster edge and pixel count from the existing image limits; `catch_unwind` around the render so a renderer bug becomes a `Failed` state.

7. **Offline.** `merman`'s input resource policy denies every external resource. The SVG then goes through the existing `svg::rasterize`, which loads no external or embedded images and no CSS files. `click`, `href` and icon references are ignored.

8. **UI.** In `preview_markdown`, a code block whose language is `mermaid` (case-insensitive, first word of the info string) and the preference on: `Ready` draws the bitmap with the code panel's copy control top-right (same `preview.code.copy`, copies the source); `Pending` draws a quiet placeholder of the last known size for that key (else 160 points high) with a spinner, so the page jumps at most once; `Failed` draws the ordinary highlighted code block with one muted line above it: "Diagram not drawn: <reason>". Document copy (Ctrl+A, Ctrl+C) still contains the Mermaid source as code text. Accessibility: the diagram's accessible name is "Mermaid diagram" plus its type ("flowchart"), and its description is the first 1000 characters of the source; the source view and the copy control give the full text.

9. **Find** skips blocks drawn as diagrams (they have no highlightable text) and searches them as code when shown as code (failed, preference off, source view).

10. **Preference** `preview_mermaid: bool` (default true) in `Prefs`, Preferences > Preview > "Draw Mermaid diagrams", set through the existing preferences command; turning it off shows code blocks on the next frame without re-parsing.

## Risks / Trade-offs

- [`merman` is young (0.10) and its API may change] → Pinned exact minor version, used only inside `fm_preview::mermaid`, covered by golden tests on the spike's two documents (parse succeeds, element counts, no `<foreignObject>` left).
- [Dependency weight: about 44 new crates with default features] → Default features off; task 1.1 records the measured count; types that drag in heavy layout engines are dropped.
- [Fidelity differs from mermaid.js for some diagrams] → Failures fall back to code with a reason; the source view is one click away.
- [Label clipping from font metrics] → Decision 4; snapshot tests in both themes on the clipped labels from the spike.
- [Palette override changes the look authors chose] → Intentional for readability; documented in the spec. Revisit if users ask for author themes.
- [Memory] → Bitmap cache cap and scale buckets; a 50-diagram document at high zoom evicts least recently drawn diagrams instead of growing.

## Migration Plan

None. The preference defaults on; turning it off restores today's behaviour exactly. Removing the feature means deleting the module, the service and the dependency.

## Open Questions

- Whether `native-fonts` measurement alone removes clipping (task 1.2 decides, outcome recorded here).
- Whether mind maps need `layout-cytoscape`; if so, and it is heavy, they stay code blocks for now.
