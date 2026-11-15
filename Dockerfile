# syntax=docker/dockerfile:1
FROM rust:1.89-slim-bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked --bin ascolta

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 ascolta

COPY --from=builder /src/target/release/ascolta /usr/local/bin/ascolta
COPY docs/ascolta.1 /usr/local/share/man/man1/ascolta.1
COPY config.example.toml /usr/local/share/doc/ascolta/config.example.toml

ENV TERM=xterm-256color \
    XDG_CONFIG_HOME=/config \
    XDG_STATE_HOME=/state \
    XDG_CACHE_HOME=/cache

USER ascolta
VOLUME ["/config", "/state", "/cache"]
ENTRYPOINT ["ascolta"]
