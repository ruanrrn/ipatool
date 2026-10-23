#!/usr/bin/env python3
"""Verify vendored upstream files against the pinned-source manifest."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
manifest = json.loads((root / "upstream-sha256.json").read_text())
for name, expected in manifest.items():
    actual = hashlib.sha256((root / name).read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"upstream source integrity check failed: {name}")
actual_sources = {str(p.relative_to(root)) for p in (root / "internal").rglob("*.go")}
if actual_sources != {name for name in manifest if name.endswith(".go")}:
    raise SystemExit("upstream source manifest does not match included Go files")
print(f"Verified {len(manifest)} pinned upstream source/license files")
