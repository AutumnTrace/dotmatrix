#!/usr/bin/env bash
# Builds the wasm module and copies it next to the static site in web/.
# Serve the folder with any static file server, e.g.
#   python3 -m http.server -d web 8080
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"
cargo build -p dotmatrix-web --target wasm32-unknown-unknown --release
cp "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/dotmatrix_web.wasm" web/dotmatrix.wasm
echo "web/dotmatrix.wasm: $(wc -c < web/dotmatrix.wasm) bytes"
