## 1. Model and parser (fm-preview)

- [x] 1.1 `html` module: tokenizer (tags, text, entities, comments, script/style) with caps and fuzz-style tests
- [x] 1.2 Builder to inlines/images/blocks/tables; `MdBlock::Centered`, `MdImage.dark_url`/`width`, `Table.headerless`
- [x] 1.3 Hook HTML blocks and inline HTML in `markdown::parse`; update `plain_text`
- [x] 1.4 Parser tests incl. the README header, picture, table; hostile input

## 2. Engine

- [x] 2.1 Find/Copy handle `Centered`

## 3. UI (fm-ui)

- [x] 3.1 Render centered blocks, theme-selected and sized images, headerless tables
- [x] 3.2 Request only the images for the current theme; tests in dark and light incl. e2e for the README

## 4. Docs and wrap-up

- [x] 4.1 `CHANGELOG.md`, `docs/architecture.md`, `log/devlog.md`
- [x] 4.2 test, clippy `-D warnings`, `cargo xtask layers`, `brand-check`, `fmt --check`
