#!/usr/bin/env python3
"""Optional pinned Unicorn prefetch for Linux/glibc amd64, not a pip install."""
import argparse
import hashlib
import os
import platform
import subprocess
import tempfile
import zipfile
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--mirror", choices=["pypi", "aliyun"], default="pypi")
args = parser.parse_args()
if platform.system() != "Linux" or platform.machine() != "x86_64" or platform.libc_ver()[0] != "glibc":
    raise SystemExit("This optional prefetch is pinned to Linux/glibc amd64; other hosts use upstream auto-download")
archive_sha = "9d6e6dea140560de4ebd8446661f7ef84a357d428c14a3ef09dacd306ec8c239"
library_sha = "ddb196ec82b52e502c18e4a34478bf7b9f61c83c2ebaa95c74d8ded45a95da9c"
member = "unicorn/lib/libunicorn.so.2"
relative = "packages/e7/df/ded5e3684c2d7600b30cc8a7530277b8cb36644a1a9d34cade7ebb45604c/unicorn-2.1.4-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl"
base = "https://files.pythonhosted.org/" if args.mirror == "pypi" else "https://mirrors.aliyun.com/pypi/"
scratch = Path(os.environ.get("TMPDIR", str(Path.home() / ".hermes/cache/scratch")))
cache = Path(os.environ.get("XDG_CACHE_HOME", os.environ.get("KBSYNC_CACHE_HOME", str(scratch / "kbsync-runtime-cache"))))
dest = cache / "ipatool/unicorn/2.1.4" / library_sha / "libunicorn.so.2"
scratch.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="kbsync-unicorn-", dir=scratch) as directory:
    archive = Path(directory) / "unicorn.whl"
    subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location", "--max-time", "600", "--output", str(archive), base + relative], check=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != archive_sha:
        raise SystemExit("Unicorn wheel SHA256 mismatch; refusing installation")
    with zipfile.ZipFile(archive) as wheel:
        library = wheel.read(member)
    if hashlib.sha256(library).hexdigest() != library_sha:
        raise SystemExit("Unicorn library SHA256 mismatch; refusing installation")
    dest.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.NamedTemporaryFile(prefix=".prefetch-", dir=dest.parent, delete=False) as file:
        temporary = Path(file.name)
        file.write(library)
    try:
        os.replace(temporary, dest)
    finally:
        temporary.unlink(missing_ok=True)
print(f"Installed verified Unicorn 2.1.4 library: {dest} ({len(library)} bytes)")
