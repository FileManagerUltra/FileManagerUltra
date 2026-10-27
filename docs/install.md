# Installing

Installers are built by the `release` workflow and attached to each GitHub Release (or, for a manual run, to the run's **Artifacts**). Each artifact is a zip; unzip it first. Verify the download against the `sha256` digest shown next to the artifact if you like (`sha256sum file.zip`, `shasum -a 256 file.zip`, or `Get-FileHash file.zip`).

The installers are **not signed**, so your OS will warn on first start (see below).

## Windows (x64)

1. Unzip `filemanagerultra-windows-x64` and run the `*-setup.exe`.
2. SmartScreen may say "Windows protected your PC": choose **More info → Run anyway**.
3. Start it from the Start menu. Uninstall via **Settings → Apps**.

## macOS (Apple silicon: `macos-arm64`, Intel: `macos-x64`)

1. Unzip, open the `*.dmg` and drag the app to **Applications**.
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

See [development.md](development.md).
