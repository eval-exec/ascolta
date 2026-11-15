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
    cargo test -p ascolta-mpd --test live -- --ignored --nocapture

run:
    cargo run -p ascolta

run-release:
    cargo run --release -p ascolta

release:
    cargo build --release -p ascolta

check-mpd:
    cargo run -q -p ascolta -- --check
