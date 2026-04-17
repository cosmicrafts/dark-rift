#!/usr/bin/env bash
# Producción WASM bundle: release build → wasm-bindgen → compresión Brotli → Sincronización al VPS
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${ROOT}"

export CARGO_TARGET_DIR="${ROOT}/target"
WASM_IN="${CARGO_TARGET_DIR}/wasm32-unknown-unknown/release/dark-rift-client.wasm"
OUT_NAME="dark_rift_client"

VPS_IP="74.208.246.177"
VPS_USER="bizkit"
WEB_DEST_DIR="/var/www/darkrift.ai/html"

echo "==> 1. Compilando dark-rift-client (WASM)..."
cargo build --release -p dark-rift-client --target wasm32-unknown-unknown

echo "==> 2. wasm-bindgen → dist/"
rm -rf dist
mkdir -p dist
/home/bizkit/.cargo/bin/wasm-bindgen --out-dir dist --target web --out-name "${OUT_NAME}" --no-typescript "${WASM_IN}"

echo "==> 3. Copiando assets..."
cp -a assets dist/assets

echo "==> 3b. Comprobando archivos que el cliente WASM pide por HTTP..."
for f in \
  dist/assets/white_array.png \
  dist/assets/white_array.png.meta \
  dist/assets/white.png \
  dist/assets/white.png.meta \
  dist/assets/settings.ron \
  dist/assets/fonts/Rajdhani-Medium.ttf \
  ; do
  if [[ ! -f "$f" ]]; then
    echo "ERROR: falta '$f' tras cp -a assets. ¿Estás en la raíz del repo dark-rift/ y existe assets/?" >&2
    exit 1
  fi
done

echo "==> 4. Creando index.html (Producción - WSS)..."
cat > dist/index.html <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Dark Rift</title>
  <style>
    html, body { margin: 0; height: 100%; background: #050508; }
    canvas { display: block; }
  </style>
</head>
<body>
  <script type="module">
    import init from "./dark_rift_client.js";
    init().catch((error) => {
      const msg = error && error.message ? error.message : String(error);
      if (msg.startsWith("Using exceptions for control flow")) return;
      throw error;
    });
  </script>
</body>
</html>
HTML

echo "==> 5. Comprimiendo con Brotli (Max level)..."
if command -v brotli >/dev/null 2>&1; then
  brotli -f -Z dist/${OUT_NAME}_bg.wasm
  brotli -f -Z dist/${OUT_NAME}.js
  echo "Compresión lista."
else
  echo "Advertencia: 'brotli' no está instalado. Omitiendo compresión."
fi

echo "==> 6. Subiendo archivos web al VPS (Nginx)..."
rsync -avzP dist/ ${VPS_USER}@${VPS_IP}:${WEB_DEST_DIR}/

echo "==> ¡Despliegue Web completado! Entra a https://darkrift.ai para probar."
