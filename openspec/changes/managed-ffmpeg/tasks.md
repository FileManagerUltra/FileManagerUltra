## Implementation

- [x] Pin, verified streaming download, safe zip extraction, manifest and atomic install (`engine/ffmpeg_install.rs`) with tests: wrong hash, oversize, hostile members, replacement
- [x] `tools.ffmpeg.install` command, background progress, `Session` polling
- [x] `find_tool` searches the app-managed folder last; negative lookups are no longer cached
- [x] Preferences > Media ffmpeg section; missing-tool hints name it on Windows
- [x] Docs, changelog, `security` and `preview-pane` deltas
- [x] Stop bundling ffmpeg in the Windows installer (`release.yml`, `packager-config`)
- [ ] Real-app check on Windows (download, play, Update, Reinstall)
