## 1. Implementation

- [x] 1.1 Apply EXIF orientation in `decode_with` and `decode_original`; report the upright size; bump the thumbnail cache version
- [x] 1.2 `preview.image.rotate {direction, pane?}` with the turn kept per file on `PreviewSlot`; enabled only while a picture is shown
- [x] 1.3 Toolbar Rotate left/right (painted icons) and drawing a turned picture with swapped size
- [x] 1.4 Tests: EXIF 1/3/6/8 and Copy Image (mutation-checked), command turns/reset/bad params, UI buttons turn the drawn picture; checked by eye on a real phone photo
- [x] 1.5 CHANGELOG, devlog
