## ADDED Requirements

### Requirement: Pictures are shown upright and can be turned
Raster pictures SHALL be shown the way their EXIF orientation says (rotation and mirroring), in the Preview pane, folder thumbnails, images inside Markdown, media cover art and Copy Image, and the size shown SHALL be the upright size. A picture's Preview toolbar SHALL offer Rotate left and Rotate right (`preview.image.rotate {direction: "left"|"right", pane?}`), which turn the shown picture a quarter turn at a time without changing the file. The turn SHALL belong to the file shown: another file starts upright, and returning to the same file in that pane keeps its turn. Fit SHALL fit the turned picture.

#### Scenario: Phone photo stored sideways
- **WHEN** a JPEG stored 4032×2268 with EXIF orientation 6 is previewed or thumbnailed
- **THEN** it is shown upright as 2268×4032

#### Scenario: Turning a picture
- **WHEN** the user clicks Rotate right on a wide picture
- **THEN** the picture is drawn a quarter turn clockwise, tall and fitted to the pane, and the file on disk is unchanged

#### Scenario: Next file
- **WHEN** the user turned a picture and selects another picture
- **THEN** the other picture is shown upright
