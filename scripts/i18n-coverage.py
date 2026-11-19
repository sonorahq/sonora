#!/usr/bin/env python3

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LANGUAGES = ROOT / "crates/i18n/src/language.rs"
LOCALES = ROOT / "assets/i18n"
README = ROOT / "README.md"

START = "<!-- i18n:start -->"
END = "<!-- i18n:end -->"

SOURCE = "en-US"
ARM = re.compile(r"Self::(\w+) => \"([^\"]+)\",")


def shipped():
    source = LANGUAGES.read_text(encoding="utf-8")
    blocks = {}
    for name in ("id", "label"):
        match = re.search(
            rf"fn {name}\(self\) -> &'static str \{{(.*?)\n    \}}", source, re.S
        )
        if not match:
            sys.exit(f"cannot find Language::{name} in {LANGUAGES}")
        blocks[name] = dict(ARM.findall(match.group(1)))

    ids, labels = blocks["id"], blocks["label"]
    missing = set(ids) - set(labels)
    if missing:
        sys.exit(f"Language::label is missing {sorted(missing)}")

    return [(ids[variant], labels[variant]) for variant in ids]


def keys(locale):
    folder = LOCALES / locale
    if not folder.is_dir():
        sys.exit(f"cannot find {folder}")

    found = set()
    for path in sorted(folder.glob("*.ftl")):
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


def main():
    readme = README.read_text(encoding="utf-8")
    if START not in readme or END not in readme:
        sys.exit(f"{README} has no {START} … {END} block")

    head, rest = readme.split(START, 1)
    _, tail = rest.split(END, 1)
    README.write_text(f"{head}{START}\n\n{table()}\n\n{END}{tail}", encoding="utf-8")
    print(table())


if __name__ == "__main__":
    main()
