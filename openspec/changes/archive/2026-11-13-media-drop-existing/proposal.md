## Why

Dropping a file on the Media player always inserted it after the current item, even when it was already in the playlist. Users drag a file they see elsewhere to play it, and ended up with duplicates.

## What Changes

- A drop on the player skips files that are already in the playlist; when the first dropped file is one of them, its existing row is selected, scrolled into view and played.
- Dropping the item that is already playing does not restart it.
- Folder drops and drops on the playlist are unchanged.

## Capabilities

### Modified Capabilities
- `media-pane`: "Drops on the player play, drops on the playlist add".

## Impact

`engine::media_source_cmds` (the `media.drop` result step). No UI or command-parameter changes.
