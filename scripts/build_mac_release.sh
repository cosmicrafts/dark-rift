#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────
# macOS Native Release Build (Apple Silicon / aarch64)
# Produces a fully self-contained executable in builds/
# Run AFTER deploy_cloud.sh so version.txt is already bumped.
# Note: Intended to be run on a macOS machine, or via cross-compilation.
# ──────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

# ── Config ──────────────────────────────────────────────────────
VPS_IP="74.208.246.177"
TARGET="aarch64-apple-darwin"

green() { echo -e "\e[32m$1\e[0m"; }
cyan()  { echo -e "\e[36m$1\e[0m"; }
red()   { echo -e "\e[31m$1\e[0m"; }

# ── 1. Read version ────────────────────────────────────────────
VERSION_FILE="${ROOT}/version.txt"
if [[ ! -f "${VERSION_FILE}" ]]; then
  red "❌ version.txt not found — run deploy_cloud.sh first to bump version."
  exit 1
fi
CLEAN_VERSION=$(cat "${VERSION_FILE}")
cyan "📦 Building macOS release v${CLEAN_VERSION} for ${TARGET}"

# ── 2. Patch server IP for production ──────────────────────────
cyan "==> Patching server IP to ${VPS_IP}..."
# Cross-platform sed (GNU/Linux vs BSD/macOS)
if sed --version 2>/dev/null | grep -q GNU; then
  sed -i "s/127.0.0.1:25565/${VPS_IP}:25565/g" crates/client/src/main.rs
else
  sed -i '' "s/127.0.0.1:25565/${VPS_IP}:25565/g" crates/client/src/main.rs
fi

# ── 3. Build static release (NO dynamic_linking) ──────────────
cyan "==> Compiling release binary..."
# Ensure target is installed
rustup target add "${TARGET}" || true

# NOTE: We deliberately do NOT pass --features dev_native here.
cargo build --release -p dark-rift-client --target "${TARGET}" 2>&1 \
  || { red "❌ Compilation failed"; git checkout crates/client/src/main.rs; exit 1; }

# ── 4. Restore source ────────────────────────────────────────
git checkout crates/client/src/main.rs
green "✅ Source restored to localhost."

# ── 5. Verify no dynamic bevy dependency ──────────────────────
BIN_PATH="target/${TARGET}/release/dark-rift-client"
if command -v otool >/dev/null 2>&1; then
  if otool -L "${BIN_PATH}" 2>/dev/null | grep -q 'libbevy_dylib'; then
    red "❌ Binary still links libbevy_dylib! Check Cargo.toml features."
    exit 1
  fi
elif command -v ldd >/dev/null 2>&1; then
  if ldd "${BIN_PATH}" 2>/dev/null | grep -q 'libbevy_dylib'; then
    red "❌ Binary still links libbevy_dylib! Check Cargo.toml features."
    exit 1
  fi
fi

# ── 6. Package into Archive (Binary + Assets) ─────────────────
cyan "==> Packaging binary and assets..."
mkdir -p builds
ARCHIVE_DIR="builds/dark_rift_mac_v${CLEAN_VERSION}"
rm -rf "${ARCHIVE_DIR}"
mkdir -p "${ARCHIVE_DIR}"

cp "${BIN_PATH}" "${ARCHIVE_DIR}/dark_rift"
cp -r assets "${ARCHIVE_DIR}/"

cd builds
tar -czf "dark_rift_mac_v${CLEAN_VERSION}.tar.gz" "dark_rift_mac_v${CLEAN_VERSION}"
rm -rf "dark_rift_mac_v${CLEAN_VERSION}"
cd ..

OUT="builds/dark_rift_mac_v${CLEAN_VERSION}.tar.gz"

# ── 7. Summary ────────────────────────────────────────────────
# Cross-platform du format
SIZE=$(du -h "${OUT}" | awk '{print $1}')
echo "========================================================="
green "🎉 macOS release build complete!"
echo "   Binary:  ${OUT}"
echo "   Size:    ${SIZE}"
echo "   Version: v${CLEAN_VERSION}"
echo "========================================================="
