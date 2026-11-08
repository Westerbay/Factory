FROM rust:1.90.0-bookworm@sha256:3914072ca0c3b8aad871db9169a651ccfce30cf58303e5d6f2db16d1d8a7e58f AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN rustup component add clippy rustfmt
COPY apps/api ./apps/api
COPY crates ./crates
COPY infra/environments/feature-launcher/launcher.py ./infra/environments/feature-launcher/launcher.py
RUN cargo build --locked --release --package factory-api --bins

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
RUN printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20261027T000000Z bookworm main' > /etc/apt/sources.list \
    && rm /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/factory /src/target/release/factory-api /usr/local/bin/
WORKDIR /var/lib/factory
ENTRYPOINT ["factory-api"]
