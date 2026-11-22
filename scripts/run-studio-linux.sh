#!/usr/bin/env bash
set -euo pipefail

# Linux / X11 / Wayland / Termux-proot launcher for PackStudio (Tauri v2)
# Automatically applies safe WebKitGTK & software OpenGL defaults (user env overrides are preserved).

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

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

# Fallback to Mesa software rendering when hardware DRI render node is unavailable (e.g. proot)
if [[ ! -e /dev/dri/renderD128 ]] && [[ ! -e /dev/dri/card0 ]]; then
  export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"
  export GALLIUM_DRIVER="${GALLIUM_DRIVER:-llvmpipe}"
fi

# Prefer installed binary, then workspace release/debug target
if command -v pack-studio-bin >/dev/null 2>&1; then
  exec pack-studio-bin "$@"
elif command -v pack-studio-tauri >/dev/null 2>&1; then
  exec pack-studio-tauri "$@"
elif [[ -x "${REPO_ROOT}/target/release/pack-studio-tauri" ]]; then
  exec "${REPO_ROOT}/target/release/pack-studio-tauri" "$@"
elif [[ -x "${REPO_ROOT}/target/debug/pack-studio-tauri" ]]; then
  exec "${REPO_ROOT}/target/debug/pack-studio-tauri" "$@"
else
  echo "ERROR: pack-studio-tauri binary not found." >&2
  echo "Build frontend + Tauri release app first:" >&2
  echo "  cd apps/pack-studio && npm install && npm run build" >&2
  echo '  cargo build --release -p pack-studio-tauri --features "embed-blockbench,custom-protocol"' >&2
  exit 1
fi
