#!/usr/bin/env python3

import math
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MASTER = ROOT / "assets" / "icon.svg"

LINUX_SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
WINDOWS_SIZES = [16, 24, 32, 48, 64, 128, 256]
TRAY_SIZES = [32, 64]
TRAY_STROKE = 3.2

ICNS_TYPES = [
    ("icp4", 16),
    ("icp5", 32),
    ("ic11", 32),
    ("ic12", 64),
    ("ic07", 128),
    ("ic13", 256),
    ("ic08", 256),
    ("ic14", 512),
    ("ic09", 512),
    ("ic10", 1024),
]

SQUIRCLE_FILL = 824 / 1024
SQUIRCLE_RADIUS = 185.4 / 824
SQUIRCLE_SMOOTHING = 0.6
WINDOWS_RADIUS = 0.1875

OPTICAL_STROKE = [(16, 4.0), (24, 3.2), (32, 2.8), (48, 2.3)]


class Master:
    def __init__(self, path):
        if not path.exists():
            sys.exit(f"icons: {path} is missing")

        svg = ET.parse(path).getroot()
        ns = "{http://www.w3.org/2000/svg}"

        box = svg.get("viewBox")
        if box is None:
            sys.exit("icons: master has no viewBox")

        _, _, width, height = (float(n) for n in box.split())
        if width != height:
            sys.exit("icons: master viewBox is not square")
        self.span = width

        rect = svg.find(f"{ns}rect")
        self.background = rect.get("fill") if rect is not None else "white"

        self.bars = []
        for path_el in svg.iterfind(f"{ns}path"):
            match = re.fullmatch(
                r"M(-?[\d.]+) (-?[\d.]+)V(-?[\d.]+)", path_el.get("d").strip()
            )
            if not match:
                sys.exit(f"icons: unsupported path {path_el.get('d')!r}")
            self.bars.append(tuple(float(n) for n in match.groups()))
            self.foreground = path_el.get("stroke", "black")
            self.stroke = float(path_el.get("stroke-width", 2))
            self.cap = path_el.get("stroke-linecap", "round")

        if not self.bars:
            sys.exit("icons: master has no bars")

    def glyph(self, stroke):
        return "\n".join(
            f'  <path d="M{x} {y0}V{y1}" stroke="{self.foreground}"'
            f' stroke-width="{stroke:g}" stroke-linecap="{self.cap}"/>'
            for x, y0, y1 in self.bars
        )


def stroke_for(pixels, master):
    for limit, width in OPTICAL_STROKE:
        if pixels <= limit:
            return width * master.stroke / 2
    return master.stroke


def squircle(side, radius, smoothing):
    budget = side / 2
    reach = (1 + smoothing) * radius
    if reach > budget:
        smoothing = budget / radius - 1
        reach = budget

    arc_measure = 90 * (1 - smoothing)
    arc = math.sin(math.radians(arc_measure / 2)) * radius * math.sqrt(2)
    alpha = (90 - arc_measure) / 2
    beta = 45 * smoothing
    span = radius * math.tan(math.radians(alpha / 2))
    c = span * math.cos(math.radians(beta))
    d = c * math.tan(math.radians(beta))
    b = (reach - arc - c - d) / 3
    a = 2 * b
    ab = a + b
    long = a + b + c
    mid = b + c

    def n(value):
        return f"{value:.4f}"

    return " ".join(
        [
            f"M{n(side - reach)} 0",
            f"c{n(a)} 0 {n(ab)} 0 {n(long)} {n(d)}",
            f"a{n(radius)} {n(radius)} 0 0 1 {n(arc)} {n(arc)}",
            f"c{n(d)} {n(c)} {n(d)} {n(mid)} {n(d)} {n(long)}",
            f"L{n(side)} {n(side - reach)}",
            f"c0 {n(a)} 0 {n(ab)} {n(-d)} {n(long)}",
            f"a{n(radius)} {n(radius)} 0 0 1 {n(-arc)} {n(arc)}",
            f"c{n(-c)} {n(d)} {n(-mid)} {n(d)} {n(-long)} {n(d)}",
            f"L{n(reach)} {n(side)}",
            f"c{n(-a)} 0 {n(-ab)} 0 {n(-long)} {n(-d)}",
            f"a{n(radius)} {n(radius)} 0 0 1 {n(-arc)} {n(-arc)}",
            f"c{n(-d)} {n(-c)} {n(-d)} {n(-mid)} {n(-d)} {n(-long)}",
            f"L0 {n(reach)}",
            f"c0 {n(-a)} 0 {n(-ab)} {n(d)} {n(-long)}",
            f"a{n(radius)} {n(radius)} 0 0 1 {n(arc)} {n(-arc)}",
            f"c{n(c)} {n(-d)} {n(mid)} {n(-d)} {n(long)} {n(-d)}",
            "Z",
        ]
    )


def round_svg(master, pixels):
    half = master.span / 2
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{master.span:g}"'
        f' height="{master.span:g}" viewBox="0 0 {master.span:g} {master.span:g}">\n'
        f'  <circle cx="{half:g}" cy="{half:g}" r="{half:g}"'
        f' fill="{master.background}"/>\n'
        f"{master.glyph(stroke_for(pixels, master))}\n"
        f"</svg>\n"
    )


def squircle_svg(master, pixels):
    canvas = 1024.0
    side = canvas * SQUIRCLE_FILL
    inset = (canvas - side) / 2
    scale = side / master.span
    path = squircle(side, side * SQUIRCLE_RADIUS, SQUIRCLE_SMOOTHING)
    glyph = master.glyph(stroke_for(pixels * SQUIRCLE_FILL, master))
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{canvas:g}"'
        f' height="{canvas:g}" viewBox="0 0 {canvas:g} {canvas:g}">\n'
        f'  <g transform="translate({inset:g} {inset:g})">\n'
        f'    <path d="{path}" fill="{master.background}"/>\n'
        f'    <g transform="scale({scale:g})">\n'
        f"{glyph}\n"
        f"    </g>\n"
        f"  </g>\n"
        f"</svg>\n"
    )


def template_svg(master):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{master.span:g}"'
        f' height="{master.span:g}" viewBox="0 0 {master.span:g} {master.span:g}">\n'
        f"{master.glyph(TRAY_STROKE)}\n"
        f"</svg>\n"
    )


def rounded_svg(master, pixels):
    radius = master.span * WINDOWS_RADIUS
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{master.span:g}"'
        f' height="{master.span:g}" viewBox="0 0 {master.span:g} {master.span:g}">\n'
        f'  <rect width="{master.span:g}" height="{master.span:g}"'
        f' rx="{radius:g}" ry="{radius:g}" fill="{master.background}"/>\n'
        f"{master.glyph(stroke_for(pixels, master))}\n"
        f"</svg>\n"
    )


def rasterize(svg_text, pixels, workdir, name):
    source = workdir / f"{name}.svg"
    target = workdir / f"{name}.png"
    source.write_text(svg_text, encoding="utf-8")
    subprocess.run(
        [
            "rsvg-convert",
            "--width", str(pixels),
            "--height", str(pixels),
            "--keep-aspect-ratio",
            "--background-color", "none",
            "--output", str(target),
            str(source),
        ],
        check=True,
    )
    return target.read_bytes()


def write_ico(entries, path):
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = len(header) + 16 * len(entries)
    directory = b""
    payload = b""
    for pixels, blob in entries:
        directory += struct.pack(
            "<BBBBHHII",
            0 if pixels >= 256 else pixels,
            0 if pixels >= 256 else pixels,
            0,
            0,
            1,
            32,
            len(blob),
            offset,
        )
        payload += blob
        offset += len(blob)
    path.write_bytes(header + directory + payload)


def write_icns(entries, path):
    body = b""
    for kind, blob in entries:
        body += kind.encode("ascii") + struct.pack(">I", len(blob) + 8) + blob
    path.write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + body)


def main():
    if not shutil.which("rsvg-convert"):
        sys.exit("icons: rsvg-convert missing, run under `nix shell nixpkgs#librsvg`")

    master = Master(MASTER)
    assets = ROOT / "assets"

    with tempfile.TemporaryDirectory() as tmp:
        workdir = Path(tmp)

        scalable = assets / "linux" / "sonora.svg"
        scalable.parent.mkdir(parents=True, exist_ok=True)
        scalable.write_text(round_svg(master, 512), encoding="utf-8")

        for pixels in LINUX_SIZES:
            blob = rasterize(round_svg(master, pixels), pixels, workdir, f"l{pixels}")
            target = (
                assets
                / "linux"
                / "icons"
                / "hicolor"
                / f"{pixels}x{pixels}"
                / "apps"
                / "sonora.png"
            )
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(blob)

        icns = []
        rendered = {}
        for kind, pixels in ICNS_TYPES:
            if pixels not in rendered:
                rendered[pixels] = rasterize(
                    squircle_svg(master, pixels), pixels, workdir, f"m{pixels}"
                )
            icns.append((kind, rendered[pixels]))
        write_icns(icns, assets / "macos" / "sonora.icns")

        if shutil.which("actool"):
            icon_doc = assets / "macos" / "AppIcon.icon"
            if icon_doc.exists():
                target_car = assets / "macos" / "Assets.car"
                target_car.unlink(missing_ok=True)
                out_car = workdir / "car_out"
                out_car.mkdir(parents=True, exist_ok=True)
                subprocess.run(
                    [
                        "actool",
                        "--compile",
                        str(out_car),
                        "--platform",
                        "macosx",
                        "--minimum-deployment-target",
                        "11.0",
                        "--app-icon",
                        "AppIcon",
                        str(icon_doc),
                        "--output-partial-info-plist",
                        str(out_car / "partial.plist"),
                    ],
                    check=True,
                    stdout=subprocess.DEVNULL,
                )

                compiled_car = out_car / "Assets.car"
                if not compiled_car.exists():
                    sys.exit("icons: actool did not produce Assets.car")
                shutil.copy(compiled_car, target_car)

        ico = [
            (
                pixels,
                rasterize(rounded_svg(master, pixels), pixels, workdir, f"w{pixels}"),
            )
            for pixels in WINDOWS_SIZES
        ]
        windows = assets / "windows"
        windows.mkdir(parents=True, exist_ok=True)
        write_ico(ico, windows / "sonora.ico")

        tray = assets / "tray"
        tray.mkdir(parents=True, exist_ok=True)
        for pixels in TRAY_SIZES:
            blob = rasterize(template_svg(master), pixels, workdir, f"t{pixels}")
            (tray / f"template-{pixels}.png").write_bytes(blob)
        (tray / "sonora.png").write_bytes(
            rasterize(round_svg(master, 32), 32, workdir, "tray")
        )

    has_car = (assets / "macos" / "Assets.car").exists()
    print(
        f"icons: {len(LINUX_SIZES)} linux png, 1 linux svg, 1 icns,"
        f" {'1 Assets.car, ' if has_car else ''}1 ico,"
        f" {len(TRAY_SIZES) + 1} tray png"
    )


if __name__ == "__main__":
    main()
