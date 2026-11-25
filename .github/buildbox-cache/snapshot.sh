#!/usr/bin/env bash
# Builds /checkout with the Arch toolchain and packs the cargo target into /work/out as two layers,
# deps.tar.zst for third-party crates and ws.tar.zst for the workspace. A previous snapshot in
# /work/prev seeds the build when one matches. Runs as root in an archlinux container.
set -euo pipefail

if [[ ${1:-} != --as-builder ]]; then
  pacman -Syu --noconfirm --needed $(grep -v '^#' /checkout/.github/buildbox-cache/packages)
  useradd -m -s /bin/bash builder
  mkdir -p /work/snap /work/out /home/.cargo /home/.rustup
  chown builder: /work/snap /work/out /home/.cargo /home/.rustup
  # Cargo fingerprints third-party crates by their absolute source path and clippy's units by the
  # path of clippy-driver, so a machine reuses this snapshot only when it builds with the same
  # CARGO_HOME and RUSTUP_HOME.
  exec runuser -u builder -- env -i HOME=/home/builder USER=builder LOGNAME=builder SHELL=/bin/bash \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/bin CARGO_HOME=/home/.cargo RUSTUP_HOME=/home/.rustup \
    CARGO_TARGET_DIR=/work/snap/target CARGO_BUILD_JOBS="$(nproc)" \
    CMAKE_BUILD_PARALLEL_LEVEL=12 MAKEFLAGS=-j12 \
    bash "$0" --as-builder </dev/null
fi

snap=/work/snap
if [[ -f /work/prev/deps.tar.zst && -f /work/prev/ws.tar.zst ]]; then
  echo "snapshot: seeding from the previous snapshot"
  for layer in deps ws; do zstd -dc --long=27 "/work/prev/$layer.tar.zst" | tar -x -C "$snap"; done
else
  echo "snapshot: no matching snapshot, building cold"
fi
mkdir -p "$snap/src" "$snap/target"
# Compares content, so unchanged files keep the mtimes the seed was built with.
rsync -rlpc --delete --exclude=/.git /checkout/ "$snap/src/"

cd "$snap/src"
rustup toolchain install
touch /tmp/build-start
# The JSON messages name every unit the build used, which the pruning below keeps. A plain check
# resolves other features than the --all-targets runs, so its units are built too.
for build in "build --locked -p sonora" "check --locked --workspace" \
  "clippy --locked --workspace --all-targets" "test --locked --workspace --no-run"; do
  cargo $build --message-format=json-render-diagnostics >> /tmp/units.jsonl
done

cargo metadata --locked --no-deps --format-version 1 > /tmp/metadata.json
python - "$snap" /tmp/metadata.json /tmp/build-start /tmp/units.jsonl <<'PY'
import json, os, re, shutil, sys
from collections import defaultdict

snap, meta = sys.argv[1], json.load(open(sys.argv[2]))
start = os.stat(sys.argv[3]).st_mtime
debug = os.path.join(snap, "target/debug")
deps = os.path.join(debug, "deps")

# Drops units this build no longer uses, found by the hash in their names or, for a binary copied
# up without one, by the inode it shares with its unit.
unit_hash = re.compile(r"-([0-9a-f]{16})(?=\.|$)")
used, inodes = set(), set()
for line in open(sys.argv[4]):
    if not line.startswith("{"):
        continue
    msg = json.loads(line)
    if msg["reason"] == "compiler-artifact":
        paths = msg["filenames"] + ([msg["executable"]] if msg.get("executable") else [])
    elif msg["reason"] == "build-script-executed":
        paths = [msg["out_dir"]]
    else:
        continue
    for path in paths:
        for part in path.rstrip("/").split("/")[-2:]:
            if found := unit_hash.search(part):
                used.add(found.group(1))
        if os.path.exists(path):
            inodes.add(os.stat(path).st_ino)
for folder in ("deps", "examples"):
    folder = os.path.join(debug, folder)
    for name in os.listdir(folder) if os.path.isdir(folder) else []:
        if (found := unit_hash.search(name)) and os.lstat(os.path.join(folder, name)).st_ino in inodes:
            used.add(found.group(1))
if len(used) < 100:
    sys.exit(f"snapshot: cargo named only {len(used)} units, refusing to prune")
pruned = 0
for folder in ("deps", ".fingerprint", "build", "examples"):
    if not os.path.isdir(os.path.join(debug, folder)):
        continue
    for name in os.listdir(os.path.join(debug, folder)):
        found = unit_hash.search(name)
        if not found or found.group(1) in used:
            continue
        path = os.path.join(debug, folder, name)
        if os.path.isdir(path) and not os.path.islink(path):
            shutil.rmtree(path)
        else:
            os.remove(path)
        pruned += 1
print(f"snapshot: kept {len(used)} units, removed {pruned} entries of units no longer built")

# A recompiled unit leaves old split debuginfo under other codegen unit names behind.
units = defaultdict(list)
for name in os.listdir(deps):
    unit = re.match(r"^(?:lib)?([A-Za-z0-9_]+-[0-9a-f]{16})", name)
    if unit:
        units[unit.group(1)].append(os.path.join(deps, name))
stale = 0
for files in units.values():
    times = {f: os.lstat(f).st_mtime for f in files}
    if max(times.values()) >= start:
        for f, t in times.items():
            if t < start:
                os.remove(f)
                stale += 1
print(f"snapshot: removed {stale} files left from earlier compiles")

names = set()
for pkg in meta["packages"]:
    for name in [pkg["name"]] + [t["name"] for t in pkg["targets"]]:
        names.update({name, name.replace("-", "_")})
alt = "|".join(re.escape(n) for n in sorted(names, key=len, reverse=True))
workspace_unit = re.compile(
    rf"^target/debug/(?:deps|\.fingerprint|build|examples)/(?:lib)?(?:{alt})-[0-9a-f]{{16}}(?:[./].*)?$"
)
layers = {"deps": [], "ws": []}
for root, dirs, files in os.walk(snap):
    rel_root = os.path.relpath(root, snap)
    if rel_root in ("target/debug/incremental", "target/tmp"):
        dirs[:] = []
        continue
    # Source folders keep their mtimes, or build scripts that watch asset folders run again.
    if rel_root == "src" or rel_root.startswith("src/"):
        layers["ws"].append(rel_root)
    for name in files + [d for d in dirs if os.path.islink(os.path.join(root, d))]:
        rel = os.path.normpath(os.path.join(rel_root, name))
        if rel.endswith(".dwo"):
            continue
        top_level = os.path.dirname(rel) in ("target", "target/debug")
        # Only workspace crates have examples, and `cargo test` relinks every one that is missing.
        example = rel.startswith("target/debug/examples/")
        if rel.startswith("src/") or top_level or example or workspace_unit.match(rel):
            layers["ws"].append(rel)
        else:
            layers["deps"].append(rel)
for layer, paths in layers.items():
    with open(f"/tmp/{layer}.list", "wb") as f:
        f.write(b"".join(p.encode() + b"\0" for p in sorted(paths)))
    print(f"snapshot: {layer} layer has {len(paths)} files")

# When this sum matches the previous snapshot's, its third-party archive is reused as it is.
import hashlib
digest = hashlib.sha256()
for rel in sorted(layers["deps"]):
    info = os.lstat(os.path.join(snap, rel))
    digest.update(f"{rel}\0{info.st_size}\0{info.st_mtime_ns}\n".encode())
with open("/work/out/deps.sum", "w") as f:
    f.write(digest.hexdigest() + "\n")
PY

if [[ -f /work/prev/deps.sum && -f /work/prev/deps.tar.zst ]] && cmp -s /work/prev/deps.sum /work/out/deps.sum; then
  echo "snapshot: third-party crates unchanged, reusing their archive"
  cp /work/prev/deps.tar.zst /work/out/deps.tar.zst
  layers=ws
else
  layers="deps ws"
fi
# Deterministic archives, so an unchanged layer keeps its digest and is neither pushed nor pulled.
# Level 9 packs the 3.3 GB workspace layer in 60% of the time level 12 takes, for 3% more bytes.
for layer in $layers; do
  tar -C "$snap" -c --format=posix --no-recursion --null -T "/tmp/$layer.list" \
    --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
    --owner=0 --group=0 --numeric-owner \
    | zstd -q -T4 -9 --long=27 -o "/work/out/$layer.tar.zst"
done
ls -la /work/out
