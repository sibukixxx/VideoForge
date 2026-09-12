#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
rustup target add wasm32-unknown-unknown
cargo build -p videoforge-timeline-wasm --target wasm32-unknown-unknown --release

cd experiments/micro-wasm
command -v wasm-pack >/dev/null || {
  echo "wasm-pack is required: cargo install wasm-pack" >&2
  exit 1
}
npm run build
npm run smoke
npm run benchmark
npm run size
