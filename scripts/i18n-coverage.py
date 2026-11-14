#!/usr/bin/env python3

import plistlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LANGUAGES = ROOT / "crates/i18n/src/language.rs"
LOCALES = ROOT / "assets/i18n"
README = ROOT / "README.md"
INFO_PLIST = ROOT / "assets/macos/Info.plist"

START = "<!-- i18n:start -->"
END = "<!-- i18n:end -->"

SOURCE = "en-US"
ARM = re.compile(r"Self::(\w+) => \"([^\"]+)\",")

# Sonora ids that macOS spells differently in CFBundleLocalizations.
APPLE_IDS = {"en-US": "en", "zh-CN": "zh-Hans"}


def shipped():
    source = LANGUAGES.read_text(encoding="utf-8")
    blocks = {}
    for name in ("id", "label"):
        match = re.search(rf"fn {name}\(self\) -> &'static str \{{(.*?)\n    \}}", source, re.S)
        if not match:
            sys.exit(f"cannot find Language::{name} in {LANGUAGES}")
        blocks[name] = dict(ARM.findall(match.group(1)))

    ids, labels = blocks["id"], blocks["label"]
    missing = set(ids) - set(labels)
    if missing:
        sys.exit(f"Language::label is missing {sorted(missing)}")

    return [(ids[variant], labels[variant]) for variant in ids]


def keys(locale):
    path = LOCALES / locale / "main.ftl"
    if not path.is_file():
        sys.exit(f"cannot find {path}")

    found = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line[0] in " #.*[" or " =" not in line:
            continue
        key = line.split(" =", 1)[0]
        if key and not key.startswith("-"):
            found.add(key)
    return found


def table():
    source = keys(SOURCE)
    total = len(source)
    rows = ["| Language | Translated | Coverage |", "| --- | --- | --- |"]

    for locale, label in shipped():
        done = len(source & keys(locale))
        share = round(done * 100 / total) if total else 0
        rows.append(f"| {label} (`{locale}`) | {done}/{total} | {share}% |")

    return "\n".join(rows)


def check_bundle():
    """Exit unless the macOS bundle declares every shipped language.

    The Dock draws Options, Hide and Quit in the app's bundle language, so a language missing from
    CFBundleLocalizations leaves those items in English.
    """
    with INFO_PLIST.open("rb") as file:
        declared = set(plistlib.load(file).get("CFBundleLocalizations", []))
    wanted = [APPLE_IDS.get(locale, locale) for locale, _ in shipped()]
    missing = [locale for locale in wanted if locale not in declared]
    if missing:
        sys.exit(f"add {missing} to CFBundleLocalizations in {INFO_PLIST}")


def main():
    check_bundle()
    readme = README.read_text(encoding="utf-8")
    if START not in readme or END not in readme:
        sys.exit(f"{README} has no {START} … {END} block")

    head, rest = readme.split(START, 1)
    _, tail = rest.split(END, 1)
    README.write_text(f"{head}{START}\n\n{table()}\n\n{END}{tail}", encoding="utf-8")
    print(table())


if __name__ == "__main__":
    main()
