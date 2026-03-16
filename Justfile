set shell := ["bash", "-lc"]

sync:
    gh repo sync

build:
    cargo build --release

smoke:
    @bash -lc 'set -euo pipefail; sudo ./target/release/miniboxd & pid=$!; sleep 1; sudo ./target/release/minibox ps; sudo kill $pid; wait $pid || true'

test:
    cargo test --workspace

sync-smoke-test: sync build smoke test
