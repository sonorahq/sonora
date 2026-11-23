#!/usr/bin/env bash
# Builds the checkout the way rbuild's plain mode builds it on the buildbox and packs the cargo
# target into two layers. Runs as root in an archlinux container started by buildbox-cache.yml:
#
#   /checkout          the commit to build, read only
#   /work/prev/        the previous snapshot's deps.tar.zst, ws.tar.zst and deps.sum, when they can
#                      seed this one
#   /work/out/         receives deps.tar.zst, ws.tar.zst and deps.sum
#
# deps.tar.zst holds the third-party crates, so its bytes only change when they are rebuilt and the
# buildbox skips downloading it otherwise. ws.tar.zst holds the workspace crates, the test binaries
# and the sources the build saw. It also holds the app binary: without it cargo compiles and links
# the app crate again in every new checkout. Left out, to keep the download small:
#   - the incremental caches, since a missing one only makes the next rebuild of that crate a full one
#   - the examples, which `cargo test` builds again in seconds
#   - the split debuginfo (.dwo): builds never read it, only a debugger looks for it
set -euo pipefail

if [[ ${1:-} != --as-builder ]]; then
  pacman -Syu --noconfirm --needed $(grep -v '^#' /checkout/.github/buildbox-cache/packages)
  useradd -m -u 1001 -s /bin/bash sleebu
  mkdir -p /work/snap /work/out
  chown sleebu: /work/snap /work/out
  # The buildbox runs rbuild-run over ssh with little more than HOME, USER and PATH, plus these.
  exec runuser -u sleebu -- env -i HOME=/home/sleebu USER=sleebu LOGNAME=sleebu SHELL=/bin/bash \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/bin \
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
# Same comparison as rbuild: unchanged files keep the mtimes the seed was built with.
rsync -rlpc --delete --exclude=/.git /checkout/ "$snap/src/"

cd "$snap/src"
rustup toolchain install
touch /tmp/build-start
# The JSON messages name every unit the build used, fresh or compiled; the human output stays on
# stderr.
for build in "build --locked -p sonora" "clippy --locked --workspace --all-targets" \
  "test --locked --workspace --no-run"; do
  cargo $build --message-format=json-render-diagnostics >> /tmp/units.jsonl
done

# Splits every file under $snap into the two layers, by the names cargo gives workspace units.
cargo metadata --locked --no-deps --format-version 1 > /tmp/metadata.json
python - "$snap" /tmp/metadata.json /tmp/build-start /tmp/units.jsonl <<'PY'
import json, os, re, shutil, sys
from collections import defaultdict

snap, meta = sys.argv[1], json.load(open(sys.argv[2]))
start = os.stat(sys.argv[3]).st_mtime
debug = os.path.join(snap, "target/debug")
deps = os.path.join(debug, "deps")

# A seed built from another Cargo.lock keeps the units of crates that changed version or source.
# Every unit this run used shows up in cargo's messages, through a file or folder named with its
# hash or, for a binary or example copied up without one, the inode it shares with its unit.
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
for folder in ("deps", ".fingerprint", "build"):
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

# A unit compiled again in this run leaves the split debuginfo of its previous compile behind
# under other codegen unit names. Drop every file of such a unit that this run did not write.
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
    if rel_root in ("target/debug/incremental", "target/debug/examples", "target/tmp"):
        dirs[:] = []
        continue
    # Source folders go in as entries of their own, so they keep their mtimes: build scripts
    # watch whole asset folders, and a folder newer than the build would run them again.
    if rel_root == "src" or rel_root.startswith("src/"):
        layers["ws"].append(rel_root)
    for name in files + [d for d in dirs if os.path.islink(os.path.join(root, d))]:
        rel = os.path.normpath(os.path.join(rel_root, name))
        if rel.endswith(".dwo"):
            continue
        top_level = os.path.dirname(rel) in ("target", "target/debug")
        if rel.startswith("src/") or top_level or workspace_unit.match(rel):
            layers["ws"].append(rel)
        else:
            layers["deps"].append(rel)
for layer, paths in layers.items():
    with open(f"/tmp/{layer}.list", "wb") as f:
        f.write(b"".join(p.encode() + b"\0" for p in sorted(paths)))
    print(f"snapshot: {layer} layer has {len(paths)} files")

# Names, sizes and mtimes of the third-party layer: when they match the previous snapshot's, its
# archive is reused as it is instead of compressing the same bytes again.
import hashlib
digest = hashlib.sha256()
for rel in sorted(layers["deps"]):
    info = os.lstat(os.path.join(snap, rel))
    digest.update(f"{rel}\0{info.st_size}\0{info.st_mtime_ns}\n".encode())
with open("/work/out/deps.sum", "w") as f:
    f.write(digest.hexdigest() + "\n")
PY

# Deterministic archives, so an unchanged layer keeps its digest and is neither pushed nor pulled.
if [[ -f /work/prev/deps.sum && -f /work/prev/deps.tar.zst ]] && cmp -s /work/prev/deps.sum /work/out/deps.sum; then
  echo "snapshot: third-party crates unchanged, reusing their archive"
  cp /work/prev/deps.tar.zst /work/out/deps.tar.zst
  layers=ws
else
  layers="deps ws"
fi
# Level 12 packs the third-party layer in about 20 s where 19 takes over 4 minutes, for 18 % more
# bytes, which the buildbox pulls from ghcr in about a second.
for layer in $layers; do
  tar -C "$snap" -c --format=posix --no-recursion --null -T "/tmp/$layer.list" \
    --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
    --owner=0 --group=0 --numeric-owner \
    | zstd -q -T4 -12 --long=27 -o "/work/out/$layer.tar.zst"
done
ls -la /work/out
