#!/usr/bin/env python3

import json
import re
import sys
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "assets/icons"
BASE = "lucide"
API = "https://api.iconify.design"

PACKS = {
    "iconoir": "iconoir",
    "solar": "solar",
    "remix": "ri",
}

MAP = {
    "arrow-up-down": ("sort", "sort-linear", "arrow-up-down-line"),
    "check": ("check", None, "check-line"),
    "chevron-down": ("nav-arrow-down", "alt-arrow-down-linear", "arrow-down-s-line"),
    "chevron-left": ("nav-arrow-left", "alt-arrow-left-linear", "arrow-left-s-line"),
    "chevron-right": ("nav-arrow-right", "alt-arrow-right-linear", "arrow-right-s-line"),
    "chevron-up": ("nav-arrow-up", "alt-arrow-up-linear", "arrow-up-s-line"),
    "chevrons-up-down": ("expand-lines", "sort-vertical-linear", "expand-up-down-line"),
    "circle-alert": ("warning-circle", "danger-circle-linear", "error-warning-line"),
    "circle-check": ("check-circle", "check-circle-linear", "checkbox-circle-line"),
    "clipboard-paste": ("paste-clipboard", "clipboard-linear", "clipboard-line"),
    "columns-3": ("view-columns-3", None, "layout-column-line"),
    "copy": ("copy", "copy-linear", "file-copy-line"),
    "disc-3": ("compact-disc", "vinyl-record-linear", "disc-line"),
    "ellipsis": ("more-horiz", "menu-dots-linear", "more-line"),
    "external-link": ("open-new-window", "square-top-down-linear", "external-link-line"),
    "file-music": (None, "music-library-2-linear", "file-music-line"),
    "folder-plus": ("folder-plus", "add-folder-linear", "folder-add-line"),
    "funnel": ("filter", "filter-linear", "filter-3-line"),
    "guitar": (None, None, None),
    "heart": ("heart", "heart-linear", "heart-3-line"),
    "heart-filled": ("heart-solid", "heart-bold", "heart-3-fill"),
    "heart-off": (None, None, None),
    "house": ("home-simple", "home-linear", "home-4-line"),
    "info": ("info-circle", "info-circle-linear", "information-line"),
    "layout-grid": ("view-grid", "widget-linear", "layout-grid-line"),
    "library-big": ("book-stack", "library-linear", "book-shelf-line"),
    "link": ("link", "link-linear", "link"),
    "list": ("list", "list-linear", "list-unordered"),
    "list-end": (None, "list-arrow-down-linear", None),
    "list-music": ("playlist", "playlist-linear", "play-list-line"),
    "list-plus": ("playlist-plus", "list-arrow-up-linear", "play-list-add-line"),
    "log-out": ("log-out", "logout-2-linear", "logout-box-r-line"),
    "maximize": ("expand", "maximize-square-3-linear", "fullscreen-line"),
    "mic-off": ("microphone-mute", None, "mic-off-line"),
    "mic-vocal": ("microphone", "microphone-linear", "mic-2-line"),
    "music": ("music-double-note", "music-note-linear", "music-2-line"),
    "music-2": ("music-note", "music-note-2-linear", "music-line"),
    "panel-left-close": ("sidebar-collapse", "sidebar-minimalistic-linear", "menu-fold-line"),
    "panel-left-open": ("sidebar-expand", "sidebar-linear", "menu-unfold-line"),
    "pause": ("pause", "pause-linear", "pause-line"),
    "pause-filled": ("pause-solid", "pause-bold", "pause-fill"),
    "pencil": ("edit-pencil", "pen-linear", "pencil-line"),
    "play": ("play", "play-linear", "play-line"),
    "play-filled": ("play-solid", "play-bold", "play-fill"),
    "play-off": (None, None, None),
    "plus": ("plus", None, "add-line"),
    "radio": ("antenna-signal", "radio-linear", "radio-line"),
    "refresh-cw": ("refresh", "refresh-linear", "refresh-line"),
    "repeat": ("repeat", "repeat-linear", "repeat-2-line"),
    "repeat-one": ("repeat-once", "repeat-one-linear", "repeat-one-line"),
    "rotate-ccw-clock": ("clock-rotate-right", "history-linear", "history-line"),
    "scissors": ("scissor", "scissors-linear", "scissors-line"),
    "search": ("search", "magnifier-linear", "search-line"),
    "settings": ("settings", "settings-linear", "settings-3-line"),
    "shuffle": ("shuffle", "shuffle-linear", "shuffle-line"),
    "skip-back": ("skip-prev", "skip-previous-linear", "skip-back-line"),
    "skip-forward": ("skip-next", "skip-next-linear", "skip-forward-line"),
    "sliders-horizontal": ("control-slider", "slider-horizontal-linear", "equalizer-line"),
    "text-select": (None, "text-selection-linear", None),
    "trash-2": ("trash", "trash-bin-minimalistic-linear", "delete-bin-line"),
    "undo-2": ("undo", "undo-left-round-linear", "arrow-go-back-line"),
    "user": ("user", "user-linear", "user-line"),
    "user-round": ("profile-circle", "user-rounded-linear", "user-3-line"),
    "volume": ("sound-min", "volume-small-linear", "volume-down-line"),
    "volume-1": ("sound-low", "volume-linear", "volume-down-line"),
    "volume-2": ("sound-high", "volume-loud-linear", "volume-up-line"),
    "volume-off": ("sound-off", "volume-cross-linear", "volume-mute-line"),
    "volume-x": ("sound-off", "volume-cross-linear", "volume-mute-line"),
    "x": ("xmark", "close-linear", "close-line"),
}

MIRROR = {
    "panel-right-close": "panel-left-close",
    "panel-right-open": "panel-left-open",
}

SLASH = {
    "mic-off": "mic-vocal",
}

GAP = 1.6

TEMPLATE = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"'
    ' viewBox="{left} {top} {width} {height}" fill="none">{body}</svg>\n'
)


def wanted(slot):
    return {name: MAP[name][slot] for name in MAP if MAP[name][slot]}


def fetch(prefix, names):
    query = urllib.parse.urlencode({"icons": ",".join(sorted(set(names)))})
    ask = urllib.request.Request(
        f"{API}/{prefix}.json?{query}", headers={"User-Agent": "sonora-fetch-icons"}
    )
    with urllib.request.urlopen(ask, timeout=60) as answer:
        return json.load(answer)


def crossed(sheet, icon):
    width = icon.get("width", sheet.get("width", 24))
    height = icon.get("height", sheet.get("height", 24))
    left = icon.get("left", sheet.get("left", 0))
    top = icon.get("top", sheet.get("top", 0))
    inset = width / 8
    line = f"M{left + inset} {top + inset}L{left + width - inset} {top + height - inset}"
    stroke = re.search(r'stroke-width="([\d.]+)"', icon["body"])
    stroke = float(stroke.group(1)) if stroke else 1.5

    body = (
        f'<mask id="cut">'
        f'<rect x="{left}" y="{top}" width="{width}" height="{height}" fill="white"/>'
        f'<path d="{line}" stroke="black" stroke-width="{stroke + 2 * GAP}" stroke-linecap="round"/>'
        f"</mask>"
        f'<g mask="url(#cut)">{icon["body"]}</g>'
        f'<path d="{line}" stroke="currentColor" stroke-width="{stroke}" stroke-linecap="round"/>'
    )
    return render(sheet, {**icon, "body": body})


def flipped(sheet, icon):
    width = icon.get("width", sheet.get("width", 24))
    left = icon.get("left", sheet.get("left", 0))
    axis = 2 * left + width
    body = f'<g transform="translate({axis} 0) scale(-1 1)">{icon["body"]}</g>'
    return render(sheet, {**icon, "body": body})


def render(sheet, icon):
    return TEMPLATE.format(
        width=icon.get("width", sheet.get("width", 24)),
        height=icon.get("height", sheet.get("height", 24)),
        left=icon.get("left", sheet.get("left", 0)),
        top=icon.get("top", sheet.get("top", 0)),
        body=icon["body"],
    )


def main():
    kept = {path.stem for path in (ICONS / BASE).glob("*.svg")}
    stray = set(MAP) - kept
    if stray:
        sys.exit(f"not in {BASE}: {', '.join(sorted(stray))}")

    short = [name for name, sources in MAP.items() if len(sources) != len(PACKS)]
    if short:
        sys.exit(f"every name needs {len(PACKS)} sources: {', '.join(sorted(short))}")

    for slot, (pack, prefix) in enumerate(PACKS.items()):
        picked = wanted(slot)
        answer = fetch(prefix, picked.values())
        icons = answer.get("icons", {})
        missing = sorted(answer.get("not_found", []))

        folder = ICONS / pack
        folder.mkdir(parents=True, exist_ok=True)
        for path in folder.glob("*.svg"):
            path.unlink()

        written = 0
        for name, source in sorted(picked.items()):
            icon = icons.get(source)
            if icon is None:
                continue
            (folder / f"{name}.svg").write_text(render(answer, icon), encoding="utf-8")
            written += 1

        for name, source in SLASH.items():
            if name in picked:
                continue
            icon = icons.get(picked.get(source, ""))
            if icon is None:
                continue
            (folder / f"{name}.svg").write_text(crossed(answer, icon), encoding="utf-8")
            written += 1

        for name, source in MIRROR.items():
            icon = icons.get(picked.get(source, ""))
            if icon is None:
                continue
            (folder / f"{name}.svg").write_text(flipped(answer, icon), encoding="utf-8")
            written += 1

        print(f"{pack}: {written}/{len(kept)} icons, {len(kept) - written} fall back to {BASE}")
        if missing:
            print(f"  unknown to {prefix}: {', '.join(missing)}")


if __name__ == "__main__":
    main()
