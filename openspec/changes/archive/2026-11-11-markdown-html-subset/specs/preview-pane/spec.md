## ADDED Requirements

### Requirement: Markdown HTML subset
The Markdown viewer SHALL interpret a closed subset of HTML in HTML blocks and inline HTML: `p`, `div`, `center`, `h1`–`h6`, `br`, `a`, `img`, `picture`, `source`, `code`, `strong`, `b`, `em`, `i`, `table`, `tr`, `th`, `td`, plus `hr`, `ul`, `ol`, `li`, `pre` and `blockquote` as plain paragraph breaks. Elements with `align="center"` SHALL render centered. `img` MAY carry `width` in pixels. A `picture` SHALL show its `source` whose `media` is `(prefers-color-scheme: dark)` when the theme is dark and its `img` otherwise. Tables SHALL render with images and text in cells. Any other tag SHALL be ignored while keeping its text, and `script` and `style` content SHALL never be shown or acted on.

#### Scenario: Theme-aware picture
- **WHEN** Markdown contains `<picture><source media="(prefers-color-scheme: dark)" srcset="d.webp"><img src="l.webp"></picture>`
- **THEN** the dark theme shows `d.webp` and the light theme shows `l.webp`, never both

#### Scenario: Centered header and links
- **WHEN** Markdown contains `<h1 align="center">Name</h1>` and a centered `<p>` of links
- **THEN** they render centered and the links are clickable like Markdown links

#### Scenario: Image grid
- **WHEN** Markdown contains a `<table>` whose cells hold pictures
- **THEN** the cells render as a grid without a header row and images load as in Markdown

#### Scenario: Unknown markup
- **WHEN** Markdown contains `<script>alert(1)</script>` or `<blink>hi</blink>`
- **THEN** the script is not shown and `hi` is shown as plain text
