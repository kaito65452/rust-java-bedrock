#!/usr/bin/env bash
set -euo pipefail

# Packages a compiled release binary of `pack-studio-tauri` into a portable `.tar.xz`
# archive for Linux (`x86_64` or `arm64`) with safe WebKitGTK wrapper and licenses.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${TAR_ARCH:-$(uname -m)}"
VERSION="${RELEASE_VERSION:-0.2.2}"
BINARY_PATH="${1:-${REPO_ROOT}/target/release/pack-studio-tauri}"
OUT_DIR="${2:-${REPO_ROOT}/dist-tar}"

if [[ ! -f "${BINARY_PATH}" ]]; then
  echo "ERROR: Release binary not found at '${BINARY_PATH}'" >&2
  exit 1
fi

DIST_INDEX="${REPO_ROOT}/apps/pack-studio/dist/index.html"
TAURI_CONF="${REPO_ROOT}/apps/pack-studio/src-tauri/tauri.conf.json"
if [[ ! -f "${DIST_INDEX}" ]] || grep -q "PackStudio frontend placeholder" "${DIST_INDEX}"; then
  echo "ERROR: Valid Vite build missing at '${DIST_INDEX}'. Run 'npm run build' in apps/pack-studio first." >&2
  exit 1
fi
if ! grep -q '"frontendDist":[[:space:]]*"../dist"' "${TAURI_CONF}"; then
  echo "ERROR: '${TAURI_CONF}' must set frontendDist to '../dist'." >&2
  exit 1
fi

BUNDLE_NAME="pack-studio-${VERSION}-linux-${ARCH}"
STAGE_DIR="${REPO_ROOT}/.cache/tar-stage/${BUNDLE_NAME}"
OUT_TAR="${OUT_DIR}/${BUNDLE_NAME}.tar.xz"
OUT_SHA="${OUT_TAR}.sha256"

rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}" "${OUT_DIR}"

cp "${BINARY_PATH}" "${STAGE_DIR}/pack-studio-bin"
chmod 0755 "${STAGE_DIR}/pack-studio-bin"
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "${STAGE_DIR}/pack-studio-bin" || true
fi

cat > "${STAGE_DIR}/pack-studio" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export NO_AT_BRIDGE="${NO_AT_BRIDGE:-1}"
if [[ -z "${GDK_BACKEND:-}" ]]; then
  if [[ -n "${DISPLAY:-}" ]]; then
    export GDK_BACKEND="x11"
  elif [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
    export GDK_BACKEND="wayland"
  fi
fi
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"
export WEBKIT_DISABLE_COMPOSITING_MODE="${WEBKIT_DISABLE_COMPOSITING_MODE:-1}"
export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS="${WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS:-1}"

if [[ -z "${DISPLAY:-}" ]] && [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  echo "ERROR: No active X11 (DISPLAY) or Wayland (WAYLAND_DISPLAY) session detected." >&2
  echo "Start Termux:X11 / VNC / desktop session and export DISPLAY (e.g. 'export DISPLAY=:0') before running PackStudio." >&2
  exit 1
fi

if [[ ! -e /dev/dri/renderD128 ]] && [[ ! -e /dev/dri/card0 ]]; then
  export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
  export GALLIUM_DRIVER="${GALLIUM_DRIVER:-llvmpipe}"
fi

exec "${HERE}/pack-studio-bin" "$@"
EOF
chmod 0755 "${STAGE_DIR}/pack-studio"

if [[ -f "${REPO_ROOT}/THIRD_PARTY_LICENSES.md" ]]; then
  cp "${REPO_ROOT}/THIRD_PARTY_LICENSES.md" "${STAGE_DIR}/THIRD_PARTY_LICENSES.md"
fi
if [[ -f "${REPO_ROOT}/assets/vendor/blockbench-web.sha256" ]]; then
  cp "${REPO_ROOT}/assets/vendor/blockbench-web.sha256" "${STAGE_DIR}/blockbench-web.sha256"
fi

cat > "${STAGE_DIR}/README-LINUX.txt" <<EOF
PackStudio ${VERSION} (${ARCH}) — Portable Linux Tarball
======================================================

Requirements:
- GTK3 (libgtk-3-0 / libgtk-3-0t64) and WebKitGTK 4.1 (libwebkit2gtk-4.1-0)
- Active X11 (DISPLAY) or Wayland (WAYLAND_DISPLAY) session
- Optional: yazi (recommended TUI file picker/browser)

Run:
  ./pack-studio
EOF

tar -C "$(dirname "${STAGE_DIR}")" -cJf "${OUT_TAR}" "${BUNDLE_NAME}"
(cd "${OUT_DIR}" && sha256sum "$(basename "${OUT_TAR}")" > "$(basename "${OUT_SHA}")")

echo "Built portable tar.xz archive:"
echo "  TAR: ${OUT_TAR}"
echo "  SHA: $(cat "${OUT_SHA}")"
