#!/usr/bin/env python3
"""Write cargo-sources.json for an offline Flatpak build from Cargo.lock.

A dependency-free stand-in for flatpak-builder-tools' flatpak-cargo-generator,
covering what this project needs: crates.io packages only (path dependencies
such as vendor/tiberius are part of the source tree already).

Usage: python3 flatpak/gen-cargo-sources.py [Cargo.lock] [cargo-sources.json]
"""

import json
import sys
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parent.parent
lock_path = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "Cargo.lock"
out_path = Path(sys.argv[2]) if len(sys.argv) > 2 else root / "flatpak" / "cargo-sources.json"

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
VENDOR = "cargo/vendor"

lock = tomllib.loads(lock_path.read_text())
sources = []
for pkg in lock["package"]:
    source = pkg.get("source")
    if source is None:
        continue  # path dependency, already in the tree
    if source != REGISTRY:
        sys.exit(f"unsupported source for {pkg['name']}: {source}")
    name, version, checksum = pkg["name"], pkg["version"], pkg["checksum"]
    dest = f"{VENDOR}/{name}-{version}"
    sources.append({
        "type": "archive",
        "archive-type": "tar-gzip",
        "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
        "sha256": checksum,
        "dest": dest,
    })
    sources.append({
        "type": "inline",
        "contents": json.dumps({"package": checksum, "files": {}}),
        "dest": dest,
        "dest-filename": ".cargo-checksum.json",
    })

sources.append({
    "type": "inline",
    "contents": (
        "[source.vendored-sources]\n"
        f'directory = "{VENDOR}"\n\n'
        "[source.crates-io]\n"
        'replace-with = "vendored-sources"\n'
    ),
    "dest": "cargo",
    "dest-filename": "config.toml",
})

out_path.write_text(json.dumps(sources, indent=4) + "\n")
print(f"wrote {out_path} ({(len(sources) - 1) // 2} crates)")
