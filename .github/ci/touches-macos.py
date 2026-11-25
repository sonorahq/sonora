#!/usr/bin/env python3
"""Prints `true` when the change from BASE to HEAD needs the macOS lint, `false` otherwise.

The Linux and Windows lints compile everything but the code only macOS builds, so only a change to
that code needs a Mac. A change needs one when it touches a manifest, the lockfile, the toolchain,
a build script, C or Objective-C sources or this lint's own setup, when a Rust file it touches
names macOS before or after, or when it touches a module declared behind a cfg that names macOS.
A change to shared code that breaks a macOS caller in a file it leaves alone gets past this, and
the lint of the push to main catches it there.

Usage: touches-macos.py BASE HEAD
"""

import re
import subprocess
import sys
from pathlib import PurePosixPath

# Anything that can change what macOS compiles without naming macOS.
ALWAYS = re.compile(
    r"(^|/)Cargo\.(toml|lock)$"
    r"|^crates/[^/]+/build\.rs$"
    r"|^rust-toolchain\.toml$"
    r"|^\.cargo/"
    r"|^\.github/workflows/ci\.yml$"
    r"|^\.github/ci/"
    r"|\.(c|cc|cpp|h|hpp|m|mm)$"
)

# How a Rust file names macOS: a cfg, a target triple, a module, a helper like `is_macos`, or a
# cfg that leaves Linux out, which a Unix one narrows to macOS. Apple alone would also match
# Apple Music.
MACOS = re.compile(
    r'macos|apple-darwin|target_vendor\s*=\s*"apple"|target_os\s*=\s*"ios"'
    r'|not\(\s*(any\(\s*)?target_os\s*=\s*"linux"',
    re.IGNORECASE,
)

# A `mod name;` behind a cfg, with any further attributes between the two.
GATED_MOD = re.compile(
    r"#\[cfg\(([^\]]*)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"
)


def main() -> None:
    base, head = sys.argv[1:3]
    print("true" if needs_macos(base, head) else "false")


def needs_macos(base: str, head: str) -> bool:
    """Whether the change needs the macOS lint, with the reason on stderr for the job's log."""
    changed = git("diff", "--name-only", "--no-renames", "-z", base, head).split("\0")
    gated = gated_modules(head)
    for path in filter(None, changed):
        if ALWAYS.search(path) or "macos" in path.lower():
            return found(f"{path} can change what macOS compiles")
        if not path.endswith(".rs"):
            continue
        if any(path == module or path.startswith(module + "/") for module in gated):
            return found(f"{path} is a module behind a macOS cfg")
        if any(MACOS.search(show(rev, path)) for rev in (base, head)):
            return found(f"{path} names macOS")
    print("macos lint: nothing in this change compiles for macOS alone", file=sys.stderr)
    return False


def gated_modules(rev: str) -> set[str]:
    """The modules a cfg naming macOS declares at `rev`, as a directory and as a `.rs` file."""
    candidates = git("grep", "-lz", "-iE", r'macos|apple|"ios"|linux', rev, "--", "*.rs").split("\0")
    modules = set()
    for entry in filter(None, candidates):
        path = PurePosixPath(entry.removeprefix(f"{rev}:"))
        parent = path.parent if path.name in ("lib.rs", "main.rs", "mod.rs") else path.with_suffix("")
        for condition, name in GATED_MOD.findall(show(rev, str(path))):
            if MACOS.search(condition):
                modules.add(str(parent / name))
    return modules | {f"{module}.rs" for module in modules}


def found(why: str) -> bool:
    print(f"macos lint: {why}", file=sys.stderr)
    return True


def show(rev: str, path: str) -> str:
    """The file at `rev`, or nothing when it does not exist there."""
    result = subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True, text=True)
    return result.stdout if result.returncode == 0 else ""


def git(*args: str) -> str:
    """Runs git and returns its output. `git grep` finding nothing is an empty output."""
    result = subprocess.run(["git", *args], capture_output=True, text=True)
    if result.returncode not in ((0, 1) if args[0] == "grep" else (0,)):
        sys.exit(f"macos lint: git {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


if __name__ == "__main__":
    main()
