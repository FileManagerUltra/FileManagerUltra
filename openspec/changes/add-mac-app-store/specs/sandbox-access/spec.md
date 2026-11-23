## ADDED Requirements

### Requirement: First run asks for folder access
On first launch of a sandboxed build, before the first listing, the app SHALL show a
dialog explaining that macOS requires permission and offering "Grant access to Home
folder" (recommended), "Grant access to the whole disk" and "Not now". Each grant
choice SHALL open the system folder picker pre-pointed at the real home folder or `/`;
the user confirms there. The dialog SHALL NOT appear again once any grant exists or
after "Not now", and SHALL be reachable later from Preferences → Folder access and
from Go → Grant Folder Access….

#### Scenario: Grant home on first run
- **WHEN** a new user starts the App Store edition and chooses Grant access to Home folder and confirms the picker
- **THEN** the pane lists the real home folder and the grant is stored

#### Scenario: Not now
- **WHEN** the user chooses Not now
- **THEN** the pane shows the access-needed state for the home folder, the app stays usable for remote connections, and the dialog does not reappear on the next launch

### Requirement: Grants persist as security-scoped bookmarks
Every folder the user confirms in the picker SHALL be stored as an app-scoped,
security-scoped bookmark together with its path, and on launch every stored bookmark
SHALL be resolved and its access started before local jobs run, on a worker, without
blocking the UI. A stale bookmark SHALL be refreshed; a bookmark that no longer
resolves SHALL be kept but marked unavailable in Preferences, never deleted silently.
Access SHALL stay started for the life of the process and be stopped on exit. Access
to a granted folder SHALL cover everything below it.

#### Scenario: Relaunch keeps access
- **WHEN** the user granted `~/Projects`, quits and starts the app again
- **THEN** `~/Projects` and its subfolders list without any prompt

#### Scenario: Moved folder
- **WHEN** a granted folder was renamed while the app was closed
- **THEN** the bookmark resolves to the new path, is refreshed and the new path is shown in Preferences

#### Scenario: Deleted folder
- **WHEN** a granted folder no longer exists
- **THEN** Preferences shows the grant as unavailable with a Remove button and no error dialog appears at startup

### Requirement: Denied folders offer to grant access
The backend SHALL, when a local operation fails because the sandbox denies access to a
path not covered by a grant, return a distinct access-not-granted error carrying the
path. A Files pane showing such a folder SHALL show an explanation and a
"Grant access…" button instead of an empty listing; the button SHALL dispatch
`access.grant` with that path, which opens the system picker at it. After the user
confirms, the pane SHALL reload. File operations failing this way SHALL report the
same actionable error in the jobs panel. On builds that are not sandboxed this error
SHALL never be produced for ordinary permission problems.

#### Scenario: Open an ungranted folder
- **WHEN** only `~/Documents` is granted and the user navigates to `/Applications`
- **THEN** the pane shows that access is needed with a Grant access… button, and after confirming the picker it lists `/Applications`

#### Scenario: Copy into an ungranted folder
- **WHEN** a copy targets a folder without a grant
- **THEN** the job fails before writing anything and its message offers Grant access…

### Requirement: Grants can be reviewed and revoked
Preferences SHALL list every grant with its path and state (active, unavailable) and
offer Add… and Remove. `access.revoke` SHALL stop access, delete the bookmark and
refresh panes that show folders below it. Commands `access.grant` and `access.revoke`
SHALL validate their params (absolute local path, length-capped, no NUL) and return an
error for malformed input; on builds without a sandbox they SHALL return an
unsupported error.

#### Scenario: Revoke
- **WHEN** the user removes the grant for `~/Projects` while a pane shows it
- **THEN** the pane switches to the access-needed state and the grant is gone after relaunch

#### Scenario: Malformed params
- **WHEN** `access.grant` or `access.revoke` is dispatched with a relative path, an empty string, a path with NUL, a 100 000-byte path or a non-string value
- **THEN** it returns an error and does not open a picker

### Requirement: Dropped items are usable for the session
Files and folders dropped onto the window from another app SHALL be usable by every
operation for the rest of the session without a grant, because macOS extends the
sandbox for them. They SHALL NOT be persisted as grants unless the user grants them.

#### Scenario: Drop from Finder
- **WHEN** the user drops a folder from Finder onto a pane in the App Store edition with no grant for it
- **THEN** the folder can be opened and copied from until the app quits

### Requirement: Real user folders inside the sandbox
Places, Go → Home, new panes and the default workspace SHALL use the real home folder.
In a sandboxed build, Places (Home, Desktop, Documents, Downloads, Music, Pictures,
Videos) and these defaults SHALL resolve under the user's real home folder, never the
sandbox container. The real home SHALL be derived from the
account's password-database entry, falling back to removing the
`Library/Containers/<bundle id>/Data` suffix from `HOME`; if neither yields an
existing absolute path, Places SHALL omit Home rather than show the container.

#### Scenario: Places in the sandbox
- **WHEN** the App Store edition lists Places
- **THEN** Home is `/Users/<name>`, not a path under `~/Library/Containers`

### Requirement: Volumes need a grant too
In a sandboxed build, Devices SHALL list mounted volumes under `/Volumes` only when
`/Volumes` or `/` is granted, and otherwise SHALL show a single "Grant access to
volumes…" entry. Ejecting a listed volume SHALL use `NSWorkspace`; if the system
refuses, the error SHALL say so.

#### Scenario: USB stick without a grant
- **WHEN** a USB stick is mounted and no grant covers `/Volumes`
- **THEN** Devices shows Grant access to volumes… and after granting it lists the stick
