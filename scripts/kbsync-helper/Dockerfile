# Build context is scripts/kbsync-helper, not the repository root.
# The final image is Linux/glibc and includes its verified offline runtime cache.
FROM golang:1.26.5-bookworm@sha256:53eeac89074db483fdf0ab3be1df32bf6e47562263d2d0d6baa7f26acb4957dd AS build
ARG UNICORN_MIRROR=pypi
RUN apt-get update && apt-get install -y --no-install-recommends python3-minimal curl ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /build/helper
ENV TMPDIR=/build/cache GOCACHE=/build/cache/go-build GOMODCACHE=/build/cache/go-mod XDG_CACHE_HOME=/runtime-cache
COPY . .
RUN GO=go ./build.sh
# Optional Linux/amd64 prefetch bypasses upstream's shorter download timeout.
# On other architectures upstream downloads its own matching Unicorn artifact.
RUN if [ "$(uname -m)" = x86_64 ]; then python3 prefetch-unicorn.py --mirror "$UNICORN_MIRROR"; fi
# Prime Apple assets and prove guest execution; discard the dummy blob entirely.
RUN printf '%s' '{"guid":"AABBCCDDEEFF","dsid":"123456789"}' | ./bin/ipatool-kbsync -timeout 10m >/dev/null

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
COPY --from=build /build/helper/bin/ipatool-kbsync /usr/local/bin/ipatool-kbsync
COPY --from=build --chown=65532:65532 /runtime-cache /opt/ipatool-cache
COPY LICENSE UNICORN-COPYING /usr/share/doc/ipatool-kbsync/
ENV XDG_CACHE_HOME=/opt/ipatool-cache
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/ipatool-kbsync"]
