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

3. **Theme from `Tokens`, passed as validated data.** `fm-ui` derives a `Palette` (dark flag, background, node fill, node border, text, line, cluster fill, cluster border, edge-label background, note fill and border) from the active `Tokens`, so dark/light, high contrast and accent settings apply. `Tokens` live in L4; the engine and `fm-preview` receive plain `#rrggbb` strings, validated in the command and again in `render` (malformed values → `Err`). The font is always the bundled face (decision 4), so it is not part of the palette. The palette becomes Mermaid `base` theme variables, compiled into a merman `DiagramTheme` bound to the request. A document cannot override it: the parser's `secure` list is merman's hardened default list plus `theme`, `themeVariables`, `themeCSS`, `darkMode`, `fontFamily`, `altFontFamily` and `look`, so directives and front matter naming those keys are ignored (without the lock, a document's `themeVariables` won over the bound theme). Other init settings (layout options) still apply.

4. **Text measurement matches drawing.** Clipped labels come from measuring and drawing with different fonts. merman measures with `native-fonts` from the same font bytes resvg draws with: the bundled Atkinson Hyperlegible (see Outcomes 1.2). The palette's font family is that font, not the UI's configurable font, so measurement and drawing always agree.

5. **Engine service `md_diagrams` modelled on `md_images`.** Key: (hash of source, palette hash, scale bucket). Scale bucket = preview zoom × pixels-per-point rounded to 0.25, so resizing never re-renders and zoom steps re-render at most once per step. State: `Pending | Ready(Arc<Thumb>) | Failed(reason)`. `preview.md_diagrams.want {pane?, blocks: [index], palette, scale}` queues missing keys for the pane's loaded document (blocks addressed like `preview.code.copy`). One worker thread, a queue of at most 32 jobs; jobs for a document that is no longer shown are dropped, and a generation counter discards stale results. The cache holds at most 64 MiB of bitmaps (least recently drawn evicted) and 256 entries.

6. **Limits** (constants in `fm_preview::mermaid`, tested): source at most 64 KiB per diagram; at most 50 diagrams drawn per document (later ones stay code blocks with the reason "Too many diagrams in this document"); `merman`'s default resource limits plus a 2 s deadline through `OperationControl`; SVG output at most 8 MiB; raster edge and pixel count from the existing image limits; `catch_unwind` around the render so a renderer bug becomes a `Failed` state.

7. **Offline.** `merman`'s input resource policy denies every external resource. The SVG then goes through the existing `svg::rasterize`, which loads no external or embedded images and no CSS files. `click`, `href` and icon references are ignored.

8. **UI.** In `preview_markdown`, a code block whose language is `mermaid` (case-insensitive, first word of the info string) and the preference on: `Ready` draws the bitmap with the code panel's copy control top-right (same `preview.code.copy`, copies the source); `Pending` draws a quiet placeholder of the last known size for that key (else 160 points high) with a spinner, so the page jumps at most once; `Failed` draws the ordinary highlighted code block with one muted line above it: "Diagram not drawn: <reason>". A drawn diagram sits in a panel with the code panel's header (language label, Copy) plus a "Show source" text button; `preview.md_diagrams.toggle_source {pane?, block}` records the block in the preview slot for the current file (like image rotation), and the block is then drawn as code with "Show diagram". Document copy (Ctrl+A, Ctrl+C) still contains the Mermaid source as code text. Accessibility: the diagram's accessible name is "Mermaid diagram" plus its type ("flowchart"), and its description is the first 1000 characters of the source; the source view and the copy control give the full text.

9. **Find** skips blocks drawn as diagrams (they have no highlightable text) and searches them as code when shown as code (failed, preference off, source view).

10. **Preference** `preview_mermaid: bool` (default true) in `Prefs`, Preferences > Preview > "Draw Mermaid diagrams", set through the existing preferences command; turning it off shows code blocks on the next frame without re-parsing.

## Outcomes of tasks 1.1 to 1.3 (2026-11-16)

- **1.1 Footprint.** `merman = 0.10.0`, `default-features = false`, features `svg` plus the eleven diagram types, plus `native-fonts` (decision 4): 145 crates in its tree, about 44 not yet in the workspace (mostly merman's own crates and the HTML/CSS sanitising stack: html5ever, cssparser, palette, phf; `native-fonts` adds rustybuzz and two Unicode tables). No diagram type needs `layout-elk` or `layout-cytoscape`; both stay off, and all eleven types stay in.
- **Layout note.** With the trimmed features, a subgraph without outside links is laid out in the crossed direction (a TB diagram gets LR subgraphs) unless it sets `direction`. This is mermaid.js's own rule, mirrored in merman's `effective_cluster_dir`; the earlier full-feature render differed and is not the reference.
- **1.2 Text measurement.** `NativeFontContext::from_font_bytes` with the bundled Atkinson Hyperlegible Regular and Bold, and resvg drawing with the same family (`font_family` and sans-serif set to it), removes the clipped edge labels ("PR build", "main deploy", "tag deploy") and wraps long node labels to their measured width. Characters missing from Atkinson (for example `→`) are drawn by resvg's system-font fallback; measurement uses Atkinson only, which is accurate enough for such rare glyphs. Consequence: the two font files move from `fm-ui` to `fm-preview` (L1) so both the renderer and the Markdown text can use them; `fm-ui::markdown_fonts` reads them from there.
- **1.3 Offline.** merman's `InputResourcePolicy` is a set of input size limits, not a fetch switch; offline-ness is structural instead: merman's tree has no HTTP client or async runtime, and its only file reads are in tests. The resvg-safe SVG of both documents contains no `<foreignObject>` element (the word appears only in data attributes and CSS selectors), no `<image>`, and only same-document `href="#…"` references. Image nodes (`@{ img: … }`) can still produce `<image>` elements; `svg::rasterize_scaled` refuses every image reference like all SVG drawing in the app, so nothing is read. merman reports some grammar budgets (the flowchart edge limit) as plain parse errors with a generic code; `render` classifies "limit exceeded" messages as too complex. `fm-preview` still rasterizes through its own fontdb with no resource directory, so nothing named by a diagram can be read. Test 2.3 asserts these properties on every rendered example.

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
