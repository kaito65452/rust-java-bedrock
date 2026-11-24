import React, { useEffect, useMemo, useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import type { ProjectSnapshot } from "../types";
import { baseName } from "../utils/mcFormat";

interface ImportAssetsModalProps {
  project: ProjectSnapshot;
  initialTargetDir: string;
  autoSanitize: boolean;
  convertImagesToPng: boolean;
  onToggleAutoSanitize: (value: boolean) => void;
  onToggleConvertImagesToPng: (value: boolean) => void;
  onClose: () => void;
  onImport: (targetDir: string, diskPaths: string[]) => Promise<void>;
}

const JAVA_DIR_PRESETS = [
  "assets/minecraft/textures/block",
  "assets/minecraft/textures/item",
  "assets/minecraft/textures/entity",
  "assets/minecraft/models/block",
  "assets/minecraft/models/item",
  "assets/minecraft/sounds",
];

const BEDROCK_DIR_PRESETS = [
  "textures/blocks",
  "textures/items",
  "textures/entity",
  "models/entity",
  "sounds",
  "font",
];

export const ImportAssetsModal: React.FC<ImportAssetsModalProps> = ({
  project,
  initialTargetDir,
  autoSanitize,
  convertImagesToPng,
  onToggleAutoSanitize,
  onToggleConvertImagesToPng,
  onClose,
  onImport,
}) => {
  const [targetDir, setTargetDir] = useState(initialTargetDir);
  const [diskPaths, setDiskPaths] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const presets = useMemo(() => {
    const base = project.edition === "bedrock" ? BEDROCK_DIR_PRESETS : JAVA_DIR_PRESETS;
    return base.map((dir) => ({
      dir,
      exists: project.all_files.some((file) => file.startsWith(`${dir}/`)),
    }));
  }, [project.edition, project.all_files]);

  const pickFiles = async () => {
    const picked = await dialogOpen({
      multiple: true,
      directory: false,
      title: "Choose assets to import",
      filters: [
        {
          name: "Minecraft assets",
          extensions: [
            "png",
            "jpg",
            "jpeg",
            "webp",
            "tga",
            "json",
            "mcmeta",
            "lang",
            "ogg",
            "wav",
            "bbmodel",
            "geo.json",
          ],
        },
        { name: "All files", extensions: ["*"] },
      ],
    });
    const list = Array.isArray(picked)
      ? picked
      : typeof picked === "string"
        ? [picked]
        : [];
    if (list.length === 0) return;
    setDiskPaths((prev) => Array.from(new Set([...prev, ...list])));
    setError(null);
  };

  const removePath = (path: string) => {
    setDiskPaths((prev) => prev.filter((item) => item !== path));
  };

  const submit = async () => {
    if (diskPaths.length === 0) {
      setError("Choose at least one file to import.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onImport(
        targetDir.trim().replace(/^\/+|\/+$/g, ""),
        diskPaths,
      );
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-dialog"
        style={{ width: 640 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header">
          <div className="modal-title">
            <span className="codicon codicon-cloud-upload" />
            Import assets
          </div>
          <button className="icon-btn" title="Close" onClick={onClose}>
            <span className="codicon codicon-close" />
          </button>
        </div>

        <div className="modal-body">
          <div className="import-dropzone">
            <span className="codicon codicon-cloud-upload" />
            <div style={{ flex: 1 }}>
              <strong>Pick files from your device</strong>
              <span>PNG, JPG, WebP, JSON, .lang, audio and Blockbench models.</span>
            </div>
            <button className="vsc-btn" onClick={() => void pickFiles()}>
              <span className="codicon codicon-folder-opened" />
              Choose files
            </button>
          </div>

          <label className="field-label">
            <span>Destination folder inside the pack</span>
            <input
              className="vsc-input mono"
              placeholder="Leave empty to import into the pack root"
              value={targetDir}
              spellCheck={false}
              onChange={(e) => setTargetDir(e.target.value)}
            />
          </label>

          <div className="chip-row">
            <button
              className={`chip ${targetDir.trim() === "" ? "active" : ""}`}
              onClick={() => setTargetDir("")}
              title="Import into the pack root"
            >
              pack root
            </button>
            {presets.map(({ dir, exists }) => (
              <button
                key={dir}
                className={`chip ${targetDir === dir ? "active" : ""}`}
                onClick={() => setTargetDir(dir)}
                title={exists ? "Folder exists in this pack" : "Folder will be created"}
              >
                {exists ? "" : "+ "}
                {dir}
              </button>
            ))}
          </div>

          <div>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                marginBottom: 5,
              }}
            >
              <span style={{ fontSize: 11, color: "var(--vsc-text-dim)" }}>
                Selected files
              </span>
              {diskPaths.length > 0 && (
                <button
                  className="vsc-btn ghost small"
                  onClick={() => setDiskPaths([])}
                >
                  Clear all
                </button>
              )}
            </div>

            {diskPaths.length === 0 ? (
              <div
                className="import-list"
                style={{
                  display: "grid",
                  placeItems: "center",
                  minHeight: 86,
                  color: "var(--vsc-text-dim)",
                  fontSize: 11.5,
                  textAlign: "center",
                  padding: 14,
                }}
              >
                No files selected yet. Choose files above, or drag &amp; drop
                them anywhere in the workbench.
              </div>
            ) : (
              <div className="import-list">
                {diskPaths.map((path) => (
                  <div className="import-file-row" key={path} title={path}>
                    <span className="codicon codicon-file-media" />
                    <span className="path">{baseName(path)}</span>
                    <span className="import-count">
                      {path.slice(0, Math.max(0, path.length - baseName(path).length - 1))}
                    </span>
                    <button
                      className="icon-btn"
                      title="Remove from import"
                      onClick={() => removePath(path)}
                    >
                      <span className="codicon codicon-close" />
                    </button>
                  </div>
                ))}
              </div>
            )}
            <div className="import-count" style={{ marginTop: 5 }}>
              {diskPaths.length} file{diskPaths.length === 1 ? "" : "s"} ready to
              import
            </div>
          </div>

          <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
            <label className="check-row">
              <input
                type="checkbox"
                checked={autoSanitize}
                onChange={(e) => onToggleAutoSanitize(e.target.checked)}
              />
              <span>
                Auto-sanitize filenames to Minecraft rules
                <span style={{ color: "var(--vsc-text-dim)" }}>
                  {" "}
                  (lowercase, a-z0-9_- only)
                </span>
              </span>
            </label>
            <label className="check-row">
              <input
                type="checkbox"
                checked={convertImagesToPng}
                onChange={(e) => onToggleConvertImagesToPng(e.target.checked)}
              />
              <span>
                Convert JPG / WebP images to PNG
                <span style={{ color: "var(--vsc-text-dim)" }}>
                  {" "}
                  (recommended for Java packs)
                </span>
              </span>
            </label>
          </div>

          {error && (
            <div className="banner failed" style={{ marginBottom: 0 }}>
              <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <span className="codicon codicon-error" />
                {error}
              </span>
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="vsc-btn secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="vsc-btn"
            disabled={busy || diskPaths.length === 0}
            onClick={() => void submit()}
          >
            <span
              className={`codicon ${busy ? "codicon-loading" : "codicon-cloud-upload"}`}
            />
            {busy
              ? "Importing..."
              : `Import ${diskPaths.length || ""} file${diskPaths.length === 1 ? "" : "s"}`}
          </button>
        </div>
      </div>
    </div>
  );
};
