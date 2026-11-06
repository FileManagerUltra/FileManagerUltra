## Why

Markdown previews show images as inert `[Image: alt]` text, so READMEs, docs and notes lose their screenshots, diagrams and badges. Previewing a project folder is a core use of the Preview pane, and images are the biggest gap in the rendered Markdown view.

## What Changes

- Rendered Markdown shows images: relative paths (resolved against the previewed file, same rules and root as Markdown links) and `http`/`https` URLs.
- Local images load through the engine's job lanes and the existing image decoders (png, jpeg, gif, webp, bmp, ico; svg is rasterized with resvg, without text or external resources), with the same dimension and size limits as image previews.
- Remote images are **off by default**, behind a new Preferences > Preview toggle, because fetching reveals the user's IP and the file being read. While off, remote images keep the alt-text placeholder with a "remote image blocked" hint.
- Remote fetches are bounded (size, time, redirects), `http`/`https` only, and refuse loopback, private, link-local and other non-public addresses.
- Images that are loading, blocked, missing or undecodable show the alt-text placeholder; they never produce a crash or a failed preview.
- Images keep working with zoom, Copy (alt text), and render in both themes.
- **BREAKING (behaviour)**: the rule "no link/image resource SHALL be fetched" is relaxed for images only, under the limits above. Links still never load anything.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `preview-pane`: Markdown rendering shows local and (opt-in) remote images; adds the preference.
- `security`: the "previews are untrusted-input safe" requirement gains the bounded local and remote image-loading rules.

## Impact

- `fm-preview` `markdown.rs`: new `MdBlock::Images` model; image parsing and caps.
- `fm-engine`: new `preview.md_images.want` loading command/job (local via `Vfs`, remote via the existing `reqwest` blocking client), a texture/bytes cache, a `Prefs` flag (non-secret boolean).
- `fm-ui` `preview_markdown.rs`: render images, placeholders, click-through for linked images; snapshot tests in dark and light.
- `docs/architecture.md`, `CHANGELOG.md`, `log/devlog.md`.
- New dependency: `resvg` (pure Rust, default features off) in `fm-preview` for SVG.
