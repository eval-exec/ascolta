default:
    @just --list

fmt:
    cargo fmt --all

check:
    cargo check --workspace --all-targets

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo nextest run --workspace
    cargo test --workspace --doc

live-test:
    cargo test -p neo-ncmp-mpd --test live -- --ignored --nocapture

run:
    cargo run -p neo-ncmp

run-release:
    cargo run --release -p neo-ncmp

release:
    cargo build --release -p neo-ncmp

check-mpd:
    cargo run -q -p neo-ncmp -- --check
