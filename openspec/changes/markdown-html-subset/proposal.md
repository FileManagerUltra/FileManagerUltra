## Why

READMEs on GitHub lean on a small HTML subset for centered headers, theme-aware screenshots (`<picture>`), sized images and image grids (`<table>`). The Markdown viewer shows that markup as literal text, so the project's own README looks broken in the Preview pane.

## What Changes

- The Markdown viewer interprets a small, closed set of HTML tags in HTML blocks and inline HTML: `p`, `div`, `center`, `h1`–`h6` (with `align`), `br`, `a`, `img` (with `width`), `picture`/`source` (dark-mode source chosen for the current theme), `code`, `strong`/`b`, `em`/`i`, and `table`/`tr`/`th`/`td`.
- Everything else is dropped as markup (its text content is kept); `script`, `style`, comments and unknown attributes never act.
- Images from HTML go through the same loader and limits as Markdown images.
- Centered blocks render centered; table cells may hold images.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `preview-pane`: Markdown rendering interprets an HTML subset.
- `security`: HTML in Markdown stays inert and bounded.

## Impact

- `fm-preview`: new `html` module (tokenizer + builder), `MdBlock::Centered`, `MdImage` gains `dark_url` and `width`, `Table` gains `headerless`.
- `fm-ui`: renders centered blocks, theme-selected images, sized images, headerless tables.
- `fm-engine`: Find/Copy walk the new variant; no new commands.
- No new dependencies.
