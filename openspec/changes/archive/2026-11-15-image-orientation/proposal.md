## Why

Phone cameras store most photos sideways and add an EXIF note saying how to turn them. The preview and folder thumbnails ignored that note, so upright photos showed lying on their side, and there was no way to turn a picture that is simply taken at the wrong angle.

## What Changes

- Pictures follow their EXIF orientation everywhere they are decoded: the Preview pane, folder thumbnails, images inside Markdown, media cover art and Copy Image. The size shown is the upright size. The thumbnail cache version is bumped so old sideways thumbnails are redrawn.
- The Preview toolbar of a picture gets Rotate left and Rotate right buttons (`preview.image.rotate`). They turn only the view, never the file; the turn belongs to the file shown and the next file starts upright again (returning to the same file keeps it).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `preview-pane`: new requirement "Pictures are shown upright and can be turned".

## Impact

`fm-preview::image_prev` (orientation on decode), `fm-preview::thumbs` (cache version), new `engine::preview_rotate_cmds` with per-file turn state on `PreviewSlot`, `fm-ui::preview` (toolbar and rotated drawing), `fm-ui::media_icons` (painted rotate icons).
