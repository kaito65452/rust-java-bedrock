import { execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(
  fs.readFileSync(path.resolve(__dirname, "package.json"), "utf-8"),
) as { version?: string };

function resolveShortCommitSha(): string {
  const fromEnv =
    process.env.VITE_APP_COMMIT_SHA ||
    process.env.PACKSTUDIO_COMMIT_SHA ||
    (process.env.GITHUB_SHA ? process.env.GITHUB_SHA.slice(0, 7) : "");
  if (fromEnv.trim()) return fromEnv.trim();
  try {
    return execSync("git rev-parse --short=7 HEAD", {
      cwd: __dirname,
      stdio: ["ignore", "pipe", "ignore"],
    })
      .toString()
      .trim();
  } catch {
    return "dev";
  }
}

const host = process.env.TAURI_DEV_HOST;
const appVersion = pkg.version || "0.3.1-beta";
const commitSha = resolveShortCommitSha();

export default defineConfig(async () => ({
  plugins: [react()],
  define: {
    "import.meta.env.VITE_APP_VERSION": JSON.stringify(appVersion),
    "import.meta.env.VITE_APP_COMMIT_SHA": JSON.stringify(commitSha),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    chunkSizeWarningLimit: 5000,
  },
}));
