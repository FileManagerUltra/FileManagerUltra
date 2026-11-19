## MODIFIED Requirements

### Requirement: Folders get a thumbnail grid
A Preview pane showing a directory SHALL offer a thumbnail grid (`preview.thumbs.toggle`): image files render a downscaled thumbnail, video files render their poster frame, PDF files render their first page (in original colours, within the PDF render bounds), and everything else renders a name tile. Encrypted PDFs that need a password and PDFs whose first page cannot render SHALL get a name tile. The grid SHALL come in three sizes (`preview.thumbs.size`: small 160 px, medium 256 px, large 512 px) and a per-tile hover label with the full name. Leaving the Preview pane kind or showing a non-directory body SHALL collapse the grid back to the renderer's default.

#### Scenario: A folder of images
- **WHEN** a Preview pane follows a Files pane into a folder of images and thumbnails are enabled
- **THEN** tiles appear for the images (and name tiles for files without a thumbnail), and toggling thumbnails off returns the directory card

#### Scenario: Size presets
- **WHEN** `preview.thumbs.size` runs with no params while the grid is visible
- **THEN** the preset advances small → medium → large → small and the visible tiles are re-requested at the new edge

#### Scenario: A folder of PDFs
- **WHEN** the thumbnail grid shows a local folder with PDFs
- **THEN** each PDF tile shows its first page, and a password-protected PDF shows a name tile without asking for a password
