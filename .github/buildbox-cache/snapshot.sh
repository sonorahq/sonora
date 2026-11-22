#!/usr/bin/env bash
# Builds the checkout the way rbuild's plain mode builds it on the buildbox and packs the cargo
# target into two layers. Runs as root in an archlinux container started by buildbox-cache.yml:
#
#   /checkout          the commit to build, read only
#   /work/prev/        the previous snapshot's deps.tar.zst and ws.tar.zst, when they can seed this one
#   /work/out/         receives deps.tar.zst and ws.tar.zst
#
# deps.tar.zst holds the third-party crates, so its bytes only change when they are rebuilt and the
# buildbox skips downloading it otherwise. ws.tar.zst holds the workspace crates, the top-level
# outputs and the sources the build saw. The incremental caches are left out: they are big, and a
# missing one only makes the next rebuild of that crate a full one.
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
  for layer in deps ws; do zstd -dc "/work/prev/$layer.tar.zst" | tar -x -C "$snap"; done
else
  echo "snapshot: no matching snapshot, building cold"
fi
mkdir -p "$snap/src" "$snap/target"
# Same comparison as rbuild: unchanged files keep the mtimes the seed was built with.
rsync -rlpc --delete --exclude=/.git /checkout/ "$snap/src/"

cd "$snap/src"
rustup toolchain install
cargo build --locked -p sonora
cargo clippy --locked --workspace --all-targets
cargo test --locked --workspace --no-run

# Splits every file under $snap into the two layers, by the names cargo gives workspace units.
cargo metadata --locked --no-deps --format-version 1 > /tmp/metadata.json
python - "$snap" /tmp/metadata.json <<'PY'
import json, os, re, sys

snap, meta = sys.argv[1], json.load(open(sys.argv[2]))
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
    for name in files + [d for d in dirs if os.path.islink(os.path.join(root, d))]:
        rel = os.path.normpath(os.path.join(rel_root, name))
        top_level = os.path.dirname(rel) in ("target", "target/debug")
        if rel.startswith("src/") or top_level or workspace_unit.match(rel):
            layers["ws"].append(rel)
        else:
            layers["deps"].append(rel)
for layer, paths in layers.items():
    with open(f"/tmp/{layer}.list", "wb") as f:
        f.write(b"".join(p.encode() + b"\0" for p in sorted(paths)))
    print(f"snapshot: {layer} layer has {len(paths)} files")
PY

# Deterministic archives, so an unchanged layer keeps its digest and is neither pushed nor pulled.
for layer in deps ws; do
  tar -C "$snap" -c --format=posix --no-recursion --null -T "/tmp/$layer.list" \
    --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
    --owner=0 --group=0 --numeric-owner \
    | zstd -q -T4 -10 -o "/work/out/$layer.tar.zst"
done
ls -la /work/out
