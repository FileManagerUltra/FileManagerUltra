## Context

`fm-preview::markdown` builds a bounded `MdDoc` on the preview worker; `fm-ui::preview_markdown` paints it. Images are currently turned into `[Image: alt]` inline text and the spec forbids any resource load. Image previews already exist (decoders, dimension checks, textures) and `reqwest` (blocking, rustls) is already used by `engine` for update checks. Layering: `preview` (data) → `engine` (I/O, jobs) → `ui` (egui). The UI must never block or call backends.

## Goals / Non-Goals

**Goals:**
- Show local and remote Markdown images in the rendered view without blocking a frame.
- Keep hostile input safe: bounded bytes, dimensions, counts, time; no SSRF; no surprise network traffic.

**Non-Goals:**
- Inline (in-text-line) image layout; raw HTML `<img>`; image editing; disk caching of remote images; authentication for remote images; data: URLs.

## Decisions

1. **Block-level model.** The parser hoists images out of paragraphs/headings/cells into `MdBlock::Images(Vec<MdImage>)` placed after the text that contained them. `MdImage { alt, url, title, link: Option<String> }`; an image wrapped in a link keeps the link (badges). Consecutive images in one paragraph form one row that wraps. Alternative (true inline layout) rejected: egui text layout cannot mix images and wrapped text reliably. Table-cell and heading images keep the alt-text placeholder (no hoisting) to keep row heights simple. Image count is capped at 200 per document, beyond which placeholders stay.
2. **Resolution reuses link rules.** Relative paths are percent-decoded, `?query`/`#fragment` dropped, `.`/`..` resolved, and refused if they climb above the root; absolute paths, `file:`, `data:` and `//` are refused (placeholder). Shared helper with `preview.open_link`.
3. **Loading in engine lanes.** The UI requests images through a command (`preview.md_images.want {urls, pane}`) that schedules a job: local reads via the entry's `Vfs` on its lane (so SMB/SFTP-hosted Markdown works on the right lane); remote fetches on a dedicated small `net` worker group (2 workers). Results (decoded `RgbaImage` bounded in size, SVG rasterized by resvg) are cached per pane in an LRU (256 MiB decoded). The UI polls the cache and shows a placeholder until ready; arrival requests a repaint.
4. **Limits.** Local and remote: 8 MiB source, header check ≤ 100 megapixels before allocation (existing limit), decoder panics contained. Remote additionally: 10 s total timeout, ≤ 3 redirects (each re-validated), `http`/`https` only, URL ≤ 2048 chars, no credentials in URL, no cookies, fixed User-Agent without version/OS details, response `Content-Length` and streamed bytes both enforced.
5. **SSRF guard.** Before connecting (and on every redirect) the host is resolved and every address must be public: reject loopback, private, link-local, unique-local, CGNAT, multicast, unspecified and IPv4-mapped equivalents. The connection uses the validated addresses to avoid DNS rebinding (custom resolver on the `reqwest` client). Alternative (hostname deny-list) rejected as bypassable.
6. **Opt-in remote.** A non-secret `Prefs` boolean `preview_remote_images` (default `false`), shown in Preferences > Preview. Toggling re-renders open Markdown previews. When off, remote images show alt text with a "Remote image blocked" tooltip and never touch the network. A pref hand-edited to a non-boolean falls back to `false`.
7. **UI.** Images scale to the document width (never upscaled beyond natural size), respect preview magnification, use theme tokens for placeholder frame/text (no `Color32::` literals), and show alt text as hover tooltip. Linked images use the existing link-click path.

## Risks / Trade-offs

- [Remote fetch leaks IP/reading activity] → off by default, clear pref label, hostile-address guard.
- [Large/many images exhaust memory] → per-image and per-document caps, LRU cache with byte budget, pane-scoped cache dropped when the previewed file changes.
- [DNS rebinding / redirects to internal hosts] → validated custom resolver, per-hop revalidation.
- [Slow servers pile up jobs] → 2-worker `net` group, 10 s timeout, cache keyed by the document's mtime.
- [Hoisting changes text/image order] → images appear right after their paragraph; acceptable for README use.

## Migration Plan

No data migration. New pref defaults to off, so upgrading users see local images only. Rollback = revert the change; prefs with the unknown key are ignored by older builds.
