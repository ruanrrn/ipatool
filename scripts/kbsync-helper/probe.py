#!/usr/bin/env python3
"""Run the actual emulator with dummy identities; never print or save blobs."""
import argparse
import base64
import json
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--runs", type=int, default=1)
parser.add_argument("--helper", default=str(Path(__file__).resolve().parent / "ipatool-kbsync"))
args = parser.parse_args()
if args.runs < 1:
    parser.error("runs must be positive")
failed = False
for index in range(args.runs):
    # Alternate JSON string/number forms; Python preserves the exact integer.
    dsid = "123456789" if index % 2 == 0 else 123456789
    started = time.monotonic()
    proc = subprocess.run(
        [args.helper, "-timeout", "10m"],
        input=json.dumps({"guid": "AABBCCDDEEFF", "dsid": dsid}),
        text=True, capture_output=True, timeout=660,
    )
    result = {
        "run": index + 1,
        "dsid_json_type": type(dsid).__name__,
        "exit_code": proc.returncode,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "stdout_bytes": len(proc.stdout.encode()),
        "stderr": proc.stderr,
    }
    if proc.returncode == 0:
        obj = json.loads(proc.stdout)
        assert set(obj) == {"kbsync"}, "unexpected output keys"
        blob = base64.b64decode(obj["kbsync"], validate=True)
        assert blob, "empty kbsync"
        result["decoded_blob_bytes"] = len(blob)
        result["json_contract_valid"] = True
        del blob, obj
    else:
        failed = True
    del proc
    print(json.dumps(result), flush=True)
raise SystemExit(1 if failed else 0)
