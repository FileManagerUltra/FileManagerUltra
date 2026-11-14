## 1. Implementation

- [x] 1.1 `fm-brand::APP_DIR_NAME` (product name on Windows, slug elsewhere)
- [x] 1.2 `prefs::project_dirs()` used by config, cache and managed-ffmpeg paths
- [x] 1.3 `prefs::fix_app_dir_case()` at startup (Windows only, best effort, keeps contents)
- [x] 1.4 Tests: one name across all folders; case-only rename keeps files; exact name, absent folder and same-named file untouched; no-op off Windows
- [x] 1.5 Docs (development, architecture), CHANGELOG, devlog
- [ ] 1.6 Manual (Windows): upgrade with an existing `Roaming\filemanagerultra`; settings kept, folder shows `FileManagerUltra`
