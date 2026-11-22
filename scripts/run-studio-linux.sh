#!/usr/bin/env bash
set -euo pipefail

# Linux / X11 / Wayland / Termux-proot launcher for PackStudio (Tauri v2)
# Automatically applies WebKitGTK DMABUF & software OpenGL fallbacks when needed.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Essential WebKitGTK stability flags on Linux X11, NVIDIA, and proot/Termux-X11
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"
export WEBKIT_DISABLE_COMPOSITING_MODE="${WEBKIT_DISABLE_COMPOSITING_MODE:-0}"

# Fallback to Mesa software rendering when hardware DRI render node is unavailable (e.g. proot)
if [[ ! -e /dev/dri/renderD128 ]] && [[ ! -e /dev/dri/card0 ]]; then
  export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
  export GALLIUM_DRIVER="${GALLIUM_DRIVER:-llvmpipe}"
fi

# Prefer system-installed binary, then workspace release/debug target
if command -v pack-studio-tauri >/dev/null 2>&1; then
  exec pack-studio-tauri "$@"
elif [[ -x "${REPO_ROOT}/target/release/pack-studio-tauri" ]]; then
  exec "${REPO_ROOT}/target/release/pack-studio-tauri" "$@"
elif [[ -x "${REPO_ROOT}/target/debug/pack-studio-tauri" ]]; then
  exec "${REPO_ROOT}/target/debug/pack-studio-tauri" "$@"
else
  echo "ERROR: pack-studio-tauri binary not found." >&2
  echo "Build frontend + Tauri app first:" >&2
  echo "  cd apps/pack-studio && npm install && npm run build" >&2
  echo "  cargo build -p pack-studio-tauri --release" >&2
  exit 1
fi
