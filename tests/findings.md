# Findings: Synchronize Folders (0.3.0 build)

Found while capturing website screenshots on 2026-11-18. Build: the working tree of `main` at
`4f203a1` plus the uncommitted 0.3.0 release changes, `cargo build --release -p fm-app`, run under
Xvfb on Linux (local folders under `/dev/shm`, demo SFTP server from the site repo's
`tools/demo_sftp_server.py`).

## 1. After "Start syncing", the sync still reads "Confirm to start syncing"

**Severity:** visible in normal use; behaviour is correct, state/labels are wrong.

**Steps**
1. File ▸ Synchronize Folders, Advanced, two local folders, goal *Copy new and changed files*.
2. Tick *Keep in sync*: it compares and shows *Cancel / Minimize to Activity panel / Start syncing*.
3. Press *Start syncing*: the changes are applied ("Last run: 21 copied") and the button becomes
   *Stop syncing*, but *Cancel* stays next to it.
4. Press *Minimize to Activity panel* and open the Activity panel.

**Expected:** "Keeping in sync … Up to date" (or similar), with Show / Stop syncing.

**Actual:** the Activity card and the status bar say, in the attention (red) colour,
`Confirm to start syncing (0 change(s))`, and the card shows a *Cancel* button. Keeping in sync
does work: a file added to the source afterwards was copied within seconds. While that follow-up
run copies, the card shows **two** *Cancel* buttons (one for the run, one for the stale confirm).

**Cause (from reading the code):** `watch_confirm()` in `crates/engine/src/sync_cmds.rs` calls
`sync_watch(&id, true)` and `sync_apply_confirmed(&id)` but never sets `arming = Arming::Off`;
`sync_watch` only resets `arming` when `on == false`. So `arming` stays `Confirm`, and
`line_for()` in `crates/ui/src/sync.rs` checks `arming == Confirm` before anything else, while
`activity()` keeps listing it with `confirming: true` (hence the extra Cancel). Setting
`st.arming = Arming::Off` once the confirm succeeds should fix all three symptoms. Worth a test:
after `sync.watch.confirm`, `arming` is `Off` and `watching()` is true.

## 2. Comparing with a typed SFTP folder of an unconnected server fails with a raw error

**Severity:** confusing; likely common (people paste server paths).

**Steps**
1. Fresh start, no SFTP connection open and no saved servers.
2. Synchronize Folders ▸ Advanced, left `/…/Photos`, right typed as
   `sftp://demo@127.0.0.1:2222/backups/Photos` (server reachable, folder does not exist yet).
3. Press Compare.

**Expected:** the sign-in dialog for that server (as Browse… does for a saved server, and as
bookmarks do since 0.2.4), then the comparison.

**Actual:** nothing happens in the sync window (the list still says "Choose two folders and press
Compare"), and the Activity panel shows `Error: unsupported: no backend registered for this
location`. The 0.2.4 changelog fixed this same message for bookmarks on unconnected SMB shares;
the sync compare path seems to need the same "connect first" step. The sync window should also
show the error itself, not only the Activity panel.

## 3. Browse… on an unconnected server hides the sign-in dialog behind the chooser

**Severity:** looks like a hang.

**Steps:** with the right folder set to `sftp://demo@127.0.0.1:2222/backups/Photos` (server not
connected), press that folder's Browse….

**Expected:** the Connect to Server dialog in front (or the chooser waiting for it).

**Actual:** the chooser opens on top, centered, spinning "Reading folder…" forever. The Connect to
Server dialog (host, port and user already filled in) is open but entirely behind the chooser;
only a sliver of its border shows. Dragging the chooser aside reveals it. Under Xvfb there is no
window manager, but both are egui windows in the same viewport, so the stacking order is the app's.

## 4. Escape with an action menu open closes the whole sync window (Advanced view)

**Severity:** loses the unsaved comparison and settings (see 7).

**Steps:** Advanced view, compare, click an item's action icon so its menu opens, press Escape.

**Expected:** only the menu closes (the comment above the Escape handling in
`crates/ui/src/sync.rs` says "Escape closes only the topmost layer").

**Actual:** the menu and the Synchronize Folders window both close. In the large Changes window
the same keys behave correctly (first Escape closes the menu, second the window).

**Probable cause (not verified):** in the normal view the table, and its menu, is drawn inside the
window before `esc` is computed, and egui closes the popup on Escape while drawing it, so
`Popup::is_any_open` is already false at the check. The Changes window is drawn after the check,
which is why it works there.

## 5. One missing field in `[window_geometry]` resets all of prefs.toml

**Severity:** low (hand-edited prefs), but silent data loss.

A `prefs.toml` with `check_updates = false`, `theme = "dark"` and
`[window_geometry] size = [1440.0, 900.0]` (no `position`, no `scale_factor`) was rewritten on
start with every default: `check_updates = true`, `theme = "system"`, size 1024×700. Expected: a
missing `scale_factor` defaults to 1.0 (or only the geometry is dropped) and the other settings
are kept, as `sync-profiles` promises for profiles ("unknown or missing fields load with
defaults"). No message said the file was ignored.

## Smaller things noticed

6. **Changes filter hides folder rows.** On the guided Review step (Changes is the default
   filter) files show indented under no folder: `IMG_0803.jpg` and `IMG_0805 (1).jpg` appear
   with no `Family/` row above them, so you can't tell which folder they are in without
   hovering. Folder rows whose own action is "none" disappear under the filter.
7. **Closing the window drops an unsaved sync silently.** Close the window (or Escape, see 4)
   after setting up and comparing, then File ▸ Synchronize Folders again: a new empty sync
   opens. Maybe intended, but there's no "discard?" prompt and no way back.
8. **The list doesn't grow with the window.** Making the window larger leaves the Review list
   and the Advanced list at about seven rows. With Advanced settings shown, the list is about
   two rows tall at 1440×900.
9. **Changes window jumps.** Clicking a folder row in the large Changes window to fold it opens
   the path details below the table, and the window moves to the top of the screen and grows.
10. **Activity panel shifts under the pointer.** Clicking *Clear all* under Messages shrinks the
    panel from the top, so the sync card's *Show* button moves to where the pointer isn't. A
    quick second click lands in the pane behind it.

## Retest on 04ed643 (2026-11-18, same setup, release build)

Fixed and checked in the real app: **1** (after Start syncing the card reads "Keeping in sync ·
Up to date" with Show / Stop syncing; the status bar "Syncing Photos backup · Up to date"; a file
added later is copied), **2** (Compare and ticking Keep in sync open the sign-in; after Trust the
compare runs by itself; cancelling shows "Not connected to 127.0.0.1: sign in, then press Compare
again" in the sync window), **3** (Connect to Server is in front of the chooser), **4** (Escape
closes only the menu), **5** (a `[window_geometry]` with only `size` keeps `check_updates` and
`theme`, and fills in position and scale_factor). 6–10 not retested.

## 11. Sync runs to SFTP break part-way: every remaining file fails with "Invalid file handle"

**Severity:** high: a first backup of a photo folder easily has thousands of files.

**Steps**
1. `tools/demo_sftp_server.py` from the site repo (asyncssh 2.24.1), sign in as demo/demo.
2. Left: a local folder with 2,053 files (53 photos plus `Camera roll 2024/` with 2,000 files of
   150–450 KB). Right: `sftp://demo@127.0.0.1:2222/backups/Photos` (does not exist yet). Goal
   Update.
3. Compare, Synchronize now (or tick Keep in sync and Start syncing: same result).

**Actual:** `Last run: 1035 copied, 0 deleted, 1024 failed`, every failed row
`Camera roll 2024/IMG_4075.jpg: i/o error: Invalid file handle` and so on; 1,029 files arrive. It
repeats exactly on a second run in the same session after emptying the target folder (again 1,035
copied, 1,024 failed), so the limit is per run, not per connection. The failures are always the
last 1,024 items.

**Not the server:** with asyncssh's own client, one session against the same server opens,
writes, `setstat`s and closes 1,100 files in a row without an error. The app has 21 descriptors
open afterwards and the server 4, so neither leaks file descriptors. asyncssh answers "Invalid file handle" only
when a request names a handle it does not know (never opened, or already closed), so the client
seems to send writes or closes on handles that are stale or were never opened, once some
1024-sized pool, queue or window in the copy pipeline or russh-sftp (3.x) wraps. Worth a test
with the in-repo `testserver` and about 2,100 files.

**Update, after more runs:** it is not a file count. A run with 959 items (53 photos plus 900
files of 400–900 KB) ended in `Last run: 441 failed` with 512 files (381.3 MiB) on the server. The
2,053-file runs delivered about 390 MiB (682 MiB planned, 292.5 MiB left). So each run stops
after roughly 380–390 MiB, whatever the number of files, and the "exactly 1,024" was a
coincidence. asyncssh's client sent 700 × 700 KB (467 MiB) in one session to the same server
without an error, so the server is not the limit. Every run reached that amount again on the same
connection, so a per-run counter, buffer or spool (or a per-channel limit, if each run opens a new
SFTP channel) is the place to look. Keep in sync's first run fails the same way.

**Update 2: it is not a size limit either; the failure point varies.** Two more first runs of Keep
in sync, each with 859 items (53 photos plus 800 files of 150–350 KB, 302.9 MiB), from a fresh
app start and sign-in:
- dark run: had already ended within 3 s of starting: `475 copied, 0 deleted, 384 failed`
  (469 files, 214 MB on the server);
- light run: still copying cleanly at 8 s (837 of 859, 21.5 MiB/s), but 495 files / 220 MB at
  the end, and the status bar then read `Sync Photos to server · 0%` (a follow-up run started).

So the point where it breaks moves (about 470, 500, 510 and 1,030 files in five runs). After the
first `Invalid file handle`, every later file in that run fails within moments, which looks like
the SFTP session (or russh-sftp's request/handle bookkeeping) getting out of step under
concurrent requests, for example responses matched to the wrong request id when several copies
share one SFTP channel. The next run on the same connection works again for a while. The sync
executor copies item by item (`crates/ops/src/transfer.rs`), so it is not parallel file copies;
more likely candidates are pipelined writes within one file (a close or the next open racing
writes still in flight) or another task sharing the session, such as remote polling. Worth
running the in-repo `testserver` with a few thousand files, and logging request ids and handles
in `fm-fs-sftp` around the first failure.

**Update 3: the failures come in alternating blocks of 128 files, and the same data fails the
same way every time.** Three dark runs with identical (seeded) data each ended `475 copied,
384 failed`. Which camera-roll files reached the server (names in plan order):

| IMG_ range  | files | result   |
|-------------|------:|----------|
| 4000–4086   |    87 | copied   |
| 4087–4214   |   128 | failed   |
| 4215–4342   |   128 | copied   |
| 4343–4470   |   128 | failed   |
| 4471–4598   |   128 | copied   |
| 4599–4726   |   128 | failed   |
| 4727–4799   |    73 | copied   |

No copied file has a wrong size. With the 53 photos and their folders ahead of the camera roll,
the first block also lines up with 128 items. So it is not "the session breaks once", but every
other batch of 128 items fails.

**Strong lead:** `fm-fs-sftp` opens two SFTP channels on one connection (`Inner::sftp` for file
I/O and `Inner::dir`, a `RawSftpSession` for listings; `crates/fs-sftp/src/fs.rs`). An SFTP
handle is only valid on the channel that opened it; asyncssh numbers handles per channel and
answers `Invalid file handle` for any handle that channel does not know. So something is sending a
request about a file handle on the wrong channel (or the other way round, a directory
handle), switching every 128 items. Look for a 128-sized batch (a listing or stat batch, a
readdir page, or a cache refresh between items) that changes which channel or session the next
open, write, fsetstat or close goes to. The in-repo `testserver` may hide this if its handle
names are shared across channels (`handles: HashMap<String, Open>` per `SftpSession` there);
check that it rejects another channel's handles like real servers do.

**Retest on v0.3.0 (2026-11-19): fixed.** Release build of `1cedf35`, same asyncssh demo server
and seeded data (2,055 local files, 682 MiB, `Thumbs.db`/`.DS_Store` left out). Advanced ▸ Copy new
and changed files to `sftp://demo@127.0.0.1:2222/backups/Photos`, Compare (sign-in and host key
prompts opened as expected), Synchronize now: "Last run: 2059 copied, 0 deleted, 0 failed" in
about 15 s, 2,053 files on the server, `diff -r` byte-identical, and a new Compare shows all equal.
(The cause was russh-sftp decoding handles as UTF-8, not the two-channel guess above.)

## 12. Advanced: "Change settings" after a compare grows the window past the screen

**Severity:** cosmetic; the title bar or the Synchronize button ends up off screen on short displays.

**Build:** v0.3.0 (`1cedf35`), release build, Xvfb 1440×900, found while retaking the site's sync
screenshots on 2026-11-19.

**Steps**
1. Synchronize Folders, drag the window to the top-left and its resize grip to fill the 1440×900
   screen (about 1408×893).
2. Advanced, two local folders, Compare. The settings fold into one line and the window fits.
3. Press *Change settings*.

**Expected:** the list shrinks to make room for the settings and the window keeps its size.

**Actual:** the window grows by about 65 px: its top moves about 19 px above the screen (title
bar half hidden) and its bottom about 46 px below (Synchronize now is cut off). *Hide settings*
brings it back. The list is already down to about two rows, so the list's minimum height plus the
settings seems to be more than the window has.
