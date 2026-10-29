#!/usr/bin/env python3
"""Regenerate checked-in icons using Inkscape and ImageMagick; no Python packages."""

import shutil
import struct
import subprocess
import tempfile
from pathlib import Path


def render(source, destination, size):
    subprocess.run(
        [
            "inkscape", str(source), "--export-type=png",
            f"--export-width={size}", f"--export-height={size}",
            f"--export-filename={destination}",
        ],
        check=True,
    )


def main():
    assets = Path(__file__).resolve().parent
    packaging = assets.parents[2] / "packaging"
    for suffix in ("", "-windows", "-macos"):
        render(assets / f"logo{suffix}.svg", assets / f"icon{suffix}-256.png", 256)
    shutil.copyfile(assets / "logo.svg", packaging / "logo.svg")
    render(assets / "logo.svg", packaging / "icon.png", 512)
    with tempfile.TemporaryDirectory(prefix="fm-icons-") as directory:
        temporary = Path(directory)
        windows = temporary / "windows.png"
        render(assets / "logo-windows.svg", windows, 1024)
        subprocess.run(
            [
                "magick", str(windows), "-define",
                "icon:auto-resize=256,128,64,48,32,16",
                str(packaging / "icon.ico"),
            ],
            check=True,
        )
        chunks = []
        for kind, size in [
            (b"icp4", 16), (b"icp5", 32), (b"icp6", 64),
            (b"ic07", 128), (b"ic08", 256), (b"ic09", 512),
            (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64),
            (b"ic13", 256), (b"ic14", 512),
        ]:
            png = temporary / f"macos-{size}.png"
            if not png.exists():
                render(assets / "logo-macos.svg", png, size)
            data = png.read_bytes()
            chunks.append(kind + struct.pack(">I", len(data) + 8) + data)
        contents = b"".join(chunks)
        (packaging / "icon.icns").write_bytes(
            b"icns" + struct.pack(">I", len(contents) + 8) + contents
        )


if __name__ == "__main__":
    main()
