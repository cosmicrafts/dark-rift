#!/usr/bin/env bash
# Local loader-only preview: same index.html.template + stub init (no Rust/WASM build).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

TEMPLATE="${ROOT}/web/index.html.template"
OUT="${ROOT}/web/.loader-preview"
PORT="${PORT:-8765}"

if [[ ! -f "${TEMPLATE}" ]]; then
  echo "Missing ${TEMPLATE}" >&2
  exit 1
fi

rm -rf "${OUT}"
mkdir -p "${OUT}/assets/ui"
cp "${ROOT}/assets/ui/darkrift.webp" "${OUT}/assets/ui/"

BUILD_TS="$(date +%s)"
# Stub replaces wasm-bindgen glue; delay mimics slow init so you can tune the bar.
STUB_DELAY_MS="${STUB_DELAY_MS:-4500}"

cat > "${OUT}/dark_rift_client.js" <<STUB
export default async function init(_wasmPath) {
  await new Promise((r) => setTimeout(r, ${STUB_DELAY_MS}));
}
STUB

sed "s/__BUILD_TS__/${BUILD_TS}/g" "${TEMPLATE}" \
  | sed '/rel="preload" href="\.\/dark_rift_client_bg\.wasm/d' \
  > "${OUT}/index.html"

echo "Loader preview (stub init ${STUB_DELAY_MS} ms; STUB_DELAY_MS=8000 $0)"
echo "  http://127.0.0.1:${PORT}/"
echo "  Edita web/index.html.template y vuelve a ejecutar este script (no hay hot-reload)."
echo "  (Ctrl+C to stop)"
exec python3 -m http.server "${PORT}" --bind 127.0.0.1 --directory "${OUT}"
