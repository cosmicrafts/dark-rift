#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────
# Windows Native Release Build (x86_64 GNU)
# Produces a self-contained archive (ZIP or tar.gz) in builds/
# containing the .exe and the assets directory.
# ──────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

# ── Config ──────────────────────────────────────────────────────
VPS_IP="74.208.246.177"
TARGET="x86_64-pc-windows-gnu"

green() { echo -e "\e[32m$1\e[0m"; }
cyan()  { echo -e "\e[36m$1\e[0m"; }
red()   { echo -e "\e[31m$1\e[0m"; }

# ── Check toolchain ────────────────────────────────────────────
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  red "❌ Missing MinGW-w64 toolchain for Windows cross-compilation."
  cyan "👉 Install it on Arch Linux using: sudo pacman -S mingw-w64-gcc"
  exit 1
fi

# ── 1. Read version ────────────────────────────────────────────
VERSION_FILE="${ROOT}/version.txt"
if [[ ! -f "${VERSION_FILE}" ]]; then
  red "❌ version.txt not found — run deploy_cloud.sh first to bump version."
  exit 1
fi
CLEAN_VERSION=$(cat "${VERSION_FILE}")
cyan "📦 Building Windows release v${CLEAN_VERSION} for ${TARGET}"

# ── 2. Patch server IP for production ──────────────────────────
cyan "==> Patching server IP to ${VPS_IP}..."
cp crates/client/src/main.rs crates/client/src/main.rs.bak
if sed --version 2>/dev/null | grep -q GNU; then
  sed -i "s/127.0.0.1:25565/${VPS_IP}:25565/g" crates/client/src/main.rs
else
  sed -i '' "s/127.0.0.1:25565/${VPS_IP}:25565/g" crates/client/src/main.rs
fi

# ── 3. Build static release (NO dynamic_linking) ──────────────
cyan "==> Compiling Windows executable..."
rustup target add "${TARGET}" || true

cargo build --release -p dark-rift-client --target "${TARGET}" 2>&1 \
  || { red "❌ Compilation failed"; mv crates/client/src/main.rs.bak crates/client/src/main.rs; exit 1; }

# ── 4. Restore source ────────────────────────────────────────
mv crates/client/src/main.rs.bak crates/client/src/main.rs
green "✅ Source restored to localhost."

# ── 5. Package into Archive (Binary + Assets) ─────────────────
cyan "==> Packaging binary and assets..."
BIN_PATH="target/${TARGET}/release/dark-rift-client.exe"

mkdir -p builds
ARCHIVE_DIR="builds/dark_rift_windows_v${CLEAN_VERSION}"
rm -rf "${ARCHIVE_DIR}"
mkdir -p "${ARCHIVE_DIR}"

cp "${BIN_PATH}" "${ARCHIVE_DIR}/dark_rift.exe"
cp -r assets "${ARCHIVE_DIR}/"

cd builds
if command -v zip >/dev/null 2>&1; then
  zip -r "dark_rift_windows_v${CLEAN_VERSION}.zip" "dark_rift_windows_v${CLEAN_VERSION}" > /dev/null
  OUT="builds/dark_rift_windows_v${CLEAN_VERSION}.zip"
else
  tar -czf "dark_rift_windows_v${CLEAN_VERSION}.tar.gz" "dark_rift_windows_v${CLEAN_VERSION}"
  OUT="builds/dark_rift_windows_v${CLEAN_VERSION}.tar.gz"
fi
rm -rf "dark_rift_windows_v${CLEAN_VERSION}"
cd ..

# ── 6. Summary ────────────────────────────────────────────────
SIZE=$(du -h "${OUT}" | awk '{print $1}')
echo "========================================================="
green "🎉 Windows release build complete!"
echo "   Archive: ${OUT}"
echo "   Size:    ${SIZE}"
echo "   Version: v${CLEAN_VERSION}"
echo "========================================================="
