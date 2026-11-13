## ADDED Requirements

### Requirement: SVG preview
An SVG file SHALL be rasterized on the preview worker, never the UI thread, at the larger of its declared size and 2048 px on the longest edge, within the existing image edge and pixel limits. The picture SHALL be drawn over a background chosen in the header: Checkered (default, from theme colours), White, Black or 50% grey; the choice SHALL be saved in preferences, apply to every SVG preview, and stay reachable when narrow toolbars fold into their menu. The header Preview toggle (`preview.toggle_source`) SHALL switch between the picture and the file's highlighted XML source, with Pretty and wrap available in the source view; selecting another file SHALL return to the picture. Files matching no filename pattern whose start contains an `<svg` element SHALL be previewed as SVG.

Text SHALL be drawn with the installed system fonts, loaded once off the UI thread; fonts are never downloaded. Generic families (serif, sans-serif, monospace) SHALL map to installed fonts, text without a font-family SHALL be serif, and a named font that is not installed SHALL fall back to an installed font of the same kind (monospace for names such as Consolas or Courier, serif for names such as Times or Georgia, otherwise sans-serif, e.g. for Helvetica or Segoe UI).

Rendering SHALL NOT load external files, URLs or embedded resources (including `<image>` references to local paths and `data:` URLs), nor run scripts or animation. An SVG that cannot be parsed, has no usable size or exceeds limits SHALL show the reason and the source view.

#### Scenario: Background choice
- **WHEN** the user picks Black from the SVG background dropdown (or runs `preview.svg.background` with `{value: "black"}`)
- **THEN** transparent areas of every SVG picture show black, and the choice survives a restart

#### Scenario: Switch to source and back
- **WHEN** the user turns Preview off on a rendered SVG
- **THEN** the highlighted XML source is shown without reading the file again, and turning Preview on shows the picture

#### Scenario: Text in a diagram
- **WHEN** an SVG labels its boxes with `font-family="Helvetica"` on a system without Helvetica
- **THEN** the labels are drawn in an installed sans-serif font, keeping weight where the font has it

#### Scenario: Broken or hostile SVG
- **WHEN** the SVG is malformed, declares a size of `1e30`, or references `/etc/passwd` or an `http` URL
- **THEN** the preview shows an error with the source (or renders without the reference), no external resource is read, and the UI stays responsive

## MODIFIED Requirements

### Requirement: Configurable preview filename filters
Preferences > Preview SHALL show, for each of Logs, Images, Video, Audio, PDF, Markdown, SVG, JSON, YAML, TOML, XML, HTML, CSV and Text, an entry in a type list (with its pattern count) beside a list of the selected type's persisted filename patterns, each list with an icon toolbar (types: move up/down; patterns: add, remove, edit). Each pattern is one case-insensitive full-filename glob (`*`, `?`, character classes or an exact name); a pattern is edited in place (toolbar, a second click or F2; Enter or leaving the field saves, Escape cancels without closing Preferences), and Delete removes the focused one. A short explanation of the pattern syntax is shown below the lists. Groups SHALL be checked from top to bottom and the first match SHALL win; users SHALL be able to move types up and down to change priority. A group with no rows disables filename matching for that type, not content detection. HTML and CSV SHALL use existing text rendering, not execute markup or introduce table rendering.

Defaults SHALL cover supported extensions and common system logs (`syslog`, `messages`, `secure`, `maillog`, `cron`, `debug`, `dmesg`), including gzip and numeric/date rotations. Rotation patterns SHALL match a numeric-starting suffix using globs, not an exact rotation-name parser. SVG defaults to `*.svg` and comes before XML. Settings are stored as an ordered list of `{type, patterns}` groups; the older one-string-per-type form SHALL migrate automatically and missing or unreadable groups SHALL use defaults. Stored settings without an SVG group SHALL gain one directly above XML, and the default `*.svg` pattern SHALL be removed from XML; other user patterns SHALL be kept. A restore action SHALL reset all groups and order.

#### Scenario: Custom associations apply immediately
- **WHEN** the user removes the Logs rows and adds `*.trace` and `server-output`
- **THEN** matching files use log preview and removed `.log` associations are not reapplied by filename fallback
- **AND** existing single and multi-file previews refresh, and directory thumbnails follow configured image/video associations
- **AND** all existing read, decoder, memory and time limits remain enforced

#### Scenario: Overlap, invalid input and reset
- **WHEN** the same filename matches multiple rows
- **THEN** the first displayed group wins, and moving a group up or down changes the result
- **WHEN** a row is empty, longer than 256 bytes, contains path separators, list separators (`;` `,` `|`) or control characters, or a group exceeds 64 rows or 1024 bytes in total
- **THEN** it is rejected visibly without partially changing saved preferences
- **WHEN** the user restores default file filters
- **THEN** every category and its editable field returns to its default

#### Scenario: Older settings gain the SVG type
- **WHEN** preferences saved before the SVG type existed are loaded
- **THEN** SVG appears directly above XML with `*.svg`, XML no longer lists `*.svg`, and every other pattern and the order of the other types are unchanged

### Requirement: Supported previews
The preview SHALL render: text and source with syntax highlighting; JSON, YAML, TOML and XML as raw highlighted text with an optional pretty-printed view; Markdown; png, jpeg, gif (animated), webp, bmp and ico images; SVG drawings rendered as a picture with a switch to their source; audio and PDF metadata; video with playback; a directory summary; and a hex dump for everything else.

Directory and file-information summaries SHALL use the dialog style guide's
proportional text, neutral section headings, muted labels, aligned wrapping values
and flat spacing, without changing document/code/log fonts. Directory summaries
SHALL separate subfolder totals from the overview. When the previewed directory
is on a listed device, Preview SHALL show its name, mount location, filesystem,
capacity, free space and a used-space indicator (when capacity is known), using
only cached device data. Nested mounts SHALL use the closest containing volume.
Unavailable capacity and unsupported removal explanations SHALL remain explicit.
Thumbnail mode SHALL retain volume information in an expandable section.
Read-only summary rows SHALL use compact spacing, not dialog-control heights.
Subfolder totals SHALL use a compact Name / Size / Files table with single-line
truncated folder names and full-name hover text.

#### Scenario: Preview a mounted directory
- **WHEN** a mounted device or a directory inside it is previewed
- **THEN** its volume information appears above the folder summary
- **AND** long values wrap within narrow panes and details update with the cached mount list
- **AND** directories outside listed volumes retain their normal summary without a volume section

#### Scenario: Structured files open raw
- **WHEN** a minified `.json` file is previewed
- **THEN** it is shown exactly as stored, and a Pretty button is available in the preview header

#### Scenario: Pretty on demand
- **WHEN** the user presses Pretty (or runs `preview.toggle_pretty`)
- **THEN** the file is shown re-indented, pressing it again returns to the raw text, and selecting another file resets to raw

#### Scenario: Invalid structured file
- **WHEN** a `.json` file is not valid JSON
- **THEN** it is shown raw, and Pretty is disabled with a "not valid JSON" tooltip

#### Scenario: SVG opens as a picture
- **WHEN** an `.svg` file is previewed
- **THEN** it is shown rendered, with Fit and 100% controls and a Preview toggle in the header

### Requirement: Markdown images
Rendered Markdown SHALL show images written as `![alt](url "title")`. Images in paragraphs
SHALL be placed as block-level rows directly after the paragraph text, consecutive images
forming one wrapping row; an image inside a link SHALL keep that link. Images in headings
and table cells SHALL remain `[Image: alt]` placeholders. At most 200 images per document
SHALL be rendered; further ones stay placeholders.

A relative URL SHALL be resolved against the previewed file with the same rules as
Markdown links (percent-decoded, `?query` and `#fragment` ignored, `.` and `..` resolved,
refused above the root) and read through the file's own backend. Absolute paths, `file:`,
`data:`, `//` URLs and other schemes SHALL NOT be loaded. Supported formats are those
of image previews. SVG images are rasterized like SVG previews: text is drawn with installed system fonts, and embedded or external resources are not loaded.

`http` and `https` images SHALL be loaded only when Preferences > Preview > "Load remote
images in Markdown" is enabled; it SHALL default to off. While off, remote images SHALL
show their alt text with a "Remote image blocked" hover hint and SHALL cause no network
access.

Images SHALL load off the UI thread; until ready, and when blocked, missing, oversized,
undecodable or failed, the alt-text placeholder SHALL be shown and the document SHALL
remain usable. Images SHALL scale down to the document width, never up past natural size,
follow preview magnification, show alt text/title on hover, and render correctly in
dark and light themes. Copy SHALL treat an image as its alt text; Find does not search alt text.

#### Scenario: Local relative image
- **WHEN** `README.md` contains `![logo](img/logo.png)` and `img/logo.png` exists
- **THEN** the rendered view shows the image below its paragraph without blocking a frame

#### Scenario: Missing or escaping path
- **WHEN** an image points at a missing file or at `../../../etc/x.png`
- **THEN** the alt-text placeholder is shown, nothing outside the root is read, and the preview stays usable

#### Scenario: Remote images off by default
- **WHEN** Markdown contains `![badge](https://example.com/b.svg)` and the preference is off
- **THEN** no connection is made and the placeholder shows "Remote image blocked" on hover

#### Scenario: Remote images enabled
- **WHEN** the preference is enabled and the URL serves a valid image within limits
- **THEN** the image appears; toggling the preference re-renders open Markdown previews

#### Scenario: Linked badge
- **WHEN** an image is wrapped in a link and clicked
- **THEN** the link is followed by the normal Markdown link rules
