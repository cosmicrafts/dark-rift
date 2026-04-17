#!/usr/bin/env bash
# Local WASM bundle: release build → wasm-bindgen → dist/ + assets + index.html
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${ROOT}"

# Keep artifacts under this workspace (avoids accidental CARGO_TARGET_DIR overrides).
export CARGO_TARGET_DIR="${ROOT}/target"

WASM_IN="${CARGO_TARGET_DIR}/wasm32-unknown-unknown/release/dark-rift-client.wasm"
OUT_NAME="dark_rift_client"

command -v wasm-bindgen >/dev/null 2>&1 || {
  echo "wasm-bindgen not found. Install with: cargo install wasm-bindgen-cli" >&2
  exit 1
}

rm -rf dist
mkdir -p dist

echo "==> cargo build (wasm32 release, -p dark-rift-client)"
cargo build --release -p dark-rift-client --target wasm32-unknown-unknown

test -f "${WASM_IN}" || {
  echo "Expected wasm not found: ${WASM_IN}" >&2
  exit 1
}

echo "==> wasm-bindgen → dist/"
wasm-bindgen \
  --out-dir dist \
  --target web \
  --out-name "${OUT_NAME}" \
  --no-typescript \
  "${WASM_IN}"

echo "==> assets → dist/assets"
cp -a assets dist/assets

for f in \
  dist/assets/white_array.png \
  dist/assets/white_array.png.meta \
  dist/assets/white.png \
  dist/assets/white.png.meta \
  dist/assets/settings.ron \
  dist/assets/fonts/Rajdhani-Medium.ttf \
  ; do
  if [[ ! -f "$f" ]]; then
    echo "ERROR: falta '$f' (revisa la carpeta assets/ en la raíz dark-rift/)." >&2
    exit 1
  fi
done

cat > dist/index.html <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Dark Rift</title>
  <style>
    html, body { margin: 0; height: 100%; background: #050508; }
  </style>
</head>
<body>
  <script type="module">
    import init from "./dark_rift_client.js";
    init().catch((error) => {
      const msg = error && error.message ? error.message : String(error);
      if (msg.startsWith("Using exceptions for control flow, don't mind me. This isn't actually an error!")) {
        return;
      }
      throw error;
    });
  </script>
</body>
</html>
HTML

echo "==> Done. Serve with: python3 -m http.server 8000 -d dist/"
echo "    Open http://localhost:8000/"
