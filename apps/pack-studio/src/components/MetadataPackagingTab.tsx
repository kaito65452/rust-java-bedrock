import React, { useEffect, useMemo, useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type {
  PackageExportConfig,
  PackageExportSummary,
  PackMetadataDto,
  ProjectSnapshot,
} from "../types";
import {
  formatBytes,
  generateUuidV4,
  packTypeBadgeLabel,
  parseMinecraftFormatting,
} from "../utils/mcFormat";

interface MetadataPackagingTabProps {
  project: ProjectSnapshot | null;
  onProjectUpdated: (snapshot: ProjectSnapshot) => void;
  onLog: (level: "info" | "warn" | "error" | "success", msg: string) => void;
  onRevealPath: (path: string) => void;
}

export const MetadataPackagingTab: React.FC<MetadataPackagingTabProps> = ({
  project,
  onProjectUpdated,
  onLog,
  onRevealPath,
}) => {
  const [meta, setMeta] = useState<PackMetadataDto | null>(null);
  const [authorsText, setAuthorsText] = useState("");
  const [savingMeta, setSavingMeta] = useState(false);

  const defaultExportFormat = (
    p: ProjectSnapshot | null,
  ): PackageExportConfig["format"] => {
    if (!p) return "zip";
    if (p.pack_type === "bedrock_addon") return "mcaddon";
    if (p.pack_type === "bedrock_resource_pack") {
      return p.companion_bp_dir ? "mcaddon" : "mcpack";
    }
    return "zip";
  };

  const [exportCfg, setExportCfg] = useState<PackageExportConfig>({
    format: defaultExportFormat(project),
    filename_template: "{pack_name}_{version}_{edition}",
    output_dir: project?.source_path
      ? `${project.source_path}`
      : ".",
    strip_junk_files: true,
    compression: "balanced",
    run_semantic_audit: true,
    allow_export_with_errors: false,
    include_dev_sources: false,
  });
  const [exporting, setExporting] = useState(false);
  const [exportSummary, setExportSummary] =
    useState<PackageExportSummary | null>(null);

  useEffect(() => {
    if (!project) {
      setMeta(null);
      return;
    }
    void api
      .getPackMetadata()
      .then((dto) => {
        setMeta(dto);
        setAuthorsText(dto.authors.join(", "));
        setExportCfg((prev) => ({
          ...prev,
          format: defaultExportFormat(project),
          output_dir: project.source_path || prev.output_dir,
        }));
      })
      .catch((e) => onLog("error", `Failed reading metadata: ${String(e)}`));
  }, [project?.pack_name, project?.source_path, project?.edition, project?.pack_type]);

  const descriptionSpans = useMemo(
    () => parseMinecraftFormatting(meta?.description ?? ""),
    [meta?.description],
  );

  if (!project || !meta) {
    return (
      <div className="panel-scroll">
        <div className="card">
          Open a resource pack workspace to edit <code>pack.mcmeta</code> /{" "}
          <code>manifest.json</code> metadata or package a distribution archive.
        </div>
      </div>
    );
  }

  const handleSaveMetadata = async () => {
    setSavingMeta(true);
    try {
      const updated: PackMetadataDto = {
        ...meta,
        authors: authorsText
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        // Enforce Bedrock min_engine_version [1, x, x] major guard:
        bedrock_min_engine_version: [
          1,
          meta.bedrock_min_engine_version[1],
          meta.bedrock_min_engine_version[2],
        ],
      };
      const snap = await api.savePackMetadata(updated);
      setMeta(updated);
      onProjectUpdated(snap);
      onLog(
        "success",
        meta.pack_type === "bedrock_addon"
          ? "Saved linked BP/manifest.json & RP/manifest.json metadata."
          : "Saved pack metadata and regenerated manifest/mcmeta.",
      );
    } catch (e) {
      onLog("error", `Failed saving metadata: ${String(e)}`);
    } finally {
      setSavingMeta(false);
    }
  };

  const handlePickOutputDir = async () => {
    const picked = await dialogOpen({ multiple: false, directory: true });
    if (typeof picked === "string") {
      setExportCfg((c) => ({ ...c, output_dir: picked }));
    }
  };

  const handlePackage = async () => {
    setExporting(true);
    setExportSummary(null);
    onLog("info", `Packaging ${exportCfg.format.toUpperCase()}...`);
    try {
      const summary = await api.packagePack(null, exportCfg);
      setExportSummary(summary);
      onLog(
        "success",
        `Exported ${summary.output_path} (${formatBytes(summary.size_bytes)}, ${summary.asset_count} assets)`,
      );
    } catch (e) {
      onLog("error", `Export failed: ${String(e)}`);
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="panel-scroll">
      <div className="card">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h3 className="card-title">
            <span className="codicon codicon-symbol-property" />
            {meta.pack_type === "bedrock_addon"
              ? "Linked BP + RP Manifests & Identity"
              : meta.pack_type === "java_resource_pack"
                ? "Java pack.mcmeta & Identity"
                : "Bedrock manifest.json & Identity"}{" "}
            <span className="edition-badge" style={{ marginLeft: 6 }}>
              {packTypeBadgeLabel(meta.pack_type)}
            </span>
          </h3>
          <button
            className="vsc-btn"
            disabled={savingMeta}
            onClick={() => void handleSaveMetadata()}
          >
            <span className="codicon codicon-save" />
            {savingMeta ? "Saving..." : "Save Metadata"}
          </button>
        </div>

        <div className="form-grid" style={{ marginTop: 10 }}>
          <label className="field-label">
            <span>Pack Name</span>
            <input
              className="vsc-input"
              value={meta.name}
              onChange={(e) => setMeta({ ...meta, name: e.target.value })}
            />
          </label>

          <label className="field-label">
            <span>Authors (comma separated)</span>
            <input
              className="vsc-input"
              value={authorsText}
              onChange={(e) => setAuthorsText(e.target.value)}
            />
          </label>

          <label className="field-label" style={{ gridColumn: "1 / -1" }}>
            <span>Description (supports Minecraft &sect; formatting codes)</span>
            <input
              className="vsc-input"
              value={meta.description}
              onChange={(e) => setMeta({ ...meta, description: e.target.value })}
            />
            <div
              style={{
                background: "#181818",
                border: "1px solid var(--vsc-border-strong)",
                padding: "6px 10px",
                borderRadius: 4,
                minHeight: 28,
              }}
            >
              {descriptionSpans.length === 0 ? (
                <span style={{ color: "var(--vsc-text-dim)" }}>(empty description)</span>
              ) : (
                descriptionSpans.map((sp, idx) => (
                  <span
                    key={idx}
                    style={{
                      color: sp.color,
                      fontWeight: sp.bold ? 700 : 400,
                      fontStyle: sp.italic ? "italic" : "normal",
                      textDecoration:
                        [
                          sp.underline ? "underline" : "",
                          sp.strikethrough ? "line-through" : "",
                        ]
                          .filter(Boolean)
                          .join(" ") || undefined,
                    }}
                  >
                    {sp.text}
                  </span>
                ))
              )}
            </div>
          </label>

          <label className="field-label">
            <span>Version [Major, Minor, Patch]</span>
            <div style={{ display: "flex", gap: 6 }}>
              {[0, 1, 2].map((idx) => (
                <input
                  key={idx}
                  type="number"
                  min={0}
                  className="vsc-input"
                  style={{ width: 72 }}
                  value={meta.version[idx]}
                  onChange={(e) => {
                    const next: [number, number, number] = [...meta.version] as [
                      number,
                      number,
                      number,
                    ];
                    next[idx] = Math.max(0, Number(e.target.value) || 0);
                    setMeta({ ...meta, version: next });
                  }}
                />
              ))}
            </div>
          </label>

          <label className="field-label">
            <span>License</span>
            <input
              className="vsc-input"
              value={meta.license}
              onChange={(e) => setMeta({ ...meta, license: e.target.value })}
            />
          </label>

          {meta.pack_type === "java_resource_pack" ? (
            <label className="field-label">
              <span>Java pack_format</span>
              <input
                type="number"
                className="vsc-input"
                value={meta.java_pack_format}
                onChange={(e) =>
                  setMeta({
                    ...meta,
                    java_pack_format: Math.max(1, Number(e.target.value) || 34),
                  })
                }
              />
            </label>
          ) : (
            <>
              <label className="field-label">
                <span>Bedrock min_engine_version (Major locked to 1)</span>
                <div style={{ display: "flex", gap: 6 }}>
                  <input
                    className="vsc-input"
                    style={{ width: 64 }}
                    value={1}
                    readOnly
                  />
                  <input
                    type="number"
                    min={0}
                    className="vsc-input"
                    style={{ width: 72 }}
                    value={meta.bedrock_min_engine_version[1]}
                    onChange={(e) =>
                      setMeta({
                        ...meta,
                        bedrock_min_engine_version: [
                          1,
                          Math.max(0, Number(e.target.value) || 0),
                          meta.bedrock_min_engine_version[2],
                        ],
                      })
                    }
                  />
                  <input
                    type="number"
                    min={0}
                    className="vsc-input"
                    style={{ width: 72 }}
                    value={meta.bedrock_min_engine_version[2]}
                    onChange={(e) =>
                      setMeta({
                        ...meta,
                        bedrock_min_engine_version: [
                          1,
                          meta.bedrock_min_engine_version[1],
                          Math.max(0, Number(e.target.value) || 0),
                        ],
                      })
                    }
                  />
                </div>
              </label>

              <label className="field-label">
                <span>
                  {meta.pack_type === "bedrock_addon"
                    ? "RP Header UUIDv4 (auto-linked in BP dependencies)"
                    : "Header UUIDv4"}
                </span>
                <div style={{ display: "flex", gap: 6 }}>
                  <input
                    className="vsc-input mono"
                    style={{ flex: 1 }}
                    value={meta.bedrock_header_uuid}
                    onChange={(e) =>
                      setMeta({ ...meta, bedrock_header_uuid: e.target.value })
                    }
                  />
                  <button
                    className="vsc-btn secondary small"
                    onClick={() =>
                      setMeta({ ...meta, bedrock_header_uuid: generateUuidV4() })
                    }
                  >
                    New UUID
                  </button>
                </div>
              </label>

              <label className="field-label">
                <span>
                  {meta.pack_type === "bedrock_addon"
                    ? "RP Module UUIDv4"
                    : "Module UUIDv4"}
                </span>
                <div style={{ display: "flex", gap: 6 }}>
                  <input
                    className="vsc-input mono"
                    style={{ flex: 1 }}
                    value={meta.bedrock_module_uuid}
                    onChange={(e) =>
                      setMeta({ ...meta, bedrock_module_uuid: e.target.value })
                    }
                  />
                  <button
                    className="vsc-btn secondary small"
                    onClick={() =>
                      setMeta({ ...meta, bedrock_module_uuid: generateUuidV4() })
                    }
                  >
                    New UUID
                  </button>
                </div>
              </label>

              {meta.pack_type === "bedrock_addon" && (
                <>
                  <label className="field-label">
                    <span>BP Header UUIDv4 (linked in RP dependencies)</span>
                    <div style={{ display: "flex", gap: 6 }}>
                      <input
                        className="vsc-input mono"
                        style={{ flex: 1 }}
                        value={meta.bp_header_uuid ?? ""}
                        onChange={(e) =>
                          setMeta({ ...meta, bp_header_uuid: e.target.value })
                        }
                      />
                      <button
                        className="vsc-btn secondary small"
                        onClick={() =>
                          setMeta({ ...meta, bp_header_uuid: generateUuidV4() })
                        }
                      >
                        New UUID
                      </button>
                    </div>
                  </label>

                  <label className="field-label">
                    <span>BP Data Module UUIDv4</span>
                    <div style={{ display: "flex", gap: 6 }}>
                      <input
                        className="vsc-input mono"
                        style={{ flex: 1 }}
                        value={meta.bp_module_uuid ?? ""}
                        onChange={(e) =>
                          setMeta({ ...meta, bp_module_uuid: e.target.value })
                        }
                      />
                      <button
                        className="vsc-btn secondary small"
                        onClick={() =>
                          setMeta({ ...meta, bp_module_uuid: generateUuidV4() })
                        }
                      >
                        New UUID
                      </button>
                    </div>
                  </label>

                  <label className="field-label">
                    <span>BP Script Module Entry</span>
                    <input
                      className="vsc-input mono"
                      value={meta.bp_script_entry ?? ""}
                      placeholder="scripts/main.js"
                      onChange={(e) =>
                        setMeta({ ...meta, bp_script_entry: e.target.value })
                      }
                    />
                  </label>
                </>
              )}
            </>
          )}
        </div>
      </div>

      <div className="card">
        <h3 className="card-title">
          <span className="codicon codicon-package" />
          Export &amp; Package Distribution (.zip / .mcpack / .mcaddon)
        </h3>

        <div className="form-grid">
          <label className="field-label">
            <span>Package Format</span>
            <select
              className="vsc-select"
              value={exportCfg.format}
              onChange={(e) =>
                setExportCfg((c) => ({
                  ...c,
                  format: e.target.value as PackageExportConfig["format"],
                }))
              }
            >
              <option value="zip">.zip (Java / Archive)</option>
              <option value="mcpack">.mcpack (Bedrock Pack)</option>
              <option value="mcaddon">.mcaddon (RP + Companion BP)</option>
            </select>
          </label>

          <label className="field-label">
            <span>Compression</span>
            <select
              className="vsc-select"
              value={exportCfg.compression}
              onChange={(e) =>
                setExportCfg((c) => ({
                  ...c,
                  compression: e.target
                    .value as PackageExportConfig["compression"],
                }))
              }
            >
              <option value="fast">Fast</option>
              <option value="balanced">Balanced (Deflate 6)</option>
              <option value="maximum">Maximum (Smallest File)</option>
            </select>
          </label>

          <label className="field-label">
            <span>Filename Template</span>
            <input
              className="vsc-input mono"
              value={exportCfg.filename_template}
              onChange={(e) =>
                setExportCfg((c) => ({
                  ...c,
                  filename_template: e.target.value,
                }))
              }
            />
          </label>

          <label className="field-label">
            <span>Output Directory</span>
            <div style={{ display: "flex", gap: 6 }}>
              <input
                className="vsc-input mono"
                style={{ flex: 1 }}
                value={exportCfg.output_dir}
                onChange={(e) =>
                  setExportCfg((c) => ({ ...c, output_dir: e.target.value }))
                }
              />
              <button
                className="vsc-btn secondary small"
                onClick={() => void handlePickOutputDir()}
              >
                Browse
              </button>
            </div>
          </label>
        </div>

        <div style={{ display: "flex", gap: 16, flexWrap: "wrap", marginTop: 12 }}>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={exportCfg.strip_junk_files}
              onChange={(e) =>
                setExportCfg((c) => ({
                  ...c,
                  strip_junk_files: e.target.checked,
                }))
              }
            />
            Strip OS &amp; VCS junk (.DS_Store, Thumbs.db, desktop.ini, .git)
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={exportCfg.run_semantic_audit}
              onChange={(e) =>
                setExportCfg((c) => ({
                  ...c,
                  run_semantic_audit: e.target.checked,
                }))
              }
            />
            Enforce Semantic Audit Gate before packaging
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={exportCfg.allow_export_with_errors}
              onChange={(e) =>
                setExportCfg((c) => ({
                  ...c,
                  allow_export_with_errors: e.target.checked,
                }))
              }
            />
            Allow export even on Audit errors
          </label>
          {meta.pack_type === "bedrock_addon" && (
            <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <input
                type="checkbox"
                checked={!!exportCfg.include_dev_sources}
                onChange={(e) =>
                  setExportCfg((c) => ({
                    ...c,
                    include_dev_sources: e.target.checked,
                  }))
                }
              />
              Include pstdr dev sources (pstdr.toml, src/) in archive
            </label>
          )}
        </div>

        <div style={{ marginTop: 14 }}>
          <button
            className="vsc-btn"
            disabled={exporting}
            onClick={() => void handlePackage()}
          >
            <span className="codicon codicon-archive" />
            {exporting ? "Packaging..." : `Export .${exportCfg.format}`}
          </button>
        </div>

        {exportSummary && (
          <div className="banner passed" style={{ marginTop: 12 }}>
            <div>
              <div>Exported: {exportSummary.output_path}</div>
              <div style={{ fontSize: 12, fontWeight: 400 }}>
                Size: {formatBytes(exportSummary.size_bytes)} | Assets:{" "}
                {exportSummary.asset_count} | Stripped junk:{" "}
                {exportSummary.stripped_junk_count} | Gate:{" "}
                {exportSummary.audit_summary}
              </div>
            </div>
            <button
              className="vsc-btn small"
              onClick={() => onRevealPath(exportSummary.output_path)}
            >
              Reveal File
            </button>
          </div>
        )}
      </div>
    </div>
  );
};
