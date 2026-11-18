## Why

Folder and file fields are plain text boxes, so users must type or paste paths (including SMB/SFTP uris). The sync window needs a way to choose its two folders, and other dialogs need the same for files.

## What Changes

- One in-app picker window (egui, no native dialog, no new dependency) that browses any backend through the existing directory cache and returns a folder or a file.
- `picker.*` commands: open (purpose, kind, start), navigate, select, accept, cancel. The result is delivered by purpose: sync folder purposes are applied to the profile by the engine; other purposes are left in `Session::picked` for the dialog that asked.
- First users: the two sync folder inputs (Browse button) and the subtitle and playlist path dialogs.

## Capabilities

### New Capabilities
- `path-picker`: choosing a folder or file on any backend from a dialog.

### Modified Capabilities
- `file-sync`: folder inputs get a Browse button.
