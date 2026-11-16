## 1. Renderer choice settled

- [x] 1.1 Add `merman` (exact minor version) to workspace dependencies with `default-features = false`, `svg` and the diagram features from the proposal; register nothing new in `xtask/src/layers.rs` (it is a dependency of `fm-preview`, L1). Measure new transitive crates and build time; drop any diagram type whose feature pulls in a layout engine that is heavy, and record the count and decision in design.md
- [x] 1.2 Text measurement: try `native-fonts` measurement with the same system font resvg draws with; check the spike's clipped edge labels ("PR build", "tag deploy", subgroup titles) in both palettes; record the outcome (or the padding fallback) in design.md
- [x] 1.3 Confirm `merman`'s input resource policy can deny every external resource and that the resvg-safe SVG has no `<foreignObject>`, `<image>` with external `href`, or `<style>` `@import`; add the policy to the render call

## 2. `fm-preview::mermaid` (L1)

- [x] 2.0 Move the Atkinson Hyperlegible font files from `crates/ui/assets/fonts` to `crates/preview/assets/fonts` and expose them from `fm_preview` (`fm-ui::markdown_fonts` uses them from there); no visual change
- [x] 2.1 `Palette` (validated `#rrggbb` colours, `hash()`), `DiagramLimits` constants (64 KiB source, 2 s deadline, 8 MiB SVG, image edge/pixel caps), `DiagramError` with short user-facing reasons
- [x] 2.2 `render(source, &Palette, scale, &cancel) -> Result<Thumb, DiagramError>`: size check, `merman` with `base` theme variables from the palette (document `%%{init}%%` theme keys overridden), `resvg_safe()`, deadline through `OperationControl`, SVG size check, `svg::rasterize`; `catch_unwind` around the renderer; `diagram_kind(source)` for the accessible name
- [x] 2.3 Tests: golden renders of the spike's two documents (succeeds, expected node/cluster/edge-label counts, no `<foreignObject>`), every supported type renders a minimal example, hostile inputs (garbage, 10 MiB, 3000-edge chain, 200 nested subgraphs, `click`/`href`/icon/`https` image references produce no I/O), deadline and cancellation, palette validation, init-directive theme override

## 3. Engine service (L3)

- [x] 3.1 `md_diagrams.rs` modelled on `md_images`: key (source hash, palette hash, scale bucket of 0.25), states `Pending | Ready(Arc<Thumb>) | Failed(reason)`, one worker, queue ≤ 32, generation counter, jobs for documents no longer shown dropped, 64 MiB / 256-entry LRU cache, waker for the UI
- [x] 3.2 `preview.md_diagrams.want {pane?, blocks, palette, scale}` command (new `*_cmds.rs` module, added to `commands.rs`), block indices addressed like `preview.code.copy`, at most 50 diagrams per document; malformed-params fuzz coverage plus explicit bad-colour/font/index/scale tests
- [x] 3.3 `Prefs::preview_mermaid` (default true, serde default) through the existing preferences command; round-trip and bad-value tests
- [x] 3.4 Tests: state transitions, cache eviction under the cap, stale generation discarded, panic in a job becomes `Failed`, nothing rendered on the UI thread (work only from the command/worker)

## 4. UI (L4)

- [x] 4.1 `Palette` from `Tokens` (both palettes exhaustive; contrast of text on node fill and of lines on background meets WCAG 1.4.3 / 1.4.11, tested for every accent in both themes like the zoom-button test)
- [x] 4.2 `preview_markdown`: `mermaid` blocks with the preference on show the bitmap (scaled to width, never up), the copy control top-right (`preview.code.copy`), a placeholder of the last known size while pending, the code block with "Diagram not drawn: <reason>" when failed; dispatch `want` for visible diagrams only
- [x] 4.3 Accessibility: accessible name "Mermaid diagram, <type>" and description (first 1000 chars of source); copy control keyboard reachable as on code panels
- [x] 4.4 Find skips drawn diagrams and searches failed/off/source-view blocks as code; document copy keeps the source
- [x] 4.5 Preferences > Preview: "Draw Mermaid diagrams" checkbox with hover text
- [x] 4.6 Snapshots (dark and light): drawn flowchart with subgroups, pending placeholder, failed block with reason, preference off; UI harness test that the copy control copies the source
- [x] 4.7 (requested during apply) "Show source" / "Show diagram" icon control per diagram: `preview.md_diagrams.toggle_source {pane?, block}`, state per document in the preview slot, diagram panel gets the code panel's header, Find searches blocks shown as source; engine and UI harness tests, snapshot
- [x] 4.8 (requested during apply) icon controls instead of text buttons (`</>` source, node diagram, picture) and "Copy diagram as image": `preview.md_diagrams.copy_image {pane?, block, palette, scale}` flattens the drawn bitmap onto the palette background with a 16-point margin (`mermaid::flatten`) into the existing clipboard-image outbox; tests in fm-preview, engine and UI harness

## 5. Docs and wrap-up

- [x] 5.1 `docs/architecture.md` (preview pools: diagrams beside Markdown images), `AGENTS.md` §7 (merman: pure Rust, features off, only behind `fm_preview::mermaid`), `docs/development.md` manual check
- [x] 5.2 CHANGELOG `Added` line, devlog entry; `cargo test` for touched crates, clippy, fmt, `cargo xtask layers`, `cargo xtask brand-check`
- [ ] 5.3 Manual: open `it-architecture.md` and `ecommerce-landscape.md` in a Preview on Linux and Windows, both themes, zoom 50% to 400%, toggle the preference, Copy on a diagram, Find, source view, screen reader reads the diagram name
