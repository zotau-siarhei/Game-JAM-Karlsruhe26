#!/usr/bin/env bash
# Собирает игру под браузер в web/pkg. Раздать: python3 -m http.server --directory web
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build --profile wasm-release --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-name lunokhod --out-dir web/pkg \
    target/wasm32-unknown-unknown/wasm-release/lunokhod.wasm
