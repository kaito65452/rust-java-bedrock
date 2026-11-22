#!/usr/bin/env bash
set -euo pipefail

# Packages a compiled release binary of `pack-studio-tauri` (or legacy `pack-studio`)
# into a Debian (.deb) package for ARM64 (`arm64`) or AMD64 (`amd64`).

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${DEB_ARCH:-$(dpkg --print-architecture)}"
VERSION="${DEB_VERSION:-0.2.1}"
BINARY_PATH="${1:-${REPO_ROOT}/target/release/pack-studio-tauri}"
OUT_DIR="${2:-${REPO_ROOT}/dist-deb}"

if [[ ! -f "${BINARY_PATH}" ]]; then
  echo "ERROR: Release binary not found at '${BINARY_PATH}'" >&2
  echo "Hint: build frontend + release binary first:" >&2
  echo "  (cd apps/pack-studio && npm install && npm run build)" >&2
  echo '  cargo build --release -p pack-studio-tauri --features "embed-blockbench,custom-protocol"' >&2
  exit 1
fi

PKG_NAME="pack-studio"
STAGE_ROOT="${REPO_ROOT}/.cache/deb-stage/${PKG_NAME}_${VERSION}_${ARCH}"
OUT_DEB="${OUT_DIR}/${PKG_NAME}_${VERSION}_${ARCH}.deb"
OUT_SHA="${OUT_DEB}.sha256"
IS_TAURI=0
if [[ "$(basename "${BINARY_PATH}")" == *"tauri"* ]] || (command -v ldd >/dev/null 2>&1 && ldd "${BINARY_PATH}" 2>/dev/null | grep -q "webkit2gtk"); then
  IS_TAURI=1
fi

if [[ "${IS_TAURI}" -eq 1 ]]; then
  DIST_INDEX="${REPO_ROOT}/apps/pack-studio/dist/index.html"
  TAURI_CONF="${REPO_ROOT}/apps/pack-studio/src-tauri/tauri.conf.json"
  if [[ ! -f "${DIST_INDEX}" ]]; then
    echo "ERROR: Built frontend missing at '${DIST_INDEX}'. Run 'npm run build' in apps/pack-studio before cargo build --release." >&2
    exit 1
  fi
  if grep -q "PackStudio frontend placeholder" "${DIST_INDEX}"; then
    echo "ERROR: '${DIST_INDEX}' is still the placeholder HTML! Run 'npm run build' in apps/pack-studio and rebuild with '--features \"embed-blockbench,custom-protocol\"'." >&2
    exit 1
  fi
  if ! grep -q '"frontendDist":[[:space:]]*"../dist"' "${TAURI_CONF}"; then
    echo "ERROR: '${TAURI_CONF}' must set frontendDist to '../dist'." >&2
    exit 1
  fi
fi

rm -rf "${STAGE_ROOT}"
mkdir -p \
  "${OUT_DIR}" \
  "${STAGE_ROOT}/DEBIAN" \
  "${STAGE_ROOT}/usr/bin" \
  "${STAGE_ROOT}/usr/share/applications" \
  "${STAGE_ROOT}/usr/share/icons/hicolor/128x128/apps" \
  "${STAGE_ROOT}/usr/share/doc/${PKG_NAME}" \
  "${STAGE_ROOT}/usr/share/${PKG_NAME}"

if [[ "${IS_TAURI}" -eq 1 ]]; then
  cp "${BINARY_PATH}" "${STAGE_ROOT}/usr/bin/${PKG_NAME}-bin"
  chmod 0755 "${STAGE_ROOT}/usr/bin/${PKG_NAME}-bin"
  if command -v strip >/dev/null 2>&1; then
    strip --strip-unneeded "${STAGE_ROOT}/usr/bin/${PKG_NAME}-bin" || true
  fi

  cat > "${STAGE_ROOT}/usr/bin/${PKG_NAME}" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

# Safe WebKitGTK defaults for Linux X11, Wayland, containers, and Termux proot
# (existing environment variables are preserved if set by the user).
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

exec /usr/bin/pack-studio-bin "$@"
EOF
  chmod 0755 "${STAGE_ROOT}/usr/bin/${PKG_NAME}"
  DEB_DEPENDS="libc6, libgtk-3-0t64 | libgtk-3-0, libwebkit2gtk-4.1-0 | libwebkit2gtk-4.0-37, libgl1 | libglx-mesa0"
else
  cp "${BINARY_PATH}" "${STAGE_ROOT}/usr/bin/${PKG_NAME}"
  chmod 0755 "${STAGE_ROOT}/usr/bin/${PKG_NAME}"
  if command -v strip >/dev/null 2>&1; then
    strip --strip-unneeded "${STAGE_ROOT}/usr/bin/${PKG_NAME}" || true
  fi
  DEB_DEPENDS="libc6, libgl1 | libglx-mesa0, libx11-6, libxkbcommon0, libwayland-client0"
fi

# Include Blockbench metadata & GPL-3.0 third-party licenses
if [[ -f "${REPO_ROOT}/assets/vendor/blockbench-web.sha256" ]]; then
  cp "${REPO_ROOT}/assets/vendor/blockbench-web.sha256" "${STAGE_ROOT}/usr/share/${PKG_NAME}/blockbench-web.sha256"
fi
if [[ -f "${REPO_ROOT}/THIRD_PARTY_LICENSES.md" ]]; then
  cp "${REPO_ROOT}/THIRD_PARTY_LICENSES.md" "${STAGE_ROOT}/usr/share/doc/${PKG_NAME}/THIRD_PARTY_LICENSES.md"
fi

cat > "${STAGE_ROOT}/usr/share/doc/${PKG_NAME}/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: pack-studio
Source: https://github.com/kaito65452/rust-java-bedrock

Files: *
Copyright: 2025-2026 rust-java-bedrock contributors
License: MIT

Files: assets/vendor/blockbench-web.tar.zst
Copyright: Jannis Tobias Petersen (JannisX11) and Blockbench contributors
License: GPL-3.0
 Complete upstream GPL-3.0 license text (LICENSE.MD) is embedded inside the
 Blockbench web runtime bundle and extracted into the user cache directory.
EOF

# Install application icon from Tauri app icons (single source of truth)
cp "${REPO_ROOT}/apps/pack-studio/src-tauri/icons/icon.png" "${STAGE_ROOT}/usr/share/icons/hicolor/128x128/apps/${PKG_NAME}.png"

cat > "${STAGE_ROOT}/usr/share/applications/${PKG_NAME}.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=PackStudio
GenericName=Minecraft Resource Pack Studio
Comment=VS Code-style Resource Pack Artist Workbench (Java + Bedrock) with Blockbench Bridge
Exec=pack-studio %F
Icon=pack-studio
Terminal=false
Categories=Graphics;3DGraphics;Development;
MimeType=application/zip;
StartupNotify=true
EOF

INSTALLED_KB="$(du -sk "${STAGE_ROOT}" | awk '{print $1}')"

cat > "${STAGE_ROOT}/DEBIAN/control" <<EOF
Package: ${PKG_NAME}
Version: ${VERSION}
Section: graphics
Priority: optional
Architecture: ${ARCH}
Installed-Size: ${INSTALLED_KB}
Maintainer: PackStudio Maintainers <noreply@github.com>
Depends: ${DEB_DEPENDS}
Recommends: xdg-utils, yazi
Description: VS Code-style Minecraft Resource Pack Studio (Java + Bedrock)
 PackStudio is a fast Tauri v2 + Rust desktop workbench for Minecraft Java and
 Bedrock resource pack artists. Includes Monaco editor, pixel-accurate 2D paint
 canvas, nearest-neighbor downscaling, New Project Wizard, .zip/.mcpack/.mcaddon
 packaging, Semantic Audit Gate, PTY Terminal / Yazi integration, and an
 embedded offline Blockbench Web Studio 2-way bridge.
EOF

dpkg-deb --build --root-owner-group "${STAGE_ROOT}" "${OUT_DEB}"
sha256sum "${OUT_DEB}" > "${OUT_SHA}"

echo ""
echo "Built Debian package cleanly:"
echo "  DEB: ${OUT_DEB}"
echo "  SHA: $(cat "${OUT_SHA}")"
