#!/usr/bin/env bash
set -euo pipefail

# Packages a compiled release binary of `pack-studio` or `pack-studio-tauri`
# into a Debian (.deb) package for ARM64 (`arm64`) or AMD64 (`amd64`).

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${DEB_ARCH:-$(dpkg --print-architecture)}"
VERSION="${DEB_VERSION:-0.1.0}"
BINARY_PATH="${1:-${REPO_ROOT}/target/release/pack-studio-tauri}"
OUT_DIR="${2:-${REPO_ROOT}/dist-deb}"

if [[ ! -f "${BINARY_PATH}" ]]; then
  echo "ERROR: Release binary not found at '${BINARY_PATH}'" >&2
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
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"
if [[ ! -d /dev/dri ]] && [[ -z "${LIBGL_ALWAYS_SOFTWARE:-}" ]]; then
  export LIBGL_ALWAYS_SOFTWARE=1
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

# Generate a clean 128x128 PNG application icon (VS Code Dark+ blue & Minecraft grass accent)
python3 - "${STAGE_ROOT}/usr/share/icons/hicolor/128x128/apps/${PKG_NAME}.png" <<'PY'
import struct, zlib, sys

out_path = sys.argv[1]
w, h = 128, 128
raw = bytearray()
for y in range(h):
    raw.append(0)
    for x in range(w):
        if x < 6 or x >= w - 6 or y < 6 or y >= h - 6:
            r, g, b, a = 0x00, 0x7A, 0xCC, 255
        elif y < 44:
            r, g, b, a = 0x59, 0xB8, 0x4C, 255
        elif y < 52 and ((x // 8) % 2 == 0):
            r, g, b, a = 0x45, 0x99, 0x3A, 255
        else:
            shade = 10 if ((x // 16 + y // 16) % 2 == 0) else 0
            r, g, b, a = 0x7A - shade, 0x55 - shade, 0x38 - shade, 255
        raw.extend((r, g, b, a))

def chunk(kind, data):
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )

png = bytearray(b"\x89PNG\r\n\x1a\n")
png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
png += chunk(b"IEND", b"")

with open(out_path, "wb") as f:
    f.write(png)
PY

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
