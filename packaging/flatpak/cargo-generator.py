#!/usr/bin/env python3
"""Run flatpak-cargo-generator with workspace dependencies inherited the way Cargo does.

The upstream script lets the workspace entry overwrite every key a member sets, so a member's
`default-features = true` over a workspace `default-features = false` is lost. The vendored
crates then resolve to a different lock and `cargo build --locked` refuses to run.
"""

import sys

from flatpak_cargo_generator import script


def update_workspace_keys(pkg, workspace):
    """Inline every `workspace = true` dependency under `pkg`, keeping what the member sets."""
    for key, item in list(pkg.items()):
        if not isinstance(item, dict):
            continue
        if key == "target":
            for target in item.values():
                update_workspace_keys(target, workspace)
            continue
        if key in ("dev-dependencies", "build-dependencies"):
            update_workspace_keys(item, (workspace or {}).get("dependencies"))
            continue
        if not workspace or key not in workspace:
            continue

        inherited = workspace[key]
        if "workspace" not in item:
            update_workspace_keys(item, inherited)
            continue

        del item["workspace"]
        if not isinstance(inherited, dict):
            # A package field such as `edition` takes the value, a dependency takes it as its version.
            if item:
                item["version"] = inherited
            else:
                pkg[key] = inherited
            continue
        member_default = item.pop("default-features", item.pop("default_features", False))
        for name, value in inherited.items():
            if name in ("default-features", "default_features"):
                continue
            if name == "features":
                item["features"] = value + item.get("features", [])
            else:
                item.setdefault(name, value)
        # A member can turn default features on over the workspace but never off.
        if inherited.get("default-features", inherited.get("default_features")) is False:
            item["default-features"] = bool(member_default)


script.update_workspace_keys = update_workspace_keys

if __name__ == "__main__":
    sys.exit(script.main())
