# Design

1. **Shortcuts.** `search.open` takes Ctrl+F; the app's existing Preview rerouting now
   maps it to `preview.find.open` in a Preview pane. `search.open_everywhere` (Ctrl+Shift+F)
   uses `VfsUri::root_file()`, which is `/` on Linux/macOS and the This PC drive list on
   Windows, so the walker needs no multi-root support. `/proc /sys /dev /run` (Linux) and
   `/System/Volumes /dev /private/var/vm /Volumes/Recovery` (macOS) are never descended.
2. **Type-to-filter.** The listing turns `Event::Text` into `view.filter {text, focus:
   true}` (appending to an open filter) when the listing has the keyboard. A leading space
   does not start a filter. The filter field puts its cursor at the end when focused.
3. **Background searches.** `Search` gains `minimized`. The window is not drawn while
   minimised; the run keeps going. The jobs panel lists the search with status and
   Show/Stop buttons, and the status-bar job text includes it. Closing the window still
   stops the search.
4. **History.** `Prefs::search_history` (newest first, max 20, deduplicated by all
   fields, each field capped as the search params are). Starting a search records it.
   Prefs never hold secrets; an `sftp://` root is stored without credentials (URIs
   already refuse embedded passwords).
5. **Speed.** For `file://` roots the walk lists with `std::fs::read_dir` and
   `DirEntry::file_type()` (no `stat`), and builds an `Entry` with a `stat` only for
   matches. A small pool (available parallelism, at most 8) takes folders from a shared
   stack; results are appended per folder, so their order follows completion and the
   window shows them as they come. Content matching lower-cases each chunk and uses
   `memchr::memmem`. SFTP keeps the `Vfs` walk on one thread.
