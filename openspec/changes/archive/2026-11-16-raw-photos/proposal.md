## Why

Camera RAW files (DNG, CR2, NEF, ARW…) showed as a hex dump and had no thumbnails. Every one of them carries a JPEG preview made by the camera or converter, which is what other file managers show.

## What Changes

- RAW photos with a TIFF structure (DNG, CR2, NEF/NRW, ARW/SR2/SRF, ORF, PEF, SRW, ERF, 3FR) are previewed and thumbnailed through their largest embedded baseline or progressive JPEG, turned by the RAW file's orientation. Sensor data (lossless JPEG, CFA, JPEG XL previews) is never decoded.
- Only the TIFF directories and the chosen JPEG are read (range reads), never the whole RAW file. Copy Image copies the embedded picture at full size.
- Detection is by content (TIFF signature), so existing file-type settings need no change; a TIFF without an embedded JPEG shows a card saying so instead of a hex dump.

## Capabilities

### Modified Capabilities
- `preview-pane`: new requirement "Camera RAW photos".

## Impact

New `fm-preview::raw_prev` (bounded TIFF walker, JPEG frame check), hooks in `image_prev` (in-memory decode and Copy Image), `sniff` (TIFF signature), `lib` dispatch and `thumbs` (RAW extensions, range-read path). No new dependency: full RAW decoders in Rust are LGPL and slow.
