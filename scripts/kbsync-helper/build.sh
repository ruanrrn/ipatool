#!/bin/sh
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
: "${GO:=go}"
: "${TMPDIR:=${HOME}/.hermes/cache/scratch}"
: "${GOCACHE:=${TMPDIR}/kbsync-go-build}"
: "${GOMODCACHE:=${TMPDIR}/kbsync-go-mod}"
export TMPDIR GOCACHE GOMODCACHE
mkdir -p "$TMPDIR" "$GOCACHE" "$GOMODCACHE" bin
python3 verify-source.py
"$GO" mod verify
"$GO" test -mod=readonly ./...
"$GO" build -mod=readonly -trimpath -buildvcs=false -o bin/ipatool-kbsync .
