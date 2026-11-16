## ADDED Requirements

### Requirement: App folders carry one name
The application's own config, cache and data folders SHALL all be named by one brand constant, `APP_DIR_NAME`: the product name on Windows, the same as the folder the installer creates in `%LOCALAPPDATA%`, and the slug on other platforms. Every such path SHALL come from one helper. On Windows, at startup and before anything is read, a folder whose name differs from `APP_DIR_NAME` only in capitals SHALL be renamed to it, keeping its contents; when the rename fails nothing else changes.

#### Scenario: Upgrading on Windows
- **WHEN** a user whose settings are in `AppData\Roaming\filemanagerultra` starts the new version
- **THEN** the folder is now `AppData\Roaming\FileManagerUltra` with the same settings, matching `AppData\Local\FileManagerUltra`

#### Scenario: Linux
- **WHEN** the application starts on Linux
- **THEN** its folders stay `~/.config/filemanagerultra` and `~/.cache/filemanagerultra`, and nothing is renamed
