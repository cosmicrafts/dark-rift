#!/usr/bin/env bash
# Despliegue Unificado a Producción (Cliente Web + Backend)
# Orquesta compilaciones en paralelo y comprime sin bloquear el hilo principal.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${ROOT}"

VPS_IP="74.208.246.177"
VPS_USER="bizkit"
WEB_DEST_DIR="/var/www/darkrift.ai/html"
BACKEND_DEST_DIR="/home/bizkit/darkrift"

export CARGO_TARGET_DIR="${ROOT}/target"
WASM_IN="${CARGO_TARGET_DIR}/wasm32-unknown-unknown/release/dark-rift-client.wasm"
OUT_NAME="dark_rift_client"

# Funciones de color para leer mejor los logs
green() { echo -e "\e[32m$1\e[0m"; }
cyan() { echo -e "\e[36m$1\e[0m"; }
red() { echo -e "\e[31m$1\e[0m"; }

echo "========================================================="
cyan "🚀 Iniciando Despliegue a Producción (Dark Rift -> VPS)"
echo "========================================================="

# 1. Compilaciones en Paralelo
cyan "==> 1. [PARALELO] Compilando Backend y Frontend..."

# Lanzar compilar Cliente en background
cargo build --release -p dark-rift-client --target wasm32-unknown-unknown &
CLIENT_PID=$!

# Lanzar compilar Servidor en background
cargo build --release -p dark-rift-server --target x86_64-unknown-linux-musl &
SERVER_PID=$!

# Esperar a que terminen ambos
wait $CLIENT_PID || { red "❌ Error compilando Cliente WASM"; exit 1; }
wait $SERVER_PID || { red "❌ Error compilando Servidor Musl"; exit 1; }
green "✅ Compilaciones de Rust exitosas."

# 2. Empaquetado Web (Wasm-bindgen y Brotli)
cyan "==> 2. Empaquetando Frontend (wasm-bindgen)..."
rm -rf dist
mkdir -p dist
wasm-bindgen --out-dir dist --target web --out-name "${OUT_NAME}" --no-typescript "${WASM_IN}"

cp -a assets dist/assets

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

cyan "==> 3. Comprimiendo archivos estáticos (Brotli)..."
# Comprimir en paralelo
if command -v brotli >/dev/null 2>&1; then
  brotli -f -Z dist/${OUT_NAME}_bg.wasm &
  BROTLI_WASM_PID=$!
  brotli -f -Z dist/${OUT_NAME}.js &
  BROTLI_JS_PID=$!
  
  wait $BROTLI_WASM_PID
  wait $BROTLI_JS_PID
  green "✅ Compresión Brotli finalizada."
else
  red "⚠️ Advertencia: 'brotli' no instalado. Archivos pesarán más."
fi

# 3. Sincronización al VPS en Paralelo
cyan "==> 4. [PARALELO] Subiendo archivos al VPS..."
# Subir Web
rsync -avz --delete dist/ ${VPS_USER}@${VPS_IP}:${WEB_DEST_DIR}/ &
RSYNC_WEB_PID=$!

# Subir Server
ssh ${VPS_USER}@${VPS_IP} "mkdir -p ${BACKEND_DEST_DIR}"
rsync -avz target/x86_64-unknown-linux-musl/release/dark-rift-server ${VPS_USER}@${VPS_IP}:${BACKEND_DEST_DIR}/ &
RSYNC_SERVER_PID=$!

wait $RSYNC_WEB_PID || { red "❌ Error subiendo Frontend"; exit 1; }
wait $RSYNC_SERVER_PID || { red "❌ Error subiendo Backend"; exit 1; }
green "✅ Archivos sincronizados en el VPS."

# 4. Reinicio de Servicios
cyan "==> 5. Reiniciando servicio de Systemd en el VPS..."
ssh ${VPS_USER}@${VPS_IP} "sudo systemctl restart darkrift-server" || { red "❌ Error reiniciando el servicio"; exit 1; }

echo "========================================================="
green "🎉 ¡Despliegue Maestro Completado Exitosamente!"
echo "🕹️  Juega en vivo: https://darkrift.ai"
echo "========================================================="
