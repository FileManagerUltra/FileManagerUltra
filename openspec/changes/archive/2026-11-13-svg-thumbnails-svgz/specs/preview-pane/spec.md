## MODIFIED Requirements

### Requirement: SVG preview
An SVG file SHALL be rasterized on the preview worker, never the UI thread, at the larger of its declared size and 2048 px on the longest edge, within the existing image edge and pixel limits. The picture SHALL be drawn over a background chosen in the header: Checkered (default, from theme colours), White, Black or 50% grey; the choice SHALL be saved in preferences, apply to every SVG preview, and stay reachable when narrow toolbars fold into their menu. The header Preview toggle (`preview.toggle_source`) SHALL switch between the picture and the file's highlighted XML source, with Pretty and wrap available in the source view; selecting another file SHALL return to the picture. Files matching no filename pattern whose start contains an `<svg` element SHALL be previewed as SVG. Gzip-compressed SVG (`.svgz`, or gzip data matched by an SVG pattern) SHALL be decompressed to at most 16 MiB before parsing; larger output SHALL be refused with a reason, never decoded further. SVG and SVGZ files matched by the SVG type SHALL get thumbnails in folder thumbnail grids, drawn by the same renderer with the same rules, within the thumbnail edge and cached like image thumbnails.

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

#### Scenario: Compressed SVG
- **WHEN** a `.svgz` file is previewed or shown in a thumbnail grid
- **THEN** it is drawn like the same `.svg`, and the source view shows the decompressed XML
- **WHEN** a `.svgz` expands beyond 16 MiB
- **THEN** decompression stops at the cap and the preview reports that the SVG is too large

#### Scenario: SVG thumbnails
- **WHEN** a folder with `.svg` files is shown as thumbnails
- **THEN** each SVG shows its drawing, and moving `*.svg` out of the SVG type removes them

### Requirement: Configurable preview filename filters
Preferences > Preview SHALL show, for each of Logs, Images, Video, Audio, PDF, Markdown, SVG, JSON, YAML, TOML, XML, HTML, CSV and Text, an entry in a type list (with its pattern count) beside a list of the selected type's persisted filename patterns, each list with an icon toolbar (types: move up/down; patterns: add, remove, edit). Each pattern is one case-insensitive full-filename glob (`*`, `?`, character classes or an exact name); a pattern is edited in place (toolbar, a second click or F2; Enter or leaving the field saves, Escape cancels without closing Preferences), Delete removes the focused one, and Up and Down move the selection in either list while it has keyboard focus. A short explanation of the pattern syntax is shown below the lists. Groups SHALL be checked from top to bottom and the first match SHALL win; users SHALL be able to move types up and down to change priority. A group with no rows disables filename matching for that type, not content detection. HTML and CSV SHALL use existing text rendering, not execute markup or introduce table rendering.

Defaults SHALL cover supported extensions and common system logs (`syslog`, `messages`, `secure`, `maillog`, `cron`, `debug`, `dmesg`), including gzip and numeric/date rotations. Rotation patterns SHALL match a numeric-starting suffix using globs, not an exact rotation-name parser. SVG defaults to `*.svg` and `*.svgz` and comes before XML. Settings are stored as an ordered list of `{type, patterns}` groups; the older one-string-per-type form SHALL migrate automatically and missing or unreadable groups SHALL use defaults. Stored settings without an SVG group SHALL gain one directly above XML, and the default `*.svg` pattern SHALL be removed from XML; other user patterns SHALL be kept. A restore action SHALL reset all groups and order.

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
