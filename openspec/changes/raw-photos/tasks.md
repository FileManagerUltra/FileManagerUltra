## 1. Implementation

- [x] 1.1 `raw_prev`: bounded TIFF walk (32 directories, 1024 entries, SubIFD depth 3, loop guard), candidates from single-strip JPEG directories and JPEGInterchangeFormat, sensor data excluded, ranked by the JPEG frame size; IFD0 orientation
- [x] 1.2 Preview and thumbnails read by range; in-memory decode and Copy Image handle RAW bytes; TIFF signature sniffed as an image
- [x] 1.3 Tests: largest preview wins over sensor data (mutation-checked), orientation, file and memory reads agree, every truncation and byte flip of a RAW file, self-referencing and oversized directories, lossless JPEG refused, RAW thumbnails; checked on a real DNG and CR2
- [x] 1.4 CHANGELOG, devlog
- [ ] 1.5 Manual: browse a folder of RAW files on SMB/SFTP (range reads), check NEF/ARW samples
