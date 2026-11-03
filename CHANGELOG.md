# Changelog

All notable changes to FileManagerUltra are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Links in the Markdown preview are clickable. Web and mail links open externally; relative links such as `docs/install.md` open the folder in a Files pane and select the file.
- Markdown files can be viewed as source: the "Preview" toolbar button toggles between the rendered document and the highlighted text (also in the View menu).
- Preview source picker numbers the file panes.

### Changed

- The log filter bar is shown by default when previewing a log.
- The Diff toolbar shows a single "Diff" toggle for every file except Markdown, instead of a Preview/Diff pair whose Preview button did nothing.

### Removed

- macOS Intel (x64) release builds; Apple has deprecated the platform.

### Documentation

- macOS install notes: clear the quarantine flag on the downloaded `.dmg` (`xattr -d com.apple.quarantine`), as the app is not yet signed or notarized.
