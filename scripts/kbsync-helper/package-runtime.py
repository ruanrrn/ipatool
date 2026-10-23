#!/usr/bin/env python3
"""Package the verified local binary and runtime cache for offline local use."""
import gzip
import hashlib
import io
import json
import os
import tarfile
from pathlib import Path

root = Path(__file__).resolve().parent
scratch = Path(os.environ.get("TMPDIR", str(Path.home() / ".hermes/cache/scratch")))
cache = Path(os.environ.get("XDG_CACHE_HOME", os.environ.get("KBSYNC_CACHE_HOME", str(scratch / "kbsync-runtime-cache"))))
profile = "ipatool/sap/apple-assets-macos-27-26A434-arm64-v1"
expected = {
    f"{profile}/AppleMediaServices": "95f4558aa2c0ccd9f96ed479e336b630b46bf4b4e0baabc1c1ac8c7d55e21ab4",
    f"{profile}/commerce": "76de40a4a32f691947cb926ce3b128d2fc446f7cac9755f831c406cf5a316ac9",
    f"{profile}/CoreFP": "2c30fa2b5f695fd66c0360dce72f54757d03ddb7107549715c12cb359a003b46",
    f"{profile}/CoreFP.icxs": "cb8f55330ec567da3e692dab5f5388bed0312dd9093340477c5759078cc2aa7b",
    "ipatool/unicorn/2.1.4/ddb196ec82b52e502c18e4a34478bf7b9f61c83c2ebaa95c74d8ded45a95da9c/libunicorn.so.2": "ddb196ec82b52e502c18e4a34478bf7b9f61c83c2ebaa95c74d8ded45a95da9c",
}
files = {"libexec/ipatool-kbsync": (root / "bin/ipatool-kbsync").read_bytes()}
for name, digest in expected.items():
    data = (cache / name).read_bytes()
    if hashlib.sha256(data).hexdigest() != digest:
        raise SystemExit(f"runtime asset checksum mismatch: {name}")
    files["cache/" + name] = data
files["ipatool-kbsync"] = b'''#!/bin/sh
set -eu
root="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$root/cache}"
exec "$root/libexec/ipatool-kbsync" "$@"
'''
files["README.md"] = (root / "README.md").read_bytes()
files["Dockerfile"] = (root / "Dockerfile.bundle").read_bytes()
files["UPSTREAM-LICENSE"] = (root / "LICENSE").read_bytes()
files["UNICORN-COPYING"] = (root / "UNICORN-COPYING").read_bytes()
files["LOCAL-USE-NOTICE.txt"] = b"This local runtime bundle contains Apple-extracted proprietary framework binaries. Do not commit or publicly redistribute it. Source-only builds fetch them directly from Apple. Unicorn GPLv2 notice is supplied as UNICORN-COPYING.\n"
# The source-only repository does not vendor or redistribute these binary assets.
files["SHA256.json"] = (json.dumps({name: hashlib.sha256(data).hexdigest() for name, data in files.items()}, indent=2, sort_keys=True) + "\n").encode()
output = root / "bin/ipatool-kbsync-linux-amd64.tar.gz"
with output.open("wb") as raw, gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w|") as archive:
    for name, data in sorted(files.items()):
        info = tarfile.TarInfo("ipatool-kbsync-bundle/" + name)
        info.size = len(data)
        info.mode = 0o755 if name in {"ipatool-kbsync", "libexec/ipatool-kbsync"} else 0o644
        archive.addfile(info, io.BytesIO(data))
print(json.dumps({"artifact": str(output), "bytes": output.stat().st_size, "sha256": hashlib.sha256(output.read_bytes()).hexdigest(), "file_count": len(files)}))
