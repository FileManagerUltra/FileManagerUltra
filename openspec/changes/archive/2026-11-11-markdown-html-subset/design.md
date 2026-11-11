## Context

`pulldown-cmark` reports HTML as `Html`/`InlineHtml` events; today they become literal text. Raw HTML blocks contain no Markdown, so the whole block can be handled by a separate tolerant tokenizer.

## Goals / Non-Goals

**Goals:** render the README's markup (centered header/links, `<picture>` screenshots, `<table>` grids of pictures); stay inert and bounded.

**Non-Goals:** CSS, `style`/`class`/`id`, `<svg>`/`<iframe>`/`<video>`, forms, lists or code blocks inside HTML, a full HTML5 parser, Markdown inside HTML blocks.

## Decisions

1. **Own tokenizer, no new crate.** `fm-preview::html` scans tags, text and entities (`&amp; &lt; &gt; &quot; &apos; &nbsp;` and numeric), in one linear pass. Input per HTML block is capped at 256 KiB, nesting at 32, attributes ignored except `src srcset alt width href align media title`. `script`/`style` content is dropped; comments are skipped; unknown tags are transparent.
2. **Builder maps onto the existing model.** Inline runs become `Inline`s (bold/italic/code/link flags), `img`/`picture` become `MdImage`s (so loading, limits, caps and the opt-in remote rule are unchanged), `p`/`h*`/`div` become `Para`/`Heading`, `table` becomes `Table` (headerless unless `th` is used), `align="center"` wraps blocks in the new `MdBlock::Centered`.
3. **Theme-aware `picture`.** `MdImage.dark_url` holds the `<source media="(prefers-color-scheme: dark)">` target; the UI uses it when the theme is dark and the `img` otherwise, and asks the engine only for the URL it shows.
4. **Sizes.** `width` (pixels, 1–4096) sets the display width, still limited to the column; magnification applies.
5. **Inline HTML in Markdown** (`<br>`, `<kbd>`, `<b>`) is run through the same tokenizer so it renders instead of showing literally.

## Risks / Trade-offs

- A tolerant tokenizer misreads exotic HTML → worst case text is shown; fuzz tests assert no panic and bounded output.
- Tables of pictures show both themes' images stacked when no `picture` is used; with `picture` only one is shown.
