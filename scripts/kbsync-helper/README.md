# Standalone upstream kbsync helper

## Verified capability and limits

This helper **really runs** upstream `machine.GenerateKBSync` in the ARM64
StoreAgent/Unicorn guest on Linux amd64. It is not a placeholder, reimplementation,
or call to a remote kbsync service. No Apple login, token, password, purchase,
account database, or download authorization request is used by the probe.
A dummy DSID producing a blob proves generation works, **not** that a particular
Apple account can download or that ent/download accepts the result.

The module is standalone. Its pinned source lives here; building/running does
not require `/root/repo/ipatool-upstream-reference` or any other Git checkout.
Nothing in Rust, production services, or deployment files is changed by this helper.

## Protocol for Rust

One process handles one request. Write a JSON object to stdin, **close stdin**, and
capture stdout. Example input (dummy identity):

```json
{"guid":"AABBCCDDEEFF","dsid":"123456789"}
```

`dsid` can instead be an integer JSON number, parsed directly as uint64 without
floating-point conversion. Prefer a string across languages. It must be nonzero;
negative, fractional, exponent, overflow, null, and whitespace-containing DSIDs
are rejected. GUID must be exactly 12 hex digits (six bytes); colon-separated MAC
addresses are not normalized. Input is limited to 4096 bytes and one request.

Success: exit 0; stdout is a single JSON line `{"kbsync":"<base64>"}`; stderr is
empty. Failure: nonzero exit, no success payload, diagnostic on stderr. Never log
stdout or persist it as a diagnostic. A result is bound to the exact GUID and DSID;
do not use a generated dummy blob for a real account or share blobs across accounts.

Default overall timeout is 10 minutes, allowing cold asset retrieval. Override
with `-timeout 2m`, for example. Upstream's one-minute **per guest invocation**
limit is unchanged. Raising overall timeout does not raise that guest limit.
Rust should impose a child-process deadline and kill/wait on a stuck helper.

## Build / launch

Requirements: Go 1.26.5 or compatible newer Go; on the verified default Linux build,
a C compiler at build time and glibc runtime. Python 3 is used by build verification
and the safe probe. `curl` is only needed by optional runtime prefetch.
No system `libunicorn-dev`, Python Unicorn import, pip, or system package install
is needed: upstream loads a downloaded, SHA256-pinned shared library with purego.

On the machine where this was built:

```sh
cd /root/repo/ipatool/scripts/kbsync-helper
GO=/root/repo/toolchains/go1.26.5/go/bin/go ./build.sh
python3 probe.py --runs 2
```

The executable binary is:

`/root/repo/ipatool/scripts/kbsync-helper/bin/ipatool-kbsync`

The ready-to-run launcher (selects the warmed scratch cache) is:

`/root/repo/ipatool/scripts/kbsync-helper/ipatool-kbsync`

The launcher resolves `bin/ipatool-kbsync` relative to itself. If integration uses
an executable named `ipatool-kbsync` beside the Rust server, copy/install the
**binary**, not this relative-path launcher. No such installation or deployment
has been done here. The native binary respects `XDG_CACHE_HOME`; the launcher
selects `XDG_CACHE_HOME`, then `KBSYNC_CACHE_HOME`, then
`${TMPDIR:-$HOME/.hermes/cache/scratch}/kbsync-runtime-cache`.

The current default warm cache is:

`/root/.hermes/cache/scratch/kbsync-runtime-cache`

Scratch caches are disposable and may be pruned. To avoid a network-dependent
first real download, prewarm the service user's eventual cache using this dummy
probe and an explicit `XDG_CACHE_HOME`, preserving permissions. Do not rely on root's
cache being accessible to a different service user.

### Reproducibility

Upstream: <https://github.com/majd/ipatool>

Pinned commit: `5dc4c8058db3a23e9de081ac766cc917713a65cf`

`internal/sap/{assets,machine,machimage,unicorn}` contains unmodified non-test Go
files from that commit; unused root signer files and upstream tests are omitted.
`LICENSE` is the upstream MIT license. `upstream-sha256.json` pins every included
upstream file; `verify-source.py` verifies the manifest and source-file set.
`go.mod` and `go.sum` pin Go dependencies; `build.sh` checks source integrity,
verifies downloaded modules, runs local tests, then builds with `-trimpath` and
`-buildvcs=false`. No replace directive references an untracked source checkout.
Build artifacts under `bin/` are intentionally gitignored.

The locally installed official Go archive is `go1.26.5.linux-amd64.tar.gz` from
`https://dl.google.com/go/go1.26.5.linux-amd64.tar.gz`, verified against Go's
`https://go.dev/dl/?mode=json&include=all` metadata:

`5c2c3b16caefa1d968a94c1daca04a7ca301a496d9b086e17ad77bb81393f053`

For another machine, download that archive, verify this SHA256, extract under a
user-selected toolchain directory, and pass its `go/bin/go` via `GO=...`.
The toolchain here was isolated under `/root/repo/toolchains/go1.26.5`; no system
Go install or user PATH was overwritten.

## Runtime assets and first-run networking

Upstream fetches pinned macOS 27.0.1 build 26A434 assets using HTTP Range against
Apple's restore IPSW, extracting the required files instead of downloading the
whole restore image. The four cached framework files are individually size/SHA256
checked by upstream (`internal/sap/assets/assets.go`). No Apple proprietary binary
is included in this source directory.

Unicorn 2.1.4's wheel and native library also have upstream-pinned hashes. On this
network the default `files.pythonhosted.org` transfer was too slow for upstream's
independent two-minute HTTP client limit: initial actual run failed at 142.340s
with empty stdout and a `save Unicorn download: context deadline exceeded` error.
That was a dependency-download problem, not guest execution failure.

Optional workaround, tested on Linux/glibc amd64:

```sh
python3 prefetch-unicorn.py --mirror aliyun
python3 probe.py --runs 2
```

This downloads the **same** upstream wheel using a mirror, verifies both wheel and
library against upstream SHA256, and atomically installs the content-addressed
library into the normal runtime cache. It does not install a Python package or
patch vendored runtime code. Default `--mirror pypi` uses the official host with a
longer prefetch timeout. Other host platforms use upstream's auto-download.

## Actual execution evidence

All successful probes used dummy GUID `AABBCCDDEEFF`, DSID `123456789`. No blob was
printed to tool logs or saved. Only JSON contract validation, lengths, and timings
were reported. Results from the final built binary:

| Scenario | Result | Time | Output |
|---|---|---:|---|
| Existing warm cache, string DSID | exit 0 | 2.819s | JSON 298 bytes; decoded blob 212 bytes; stderr empty |
| Existing warm cache, numeric DSID | exit 0 | 2.180s | JSON 298 bytes; decoded blob 212 bytes; stderr empty |
| Fresh Apple asset cache, only verified Unicorn prefetched | exit 0 | 20.261s | JSON 298 bytes; decoded blob 212 bytes; stderr empty |
| Same fresh cache, second run, numeric DSID | exit 0 | 2.179s | JSON 298 bytes; decoded blob 212 bytes; stderr empty |

Before the final VCS-metadata-free rebuild, a warm-cache run with HTTP/HTTPS/ALL
proxies set to unreachable `127.0.0.1:9` also succeeded (2.157s, 212 decoded bytes),
confirming no credential/auth service is necessary once runtime assets are cached.
Malformed JSON, zero DSID, and a 1ns expired context each returned exit 1 with zero
stdout. Local protocol/parser tests and `go vet ./...` passed; vendored upstream
packages have no copied tests, so these are not claims of upstream test-suite coverage.

Final local binary SHA256:

`58a2f0c3a7239ca6fc8d8b33758777ca0c14cc581d9fa0bcab1a9d22d0b75581`

## Standalone local bundle and Docker

A ready-to-run local-only archive is generated with `python3 package-runtime.py`:

`/root/repo/ipatool/scripts/kbsync-helper/bin/ipatool-kbsync-linux-amd64.tar.gz`

It includes the binary, all four Apple framework assets, the verified native
Unicorn `.so`, notices, a relocatable launcher, a pinned-base Dockerfile, and a
`SHA256.json` manifest for every payload file. Extract it and execute
`ipatool-kbsync-bundle/ipatool-kbsync`; no Go, Python, pip, credentials, network,
external reference checkout, or system Unicorn install is needed at runtime.
The bundle's native library path relative to its root is:

`cache/ipatool/unicorn/2.1.4/ddb196ec82b52e502c18e4a34478bf7b9f61c83c2ebaa95c74d8ded45a95da9c/libunicorn.so.2`

This is a **local test bundle** containing proprietary Apple binaries. It is
ignored by Git. Do not commit, publish, or redistribute it. Source-only builds
fetch pinned Apple assets directly. The Unicorn GPLv2 notice is included as
`UNICORN-COPYING` (from upstream tag `2.1.4`); upstream helper Go code is MIT.

Docker options:

1. **Verified offline runtime image**: extract the bundle and build its included
   Dockerfile from the extracted `ipatool-kbsync-bundle` directory:
   `docker build -t ipatool-kbsync:local .`. The base Debian Bookworm slim manifest
   is pinned by digest; the bundle contains every helper/runtime dependency.
   Run with `docker run --rm --network=none --read-only -i ipatool-kbsync:local`,
   passing the same JSON stdin protocol and capturing stdout. Final user is
   unprivileged `65532:65532`.
2. **Source-only multi-stage build**: from this source directory run
   `docker build --build-arg UNICORN_MIRROR=aliyun -t ipatool-kbsync:source .`.
   `Dockerfile` pins both Go 1.26.5 Bookworm and Debian Bookworm slim base digests,
   builds/verifies the standalone module, prefetches verified Unicorn, and primes
   pinned Apple assets with a dummy generation whose stdout is discarded. The
   final image copies only binary, runtime cache, and licenses. This full
   source-only Docker build recipe was **not** exercised in this task; the native
   source build and bundled-runtime Docker build/run were exercised separately.

Actual bundle verification: extraction into an independent scratch directory,
all 12 payload checksums verified, then a relocated launcher run succeeded with
unreachable HTTP proxies (2.267s; decoded blob 212 bytes). The bundled Docker
image `ipatool-kbsync-probe:local` was built and inspected, then dummy generation
succeeded as user `65532:65532`, network disabled, read-only filesystem (2.678s;
stdout JSON 298 bytes, decoded blob 212 bytes, no stderr).
This host is a nested container whose AppArmor profile loader cannot install the
Docker default profile; only the **test container** needed
`--security-opt apparmor=unconfined`. No host security setting was changed.
A normal Docker host should not require that flag.

Remaining validation is the caller's real-account ent/download request with the
matching GUID/DSID and valid session. This task intentionally did not attempt that.
