# syntax=docker/dockerfile:1

# ---- builder: Alpine (musl) ----
FROM rust:1.98.1-alpine3.24 AS builder
RUN apk add --no-cache build-base musl-dev cmake perl clang clang22-libclang git
WORKDIR /build
ARG TARGETARCH
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# bindgen (btls-sys) dlopens libclang from build scripts, which needs a
# dynamically linked musl host: with crt-static it fails with
# "Dynamic loading not supported".
ENV RUSTFLAGS="-C target-feature=-crt-static"
# Parallel architectures must not unpack crates or overwrite binaries in the
# same cache. Lock each architecture's mounts against concurrent builds.
RUN --mount=type=cache,id=reddit-cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=reddit-cargo-target-${TARGETARCH},target=/build/target,sharing=locked \
    cargo build --release --locked \
    && cp target/release/reddit /usr/local/bin/reddit

# ---- runtime: bare Alpine + the musl binary ----
FROM alpine:3.24
RUN apk add --no-cache ca-certificates-bundle libgcc libstdc++ \
    && addgroup -S -g 10001 rduser \
    && adduser -S -D -u 10001 -G rduser rduser
WORKDIR /data
# archives land directly in the mounted volume instead of /data/output
ENV REDDIT_OUT_DIR=/data
COPY --from=builder /usr/local/bin/reddit /usr/local/bin/reddit
USER rduser
VOLUME /data
ENTRYPOINT ["reddit"]
CMD ["--help"]
