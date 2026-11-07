#!/usr/bin/env python3
"""Print the plugins affected by the files changed since a git base, as a JSON array.

Usage: scripts/affected.py [base-ref]   (no base ref, or a global change, means every plugin)

A plugin is affected when its own directory changed, or any workspace crate it depends on
(directly or not) did. Changes to the workspace manifest, lockfile, toolchain config, bundler.toml
or xtask rebuild everything.
"""
import json
import os
import subprocess
import sys

root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=root, text=True))
members = set(meta["workspace_members"])
pkgs = {p["id"]: p for p in meta["packages"] if p["id"] in members}
by_dir = {os.path.relpath(os.path.dirname(p["manifest_path"]), root): pid for pid, p in pkgs.items()}
plugins = sorted(p["name"] for p in pkgs.values()
                 if os.path.relpath(os.path.dirname(p["manifest_path"]), root).startswith("plugins/"))

GLOBAL = ("Cargo.toml", "Cargo.lock", "bundler.toml", "rust-toolchain", "rust-toolchain.toml",
          ".cargo/", "xtask/", ".github/workflows/ci.yml", "scripts/affected.py")

base = sys.argv[1] if len(sys.argv) > 1 else ""
if not base or set(base) == {"0"}:
    print(json.dumps(plugins))
    sys.exit(0)

changed = subprocess.check_output(["git", "diff", "--name-only", base, "HEAD"], cwd=root, text=True).split()
if any(f == g or (g.endswith("/") and f.startswith(g)) for f in changed for g in GLOBAL):
    print(json.dumps(plugins))
    sys.exit(0)

dirty = set()
for f in changed:
    for d, pid in by_dir.items():
        if f.startswith(d + "/"):
            dirty.add(pid)

# Reverse-dependency closure over workspace members, matched by path dependency.
path_of = {pid: os.path.dirname(p["manifest_path"]) for pid, p in pkgs.items()}
deps = {pid: {q for q in pkgs for d in p["dependencies"]
              if d.get("path") and os.path.realpath(d["path"]) == os.path.realpath(path_of[q])}
        for pid, p in pkgs.items()}
grew = True
while grew:
    grew = False
    for pid, ds in deps.items():
        if pid not in dirty and ds & dirty:
            dirty.add(pid)
            grew = True

print(json.dumps(sorted(pkgs[pid]["name"] for pid in dirty if pkgs[pid]["name"] in plugins)))
