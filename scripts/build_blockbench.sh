#!/usr/bin/env bash
set -euo pipefail

# Build Mandiri JannisX11/blockbench (Web Bundle), Strip Electron/Dev/Sourcemaps,
# Inject studio_bridge.js, and Compress to assets/vendor/blockbench-web.tar.zst.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE_DIR="${REPO_ROOT}/.cache/blockbench-src"
STAGE_DIR="${REPO_ROOT}/dist-web"
VENDOR_DIR="${REPO_ROOT}/assets/vendor"
BRIDGE_SRC="${REPO_ROOT}/scripts/studio_bridge.js"
OUT_ARCHIVE="${VENDOR_DIR}/blockbench-web.tar.zst"
OUT_SHA="${VENDOR_DIR}/blockbench-web.sha256"

BLOCKBENCH_REPO="https://github.com/JannisX11/blockbench.git"

# Allow overriding tag via BLOCKBENCH_TAG env var; otherwise resolve latest stable v* tag
if [[ -n "${BLOCKBENCH_TAG:-}" ]]; then
  TAG="${BLOCKBENCH_TAG}"
else
  echo "[1/6] Resolving latest stable release tag from ${BLOCKBENCH_REPO}..."
  TAG="$(git ls-remote --tags --refs "${BLOCKBENCH_REPO}" \
    | awk '{print $2}' \
    | sed 's#refs/tags/##' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | sort -V \
    | tail -n 1)"
fi

if [[ -z "${TAG}" ]]; then
  echo "ERROR: Could not resolve latest stable Blockbench release tag." >&2
  exit 1
fi

echo "[2/6] Shallow cloning Blockbench ${TAG} into ${CACHE_DIR}..."
mkdir -p "${REPO_ROOT}/.cache" "${VENDOR_DIR}"
if [[ -d "${CACHE_DIR}/.git" ]]; then
  CURRENT_TAG="$(git -C "${CACHE_DIR}" describe --tags --exact-match 2>/dev/null || true)"
  if [[ "${CURRENT_TAG}" != "${TAG}" ]]; then
    rm -rf "${CACHE_DIR}"
    git clone --depth 1 --branch "${TAG}" "${BLOCKBENCH_REPO}" "${CACHE_DIR}"
  else
    echo "      Reusing existing shallow clone of ${TAG}."
  fi
else
  rm -rf "${CACHE_DIR}"
  git clone --depth 1 --branch "${TAG}" "${BLOCKBENCH_REPO}" "${CACHE_DIR}"
fi

echo "[3/6] Installing JS dependencies & building production web bundle (no Electron binary)..."
pushd "${CACHE_DIR}" >/dev/null
export ELECTRON_SKIP_BINARY_DOWNLOAD=1

if [[ -f package-lock.json ]]; then
  npm ci --include=dev --no-audit --no-fund --ignore-scripts
else
  npm install --include=dev --no-audit --no-fund --ignore-scripts
fi

# Run production web bundle (esbuild minifies automatically when not in --watch/--serve mode)
NODE_ENV=production npm run build-web

SRC_SIZE_BYTES="$(du -sb "${CACHE_DIR}" | awk '{print $1}')"
SRC_SIZE_HUMAN="$(du -sh "${CACHE_DIR}" | awk '{print $1}')"
popd >/dev/null

echo "[4/6] Staging web-only runtime into ${STAGE_DIR} (stripping Electron, sourcemaps, dev files)..."
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}/dist"

cp "${CACHE_DIR}/index.html" "${STAGE_DIR}/index.html"
cp "${CACHE_DIR}/LICENSE.MD" "${STAGE_DIR}/LICENSE.MD"
if [[ -f "${CACHE_DIR}/favicon.png" ]]; then
  cp "${CACHE_DIR}/favicon.png" "${STAGE_DIR}/favicon.png"
fi

# Copy production JS/WASM outputs without sourcemaps
find "${CACHE_DIR}/dist" -maxdepth 2 -type f \( -name '*.js' -o -name '*.wasm' -o -name '*.css' \) ! -name '*.map' -exec cp {} "${STAGE_DIR}/dist/" \;

# Copy essential runtime directories (keeping Modeling 3D, Paint 2D/3D, UV, Animate 100% intact)
for dir in css assets font lang; do
  if [[ -d "${CACHE_DIR}/${dir}" ]]; then
    cp -r "${CACHE_DIR}/${dir}" "${STAGE_DIR}/${dir}"
  fi
done

if [[ -f "${CACHE_DIR}/lib/gif.worker.js" ]]; then
  mkdir -p "${STAGE_DIR}/lib"
  cp "${CACHE_DIR}/lib/gif.worker.js" "${STAGE_DIR}/lib/gif.worker.js"
fi

# Strip any stray sourcemaps, Electron artifacts, desktop installers, or test/doc files
find "${STAGE_DIR}" -type f \( \
  -name '*.map' -o \
  -name '*.ico' -o \
  -name '*.icns' -o \
  -name '*.dmg' -o \
  -name '*.exe' -o \
  -name '*.AppImage' -o \
  -name 'main.js' \
\) -delete
rm -rf "${STAGE_DIR}/node_modules" "${STAGE_DIR}/electron" "${STAGE_DIR}/test" "${STAGE_DIR}/build"

echo "[5/6] Injecting PackStudio 2-way bridge (${BRIDGE_SRC}) into index.html..."
cp "${BRIDGE_SRC}" "${STAGE_DIR}/studio_bridge.js"

python3 - "${STAGE_DIR}/index.html" <<'PY'
import sys, pathlib
p = pathlib.Path(sys.argv[1])
html = p.read_text(encoding="utf-8")
tag = '<script src="./studio_bridge.js"></script>'
if tag not in html:
    if "</body>" in html:
        html = html.replace("</body>", f"\t{tag}\n</body>", 1)
    else:
        html += f"\n{tag}\n"
p.write_text(html, encoding="utf-8")
PY

STAGE_SIZE_BYTES="$(du -sb "${STAGE_DIR}" | awk '{print $1}')"
STAGE_SIZE_HUMAN="$(du -sh "${STAGE_DIR}" | awk '{print $1}')"

echo "[6/6] Compressing ${STAGE_DIR} -> ${OUT_ARCHIVE} (zstd -19)..."
tar -C "${STAGE_DIR}" -cf - . | zstd -19 -T0 -f -o "${OUT_ARCHIVE}"

ARCHIVE_SIZE_BYTES="$(stat -c '%s' "${OUT_ARCHIVE}")"
ARCHIVE_SIZE_HUMAN="$(du -sh "${OUT_ARCHIVE}" | awk '{print $1}')"
SHA256_HEX="$(sha256sum "${OUT_ARCHIVE}" | awk '{print $1}')"

cat > "${OUT_SHA}" <<EOF
tag=${TAG}
sha256=${SHA256_HEX}
archive=blockbench-web.tar.zst
license=GPL-3.0
EOF

echo ""
echo "============================================================"
echo " Blockbench Web Bundle Build Complete (${TAG})"
echo "------------------------------------------------------------"
echo " Source + node_modules : ${SRC_SIZE_HUMAN} (${SRC_SIZE_BYTES} bytes)"
echo " Staged dist-web/      : ${STAGE_SIZE_HUMAN} (${STAGE_SIZE_BYTES} bytes)"
echo " Final tar.zst (-19)   : ${ARCHIVE_SIZE_HUMAN} (${ARCHIVE_SIZE_BYTES} bytes)"
echo " SHA-256               : ${SHA256_HEX}"
echo " Output Archive        : ${OUT_ARCHIVE}"
echo " Checksum File         : ${OUT_SHA}"
echo "============================================================"
