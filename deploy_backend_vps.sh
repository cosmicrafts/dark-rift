#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${ROOT}"

VPS_IP="74.208.246.177"
VPS_USER="bizkit" # Ajustar si usas otro usuario
DEST_DIR="/home/bizkit/darkrift"

echo "==> Compilando dark-rift-server (x86_64-linux-musl)..."
cargo build --release -p dark-rift-server --target x86_64-unknown-linux-musl

echo "==> Asegurando directorio remoto en el VPS..."
ssh ${VPS_USER}@${VPS_IP} "mkdir -p ${DEST_DIR}"

echo "==> Sincronizando binario al VPS..."
rsync -avzP target/x86_64-unknown-linux-musl/release/dark-rift-server ${VPS_USER}@${VPS_IP}:${DEST_DIR}/

echo "==> ¡Listo! El backend está en el VPS."
echo "Para probarlo, entra por SSH y corre: ${DEST_DIR}/dark-rift-server"
