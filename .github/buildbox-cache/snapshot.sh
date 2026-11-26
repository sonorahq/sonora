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
# The JSON messages name every unit the build used, which the pruning below keeps. A plain check
# resolves other features than the --all-targets runs, so its units are built too.
for build in "build --locked -p sonora" "check --locked --workspace" \
  "clippy --locked --workspace --all-targets" "test --locked --workspace --no-run"; do
  cargo $build --message-format=json-render-diagnostics >> /tmp/units.jsonl
done

python - "$snap" /tmp/units.jsonl <<'PY'
import hashlib, json, os, re, shutil, sys

snap = sys.argv[1]
units_root = os.path.join(snap, "target/debug/build")


def unit_of(path):
    """Returns the unit folder that holds path, as `<package>/<hash>` under target/debug/build, or None."""
    parts = os.path.relpath(path, units_root).split(os.sep)
    return "/".join(parts[:2]) if len(parts) > 2 and parts[0] != ".." else None


# Cargo keeps each unit in target/debug/build/<package>/<hash> and names in its JSON messages every
# unit this build used. A bin or an example is named only by its copy uplifted out of the unit,
# which shares an inode with the file inside it. Units of path packages are the workspace's own.
used, workspace, uplifted = set(), set(), {}
for line in open(sys.argv[2]):
    if not line.startswith("{"):
        continue
    msg = json.loads(line)
    if msg["reason"] == "compiler-artifact":
        paths = msg["filenames"] + ([msg["executable"]] if msg.get("executable") else [])
    elif msg["reason"] == "build-script-executed":
        paths = [msg["out_dir"]]
    else:
        continue
    local = msg["package_id"].startswith("path+")
    for path in paths:
        if unit := unit_of(path):
            used.add(unit)
            if local:
                workspace.add(unit)
        elif os.path.exists(path):
            uplifted[os.stat(path).st_ino] = local
unit_hash = re.compile(r"[0-9a-f]{16}")
units = [
    f"{package.name}/{unit.name}"
    for package in os.scandir(units_root)
    if package.is_dir(follow_symlinks=False)
    for unit in os.scandir(package.path)
    if unit.is_dir(follow_symlinks=False) and unit_hash.fullmatch(unit.name)
]
for unit in units:
    out = os.path.join(units_root, unit, "out")
    if unit in used or not os.path.isdir(out):
        continue
    for entry in os.scandir(out):
        if entry.is_file(follow_symlinks=False) and (local := uplifted.get(entry.inode())) is not None:
            used.add(unit)
            if local:
                workspace.add(unit)
            break
if len(used) < 100:
    sys.exit(f"snapshot: cargo named only {len(used)} units, refusing to prune")
pruned = [unit for unit in units if unit not in used]
for unit in pruned:
    shutil.rmtree(os.path.join(units_root, unit))
for package in {unit.split("/")[0] for unit in pruned}:
    if not os.listdir(os.path.join(units_root, package)):
        os.rmdir(os.path.join(units_root, package))
print(f"snapshot: kept {len(used)} units, removed {len(pruned)} units no longer built")

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
        # Split debuginfo is read only by a debugger.
        if rel.endswith(".dwo"):
            continue
        # The sources, the uplifted bins and examples, and the workspace units go with the workspace.
        unit = unit_of(os.path.join(snap, rel))
        layers["ws" if unit is None or unit in workspace else "deps"].append(rel)
for layer, paths in layers.items():
    with open(f"/tmp/{layer}.list", "wb") as f:
        f.write(b"".join(p.encode() + b"\0" for p in sorted(paths)))
    print(f"snapshot: {layer} layer has {len(paths)} files")

# When this sum matches the previous snapshot's, its third-party archive is reused as it is.
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
