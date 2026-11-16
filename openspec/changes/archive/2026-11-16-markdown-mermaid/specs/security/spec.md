## ADDED Requirements

### Requirement: Mermaid diagrams are offline and bounded
Mermaid source in a previewed document is untrusted. Drawing it SHALL never happen on the UI thread and SHALL never load any resource: no network access, no file reads, no fonts, images, icons, stylesheets or links named by the diagram (text is measured and drawn with the bundled reading face, falling back to installed system fonts for missing characters). A document's own directives SHALL NOT change the renderer's hardened settings or any theme setting. Each diagram's source SHALL be at most 64 KiB; the renderer's own limits on nodes, edges and nesting SHALL apply; one render SHALL be abandoned after 2 seconds; the intermediate SVG SHALL be at most 8 MiB; the bitmap SHALL obey the image preview's edge and pixel limits; drawn diagrams SHALL be cached in at most 64 MiB, least recently drawn evicted; at most one diagram SHALL be drawn at a time, with at most 32 waiting, and work for a document that is no longer shown SHALL be dropped. A renderer failure, including a panic inside the renderer, SHALL become a "Diagram not drawn" reason and SHALL NOT crash the app. The `preview.md_diagrams.want` command SHALL reject malformed parameters (unknown pane or block, colours that are not `#rrggbb`, an out-of-range scale) with an error.

#### Scenario: Hostile diagram source
- **WHEN** a document contains a 3000-edge chain, 200 nested subgraphs, a 10 MiB `mermaid` block, and diagrams referencing `https://` images, icons and `click` links
- **THEN** the preview stays responsive, oversized or over-limit diagrams show their reason, memory stays within the caps, and nothing is fetched or read

#### Scenario: Renderer failure is contained
- **WHEN** the renderer panics or runs past its deadline on a diagram
- **THEN** that block shows "Diagram not drawn" with a reason, other diagrams still draw, and the app keeps running

#### Scenario: Malformed command parameters
- **WHEN** `preview.md_diagrams.want` receives a colour `red; url(x)`, a scale of 0 or NaN, or a block index past the end
- **THEN** it returns an error and queues nothing
