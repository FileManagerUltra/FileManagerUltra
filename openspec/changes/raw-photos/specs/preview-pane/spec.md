## ADDED Requirements

### Requirement: Camera RAW photos
Camera RAW photos with a TIFF structure (DNG, CR2, NEF, NRW, ARW, SR2, SRF, ORF, PEF, SRW, ERF, 3FR), recognised by their content, SHALL be previewed, thumbnailed and copied through the largest embedded baseline or progressive JPEG they contain, turned by the RAW file's orientation. Sensor data SHALL never be decoded. The preview SHALL read only the TIFF directories and the chosen JPEG, not the whole file, and SHALL bound the walk (directories visited, entries per directory, SubIFD depth, loops) and check every offset. The info line SHALL name the format ("DNG camera preview"). A TIFF file without such a JPEG SHALL show a card saying it has no embedded picture.

#### Scenario: Canon CR2
- **WHEN** a CR2 with a full-size 5184×3456 embedded JPEG and a lossless sensor strip is previewed
- **THEN** the 5184×3456 picture is shown (downscaled for the pane), labelled "Canon CR2 camera preview", and the sensor strip is not read

#### Scenario: Damaged RAW file
- **WHEN** a RAW file is truncated anywhere or has corrupted directory entries, offsets pointing outside the file or a directory that points at itself
- **THEN** the preview shows the picture or the no-picture card, never crashes and never hangs
