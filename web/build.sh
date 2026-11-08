#!/usr/bin/env bash
# Builds the web version into web/dist (serve that folder with any static file server).
# Needs: the wasm32 Rust target (`rustup target add wasm32-unknown-unknown`) and python3.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p drdriving --target wasm32-unknown-unknown
out=web/dist
rm -rf "$out"
mkdir -p "$out"
cp target/wasm32-unknown-unknown/release/drdriving.wasm "$out/"
# the JS glue must match the macroquad version that was compiled
mq=$(cargo metadata --format-version 1 --locked | python3 -c '
import json,sys
m=json.load(sys.stdin)
print(next(p["manifest_path"] for p in m["packages"] if p["name"]=="macroquad"))')
cp "$(dirname "$mq")/js/mq_js_bundle.js" "$out/"

# All assets go into ONE file (a single request with real byte progress instead of ~170 sequential
# ones). Layout: repeated [u16 path len][path][u32 data len][data], little endian. The logo and
# font are left out and fetched separately: they are needed to draw the loading screen itself.
python3 - "$out" <<'PY'
import os, struct, sys
out = sys.argv[1]
skip = {"logo.png", "fonts/Roboto-Regular.ttf"}
blob = bytearray()
for root, _, files in sorted(os.walk("assets")):
    for name in sorted(files):
        full = os.path.join(root, name)
        rel = os.path.relpath(full, "assets").replace(os.sep, "/")
        if rel in skip:
            os.makedirs(os.path.dirname(os.path.join(out, "assets", rel)), exist_ok=True)
            with open(full, "rb") as f, open(os.path.join(out, "assets", rel), "wb") as g:
                g.write(f.read())
            continue
        data = open(full, "rb").read()
        p = rel.encode()
        blob += struct.pack("<H", len(p)) + p + struct.pack("<I", len(data)) + data
open(os.path.join(out, "assets.bin"), "wb").write(blob)
html = open("web/index.html").read().replace("__ASSET_BYTES__", str(len(blob)))
open(os.path.join(out, "index.html"), "w").write(html)
print(f"assets.bin: {len(blob)/1048576:.1f} MB")
PY
touch "$out/.nojekyll"
echo "built $out  (try: python3 -m http.server -d $out)"
