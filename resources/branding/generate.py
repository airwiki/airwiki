#!/usr/bin/env python3
"""Regenerate AirWiki's branding with the repository's pinned Tauri CLI and Pillow."""

import argparse
import io
import re
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[2]
BRANDING = ROOT / "resources/branding"
ICONS = ROOT / "apps/desktop/icons"
CLI = ROOT / "apps/desktop/ui/node_modules/@tauri-apps/cli/tauri.js"
SOURCE = BRANDING / "airwiki-mark.svg"
WIKI = ROOT / "docs/assets/wiki-linked-w.svg"
COMPONENT = ROOT / "apps/desktop/ui/src/components/WikiIcon.svelte"


def geometry(element):
    """Compare the actual strokes and nodes, independent of SVG wrappers."""
    return [
        (node.tag.rsplit("}", 1)[-1], node.attrib)
        for node in element.iter()
        if node.tag.rsplit("}", 1)[-1] in ("path", "circle")
    ]


def validate_geometry():
    reference = geometry(ET.parse(WIKI).getroot())
    component_shapes = re.findall(r"<(?:path|circle)\b[^>]*?/>", COMPONENT.read_text())
    component = geometry(ET.fromstring("<svg>" + "".join(component_shapes) + "</svg>"))
    if not reference or geometry(ET.parse(SOURCE).getroot()) != reference or component != reference:
        raise ValueError("The brand, wiki reference, and WikiIcon must use exactly the same W geometry")


def tauri_icon(source, destination, sizes=()):
    command = ["node", str(CLI), "icon", str(source), "--output", str(destination)]
    for size in sizes:
        command.extend(["--png", str(size)])
    subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True)


def png(image):
    output = io.BytesIO()
    image.save(output, format="PNG", compress_level=9)
    return output.getvalue()


def canonical_icns(content):
    """Tauri emits ICNS entries in hash-map order; sort intact entries for stable bytes."""
    if content[:4] != b"icns" or int.from_bytes(content[4:8], "big") != len(content):
        raise ValueError("Invalid ICNS container")
    chunks = []
    offset = 8
    while offset < len(content):
        kind, length = struct.unpack(">4sI", content[offset:offset + 8])
        if length < 8 or offset + length > len(content):
            raise ValueError("Invalid ICNS entry")
        chunks.append((kind, content[offset:offset + length]))
        offset += length
    return content[:8] + b"".join(chunk for _, chunk in sorted(chunks))


def render_outputs(directory):
    validate_geometry()
    if not CLI.is_file():
        raise FileNotFoundError("Install the pinned desktop UI dependencies before generating branding")
    native = directory / "native"
    raster = directory / "raster"
    tauri_icon(SOURCE, native)
    tauri_icon(SOURCE, raster, (24, 32, 64, 128, 256, 500, 512, 1024))
    images = {
        size: Image.open(raster / f"{size}x{size}.png").convert("RGBA")
        for size in (24, 32, 64, 128, 256, 500, 512, 1024)
    }
    # The macOS template uses the W's alpha, so the tray must never contain the tile.
    # Its blue RGB also identifies the same W on the Windows tray.
    tray_svg = ET.parse(WIKI).getroot()
    tray_svg.set("width", "24")
    tray_svg.set("height", "24")
    tray_svg.set("color", "#1461e8")
    tray_source = directory / "tray.svg"
    ET.ElementTree(tray_svg).write(tray_source, encoding="unicode")
    tauri_icon(tray_source, directory / "tray", (24,))
    tray = Image.open(directory / "tray/24x24.png").convert("RGBA")
    avatar = Image.new("RGB", (500, 500), "white")
    avatar.paste(images[500], mask=images[500].getchannel("A"))
    social = Image.new("RGB", (1280, 640), "white")
    social.paste(images[512], (384, 64), images[512].getchannel("A"))
    outputs = {
        BRANDING / "airwiki-mark.png": png(images[1024]),
        BRANDING / "airwiki-app-icon.png": png(images[1024]),
        BRANDING / "github-avatar.png": png(avatar),
        BRANDING / "github-social-preview.png": png(social),
        BRANDING / "airwiki-window.rgba": images[128].tobytes(),
        BRANDING / "airwiki-tray.rgba": tray.tobytes(),
        ROOT / "site/src/assets/airwiki-mark.png": png(images[1024]),
        ROOT / "apps/desktop/ui/src/assets/airwiki-mark-transparent.png": png(images[256]),
    }
    for name in ("icon.ico", "icon.icns"):
        content = (native / name).read_bytes()
        if name.endswith(".icns"):
            content = canonical_icns(content)
        outputs[ICONS / name] = content
        outputs[BRANDING / name.replace("icon", "airwiki")] = content
    for name, size in {"icon.png": 512, "32x32.png": 32, "64x64.png": 64,
                       "128x128.png": 128, "128x128@2x.png": 256}.items():
        outputs[ICONS / name] = png(images[size])
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Check every derivative without modifying it")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="airwiki-branding-") as temporary:
        outputs = render_outputs(Path(temporary))
    changed = []
    for destination, content in outputs.items():
        if destination.is_file() and destination.read_bytes() == content:
            continue
        changed.append(destination.relative_to(ROOT))
        if not args.check:
            destination.write_bytes(content)
    if args.check and changed:
        for name in changed:
            print(f"Outdated branding: {name}")
        return 1
    print(f"Branding {'verified' if args.check else 'generated'}: {len(outputs)} assets; {len(changed)} changed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.stderr or error.stdout or "Tauri icon generation failed") from error
