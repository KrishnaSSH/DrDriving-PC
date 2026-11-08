#!/usr/bin/env bash
# Builds the web version into web/dist (serve that folder with any static file server).
# Needs: the wasm32 Rust target (`rustup target add wasm32-unknown-unknown`).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p drdriving --target wasm32-unknown-unknown
out=web/dist
rm -rf "$out"
mkdir -p "$out"
cp target/wasm32-unknown-unknown/release/drdriving.wasm "$out/"
cp web/index.html "$out/"
# the JS glue must match the macroquad version that was compiled
mq=$(cargo metadata --format-version 1 --locked | python3 -c '
import json,sys
m=json.load(sys.stdin)
print(next(p["manifest_path"] for p in m["packages"] if p["name"]=="macroquad"))')
cp "$(dirname "$mq")/js/mq_js_bundle.js" "$out/"
cp -r assets "$out/assets"
touch "$out/.nojekyll"
echo "built $out  (try: python3 -m http.server -d $out)"
