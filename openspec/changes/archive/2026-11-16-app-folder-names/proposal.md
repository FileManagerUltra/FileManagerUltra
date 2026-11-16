## Why

On Windows the app's settings folder was `AppData\Roaming\filemanagerultra` (the lowercase slug), while its folder in `AppData\Local` showed as `FileManagerUltra`: the installer creates that one with the product name, and Windows keeps the capitals a folder was created with, so the app's lowercase cache and data paths landed inside it. The same app's folders should always carry the same name.

## What Changes

- New brand constant `APP_DIR_NAME`: the product name on Windows (matching the installer's `%LOCALAPPDATA%\FileManagerUltra`), the slug elsewhere (Linux XDG convention; macOS keeps its existing folders).
- One helper, `prefs::project_dirs()`, for every config, cache and data path (preferences, thumbnails, managed ffmpeg).
- At startup on Windows, an existing folder with other capitals (`filemanagerultra`) is renamed to the exact name before anything is read. Its contents stay; a failed rename changes nothing (Windows paths are case-insensitive, so the app keeps working either way).

## Capabilities

### Modified Capabilities
- `branding`: new requirement "App folders carry one name".

## Impact

`fm-brand` (`APP_DIR_NAME`), `fm-engine::prefs` (`project_dirs`, `fix_app_dir_case`), `ffmpeg_install::tools_dir`, `fm-app` startup, docs (development, architecture). No dependency changes. Linux and macOS paths are unchanged.
