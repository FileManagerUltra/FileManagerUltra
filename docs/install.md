# Installing

Installers are built by the `release` workflow and attached to each GitHub Release (or, for a manual run, to the run's **Artifacts**). Each artifact is a zip; unzip it first. Verify the download against the `sha256` digest shown next to the artifact if you like (`sha256sum file.zip`, `shasum -a 256 file.zip`, or `Get-FileHash file.zip`).

The installers are **not signed**, so your OS will warn on first start (see below).

## Windows (x64)

1. Unzip `filemanagerultra-windows-x64` and run the `*-setup.exe`.
2. SmartScreen may say "Windows protected your PC": choose **More info → Run anyway**.
3. Start it from the Start menu. Uninstall via **Settings → Apps**.

## macOS (Apple silicon: `macos-arm64`)

1. Unzip, clear the quarantine flag on the disk image (`xattr -d com.apple.quarantine ~/Downloads/FileManagerUltra_*.dmg`), open the `*.dmg` and drag the app to **Applications**.
2. Because the app is unsigned, Gatekeeper blocks the first start. Either right-click the app → **Open** → **Open**, or run:
   ```sh
   xattr -dr com.apple.quarantine /Applications/FileManagerUltra.app
   ```

## Linux (x64)

Debian / Ubuntu — the `.deb`:

```sh
sudo apt install ./filemanagerultra_*.deb
```

Any distro — the `.AppImage`:

```sh
chmod +x filemanagerultra_*.AppImage
./filemanagerultra_*.AppImage
```

The AppImage needs FUSE 2 (`sudo apt install libfuse2`, or run it with `--appimage-extract-and-run`). The window needs `libxkbcommon`, `libwayland-client`, `libGL` and `libEGL`, which a normal desktop install already has.

## Building from source

With the Rust toolchain installed, build an optimized executable locally without
using GitHub Actions minutes:

```sh
cargo build --release --locked -p fm-app
./target/release/filemanagerultra
```

These commands target the current operating system and architecture. On Windows,
the executable is `target/release/filemanagerultra.exe`. This builds the app, not
an installer. See [development.md](development.md) for prerequisites and platform
notes.

## Updating

The app checks the official repository's published stable GitHub Releases in the
background at startup. This contacts GitHub (which receives the request's IP
address and app version), but does not delay opening your folders or interrupt
file transfers. Disable **Check for updates at startup** in Preferences to opt out.
**Help → Check for Updates…** always lets you check manually.

A newer version opens the **Jobs** panel with a dismissible update entry.
**Download update…** opens the
GitHub release page in your browser; download the package for your OS and
architecture and follow the installation steps above. Installation is manual:
the app does not download, run installers, replace itself or restart automatically.
Wait for active transfers to finish and close the app before replacing it.
For `.deb` installations, use `apt install ./<new-package>.deb`.

Workflow artifacts and unpublished tags do not trigger update notifications:
there must be a published, non-prerelease GitHub Release with a newer version.
Checking and no-update results do not open the Jobs panel. An offline or
rate-limited check reports an error in the status bar; retry through
**Help → Check for Updates…**. Manual no-update results also use the status bar.

Windows packages built with the icon-resource fix embed the application icon in
the executable. Reinstall the updated package to recreate shortcuts; an old
pinned taskbar shortcut may need to be unpinned and pinned again.
