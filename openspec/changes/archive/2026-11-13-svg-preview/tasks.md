## 1. Model and routing

- [x] 1.1 `fm-preview`: add `Kind::Svg`; sniff unmatched files with an `<svg` element near the start as SVG; tests
- [x] 1.2 `fm-preview`: SVG preview model holding the raster and the source text; rasterize at max(declared size, 2048 px) within image limits; tests for size, hostile input and external references
- [x] 1.3 `fm-engine`: `PreviewType::Svg` (label "SVG", default `*.svg`, before XML; `*.svg` removed from XML defaults); prefs migration above XML; tests incl. old settings and restore defaults
- [x] 1.4 `fm-engine`: build SVG previews on the preview worker; `preview.toggle_source` enabled for SVG; source flag resets on selection change; fuzz and graceful-failure tests

## 2. UI

- [x] 2.1 `fm-ui`: render the SVG picture with Fit / 100% and pane zoom over a token-based checkerboard
- [x] 2.2 `fm-ui`: header Preview toggle for SVG; source view with XML highlighting, Pretty and wrap; error line above the source on failure
- [x] 2.3 `fm-ui`: File types list shows SVG; snapshots for rendered, source and error views in dark and light

## 3. Wrap-up

- [x] 3.1 Settle design Open Question 1 (text rendering) and, if approved, enable `resvg` text with system fonts plus a test (approved; stand-in fonts for absent names; `<image>` references refused)
- [x] 3.2 CHANGELOG, `docs/development.md` manual checks, devlog; clippy, fmt, layers, brand-check green

## 4. Follow-up

- [x] 4.1 Background choice for SVG pictures (Checkered default, White, Black, 50% grey): saved pref `svg_background`, command `preview.svg.background`, header dropdown (submenu inside the narrow toolbar menu), tests and snapshots
