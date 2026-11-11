# Built from the repository root: docker build -f crates/horizon-mod-registry/Dockerfile .
FROM rust:1-slim AS build
WORKDIR /src
# The root manifest only for the shared crate's workspace settings (edition, lints).
COPY Cargo.toml Cargo.toml
COPY crates/horizon-core crates/horizon-core
COPY crates/horizon-game-discovery crates/horizon-game-discovery
COPY crates/horizon-mod-api-gen crates/horizon-mod-api-gen
COPY crates/horizon-mod-api-macros crates/horizon-mod-api-macros
COPY crates/horizon-mod-point-shape crates/horizon-mod-point-shape
COPY crates/horizon-mod-runtime crates/horizon-mod-runtime
COPY crates/horizon-mod-vocab crates/horizon-mod-vocab
# Every crate the registry's Cargo.lock builds from a path (tests/test_service_images.py checks the list); cargo needs
# an optional dependency's manifest even with its feature off (horizon-shader-api).
COPY crates/horizon-mod-package crates/horizon-mod-package
COPY crates/horizon-release-sign crates/horizon-release-sign
COPY crates/horizon-shader-api crates/horizon-shader-api
COPY crates/horizon-mod-registry crates/horizon-mod-registry
RUN cargo build --release --locked --manifest-path crates/horizon-mod-registry/Cargo.toml

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/crates/horizon-mod-registry/target/release/horizon-mod-registry /usr/local/bin/
ENV REGISTRY_DB=/data/registry.sqlite3
# Railway mounts the ratings' volume at /data owned by root: hand it to nobody, then run as nobody (setpriv is
# util-linux's, in every Debian image).
CMD ["sh", "-c", "mkdir -p /data && chown nobody:nogroup /data && exec setpriv --reuid=nobody --regid=nogroup --clear-groups horizon-mod-registry"]
