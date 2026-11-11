# App-managed ffmpeg

Video, chapters and poster frames need `ffmpeg` and `ffprobe`. Instead of
bundling them in every installer (a duplicate when the user already has them),
the app finds a system install first and otherwise offers a one-click download
of a pinned build, verified with a SHA-256 hash compiled into the app.

## Why

Windows has no package manager and macOS rarely has ffmpeg, so "install it
yourself" is a poor first run. Bundling adds ~90 MB per installer and a second
copy for users who already have ffmpeg. A pinned download keeps installers
small, never duplicates a system install, and keeps the trust anchor in our own
signed release rather than in a server.

## Scope

- Lookup order: sidecar next to the executable, `PATH` and package-manager
  locations, then the app's own `tools` data folder.
- Windows: Preferences > Media has an ffmpeg section with Download, Update and
  Reinstall. The pin is a versioned release archive (hash, size, URL, member
  names). Linux relies on the distribution package; macOS keeps using Homebrew
  (no trustworthy versioned static build exists to pin).
- Verification: HTTPS only, size-capped streaming download, SHA-256 compared
  with the pin before anything is unpacked, extraction restricted to the two
  expected members written to fixed names (no archive-supplied paths, no
  symlinks), atomic install, owner-only permissions, manifest recording the
  installed version.
- A newer pin shipped in a later app release shows "Update" in Preferences;
  nothing is downloaded without the user asking.
- Missing-tool hints point to the Preferences section on Windows.
- Bumping a pin is a reviewed commit, after the publisher's own checksum or
  signature has been checked.

## Out of scope

- Floating "latest" builds, a hosted manifest, silent background updates.
- Linux and macOS downloads, Authenticode checks of the downloaded binaries.
