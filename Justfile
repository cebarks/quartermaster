default:
    @just --list

build:
    cargo build

check:
    cargo check

test:
    cargo test

clippy:
    cargo clippy -- -D warnings

fmt:
    cargo fmt

lint: fmt clippy

run *ARGS:
    cargo run -- {{ARGS}}
