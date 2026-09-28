# syntax=docker/dockerfile:1

# ---- builder: Alpine (musl) ----
FROM rust:1.97-alpine AS builder
RUN apk add --no-cache build-base musl-dev cmake perl clang clang22-libclang git
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# bindgen (used by btls-sys) dlopens libclang at build time, which fails in a
# fully-static musl binary; link against musl/libgcc dynamically instead.
ENV RUSTFLAGS="-C target-feature=-crt-static"
RUN cargo build --release --locked \
    && strip target/release/reddit

# ---- runtime: bare Alpine + musl-linked binary ----
FROM alpine:3.24
RUN apk add --no-cache ca-certificates-bundle libgcc libstdc++ \
    && addgroup -S -g 10001 rduser \
    && adduser -S -D -u 10001 -G rduser rduser
WORKDIR /data
COPY --from=builder /build/target/release/reddit /usr/local/bin/reddit
USER rduser
VOLUME /data
ENTRYPOINT ["reddit"]
CMD ["--help"]
