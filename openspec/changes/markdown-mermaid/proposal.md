## Why

Project documentation increasingly carries its architecture and process diagrams as Mermaid code blocks (`` ```mermaid ``), which every major Markdown host draws as pictures. The Markdown preview shows them as code, so a README or architecture document reads very differently in the file manager than on the forge where it lives. A spike (2026-11-16) showed that a pure-Rust renderer, `merman`, draws real-world flowcharts with subgraphs, edge labels and dotted links faithfully, offline, in about 15 ms, and through the resvg path the preview already uses for SVG. Diagrams can therefore be added without a webview or JavaScript.

## What Changes

- Rendered Markdown previews draw `mermaid` code blocks as diagrams, in the app's dark or light theme colours, scaled with the Preview zoom and the document width.
- Supported diagram types: flowchart/graph, sequence, class, state, entity-relationship, Gantt, pie, user journey, timeline, git graph and mind map. Unsupported, broken or over-limit diagrams keep showing their code with a one-line reason, so nothing is lost.
- A diagram keeps the code block's copy control, which copies the Mermaid source. The existing source view (`preview.toggle_source`) shows the source of the whole document.
- Rendering runs off the UI thread on a small bounded pool, with caps on diagram source size, diagrams per document, layout size, render time and cached bitmap memory. Nothing is ever fetched: links, icons and images inside diagram definitions are not loaded.
- New preference Preferences > Preview > "Draw Mermaid diagrams" (default on). When off, Mermaid blocks are ordinary code blocks.
- Preview Find skips the source of a block that is shown as a diagram (there is no text to highlight); it searches it in the source view or when diagrams are off.
- New dependency `merman` (MIT OR Apache-2.0) with default features off, built with only the SVG output and the listed diagram types.

## Capabilities

### New Capabilities

None. Diagrams are part of the Markdown preview.

### Modified Capabilities

- `preview-pane`: new requirement "Mermaid diagrams in Markdown" (rendering, theme, zoom, fallback, copy, Find, preference, limits as seen by the user).
- `security`: new requirement "Mermaid diagrams are offline and bounded" (untrusted diagram source: no resource loading, size/time/memory caps, no panics escaping, never on the UI thread).

## Impact

- `crates/preview` (L1): new `mermaid` module that turns bounded Mermaid source plus a theme palette into a rasterized image through `merman` (resvg-safe SVG pipeline) and the existing `svg::rasterize`.
- `crates/engine` (L3): new `md_diagrams` service and `preview.md_diagrams.want` command, modelled on `md_images` (state queried by the UI, work queued by command, bounded pool, cache, cancellation); new `Prefs` field `preview_mermaid`.
- `crates/ui` (L4): `preview_markdown` draws a diagram, a loading placeholder or the code block with its reason; palette derived from `Tokens`; Preferences > Preview toggle; Find skips drawn diagrams.
- `Cargo.toml`: `merman` in workspace dependencies (resvg is already present). Expect roughly 20 to 40 new transitive crates; the exact feature set is settled in the first task by measuring.
- Docs: `docs/architecture.md` (preview pools), `AGENTS.md` §7 (dependency note), CHANGELOG.
