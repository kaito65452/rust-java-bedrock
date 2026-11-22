import React, { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as dialogOpen, save as dialogSave } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type {
  ConversionResultDto,
  ConverterOptions,
  ConvertProgressEvent,
  PreflightReportDto,
  ProjectSnapshot,
  RotationSnapMode,
  StudioSettings,
  UvAtlasGuardMode,
} from "../types";

interface ConverterTabProps {
  project: ProjectSnapshot | null;
  settings: StudioSettings;
  onAuditWarnings: (warnings: PreflightReportDto["diagnostics"]) => void;
  onLog: (level: "info" | "warn" | "error" | "success", msg: string) => void;
  onOpenConvertedPack: (path: string) => void;
  onRevealPath: (path: string) => void;
}

export const ConverterTab: React.FC<ConverterTabProps> = ({
  project,
  settings,
  onAuditWarnings,
  onLog,
  onOpenConvertedPack,
  onRevealPath,
}) => {
  const [useActivePack, setUseActivePack] = useState(true);
  const [externalInput, setExternalInput] = useState("");
  const [direction, setDirection] = useState<
    "java_to_bedrock" | "bedrock_to_java"
  >(
    project?.edition === "bedrock" ? "bedrock_to_java" : "java_to_bedrock",
  );
  const [options, setOptions] = useState<ConverterOptions>({
    target_version: "1.21.50",
    composite_grass_tga: settings.defaultCompositeGrassTga,
    uv_atlas_guard_mode: settings.defaultUvAtlasGuard,
    convert_pbr: settings.defaultConvertPbr,
    auto_downscale: settings.defaultTargetResolution,
    strip_junk_and_raw_java: true,
    rotation_mode: "Preserve",
    bedrock_manifest_version: 2,
  });

  const [preflight, setPreflight] = useState<PreflightReportDto | null>(null);
  const [result, setResult] = useState<ConversionResultDto | null>(null);
  const [rowFilter, setRowFilter] = useState("");
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<ConvertProgressEvent | null>(null);

  useEffect(() => {
    if (project && useActivePack) {
      setDirection(
        project.edition === "bedrock" ? "bedrock_to_java" : "java_to_bedrock",
      );
    }
  }, [project?.edition, useActivePack]);

  useEffect(() => {
    const unlistenPromise = listen<ConvertProgressEvent>(
      "converter://progress",
      (ev) => setProgress(ev.payload),
    );
    return () => {
      void unlistenPromise.then((fn) => fn());
    };
  }, []);

  const resolvedInput = useActivePack
    ? null
    : externalInput.trim() || null;

  const pickExternalSource = async () => {
    const picked = await dialogOpen({
      multiple: false,
      directory: false,
      filters: [
        { name: "Resource Pack Archives", extensions: ["zip", "mcpack"] },
      ],
    });
    if (typeof picked === "string") {
      setExternalInput(picked);
      setUseActivePack(false);
    }
  };

  const pickExternalFolder = async () => {
    const picked = await dialogOpen({ multiple: false, directory: true });
    if (typeof picked === "string") {
      setExternalInput(picked);
      setUseActivePack(false);
    }
  };

  const handleRunPreflight = async () => {
    if (useActivePack && !project) {
      onLog("warn", "Open a resource pack or select an external input first.");
      return;
    }
    setRunning(true);
    setResult(null);
    onLog("info", `Running Preflight (${direction})...`);
    try {
      const report = await api.runPreflight(resolvedInput, direction, options);
      setPreflight(report);
      onAuditWarnings(report.diagnostics);
      onLog(
        report.semantic_preview_status === "failed" ? "error" : "success",
        `Preflight complete in ${report.elapsed_ms}ms: ${report.ready_count} ready, ${report.warning_count} warnings`,
      );
    } catch (e) {
      onLog("error", `Preflight failed: ${String(e)}`);
    } finally {
      setRunning(false);
    }
  };

  const handleExecuteConvert = async () => {
    if (useActivePack && !project) {
      onLog("warn", "Open a resource pack or select an external input first.");
      return;
    }
    const defaultExt = direction === "java_to_bedrock" ? "mcpack" : "zip";
    const outPath = await dialogSave({
      defaultPath: `${project?.pack_name ?? "converted_pack"}.${defaultExt}`,
      filters: [{ name: "Pack Output", extensions: ["mcpack", "zip"] }],
    });
    if (!outPath) return;

    setRunning(true);
    setProgress({ fraction: 0.05, message: "Starting conversion..." });
    onLog("info", `Converting (${direction}) -> ${outPath}`);
    try {
      const res = await api.executeConvert(
        resolvedInput,
        outPath,
        direction,
        options,
      );
      setResult(res);
      onAuditWarnings(res.warnings);
      onLog(
        res.gate_status === "failed" ? "error" : "success",
        `Convert ${res.gate_status.toUpperCase()} (${res.elapsed_ms}ms): ${res.gate_summary}`,
      );
    } catch (e) {
      onLog("error", `Conversion failed: ${String(e)}`);
    } finally {
      setRunning(false);
    }
  };

  const filteredRows = (preflight?.rows ?? []).filter((r) => {
    const q = rowFilter.trim().toLowerCase();
    if (!q) return true;
    return (
      r.source_path.toLowerCase().includes(q) ||
      r.status.toLowerCase().includes(q) ||
      r.target_hint.toLowerCase().includes(q)
    );
  });

  return (
    <div className="panel-scroll">
      <div className="card">
        <h3 className="card-title">
          <span className="codicon codicon-arrow-swap" />
          Java &harr; Bedrock Converter &amp; Preflight Gate
        </h3>

        <div className="form-grid">
          <div className="field-label">
            <span>Source Pack</span>
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
              <button
                className={`vsc-btn small ${useActivePack ? "" : "secondary"}`}
                disabled={!project}
                onClick={() => setUseActivePack(true)}
              >
                Use Active Project ({project ? project.pack_name : "None"})
              </button>
              <button
                className="vsc-btn secondary small"
                onClick={() => void pickExternalSource()}
              >
                Archive (.zip/.mcpack)
              </button>
              <button
                className="vsc-btn secondary small"
                onClick={() => void pickExternalFolder()}
              >
                Folder...
              </button>
            </div>
            {!useActivePack && (
              <input
                className="vsc-input mono"
                value={externalInput}
                onChange={(e) => setExternalInput(e.target.value)}
                placeholder="/path/to/pack"
              />
            )}
          </div>

          <label className="field-label">
            <span>Conversion Direction</span>
            <select
              className="vsc-select"
              value={direction}
              onChange={(e) =>
                setDirection(
                  e.target.value as "java_to_bedrock" | "bedrock_to_java",
                )
              }
            >
              <option value="java_to_bedrock">Java Edition &rarr; Bedrock Edition</option>
              <option value="bedrock_to_java">Bedrock Edition &rarr; Java Edition</option>
            </select>
          </label>

          <label className="field-label">
            <span>
              Target Version ({direction === "java_to_bedrock" ? "Bedrock min_engine_version normalized [1,x,x]" : "Java target"})
            </span>
            <input
              className="vsc-input mono"
              value={options.target_version}
              onChange={(e) =>
                setOptions((o) => ({ ...o, target_version: e.target.value }))
              }
            />
          </label>

          <label className="field-label">
            <span>3D UV-Atlas Guard</span>
            <select
              className="vsc-select"
              value={options.uv_atlas_guard_mode}
              onChange={(e) =>
                setOptions((o) => ({
                  ...o,
                  uv_atlas_guard_mode: e.target.value as UvAtlasGuardMode,
                }))
              }
            >
              <option value="safe_fallback2d">Safe Fallback 2D (Recommended)</option>
              <option value="skip">Skip Offending Geometry</option>
              <option value="warn">Warn Only</option>
            </select>
          </label>

          <label className="field-label">
            <span>Auto-Downscale Textures</span>
            <select
              className="vsc-select"
              value={options.auto_downscale ?? ""}
              onChange={(e) =>
                setOptions((o) => ({
                  ...o,
                  auto_downscale: e.target.value ? Number(e.target.value) : null,
                }))
              }
            >
              <option value="">Original Resolution (No Downscale)</option>
              <option value="16">16x16 Vanilla</option>
              <option value="32">32x32 HD</option>
              <option value="64">64x64 HD</option>
              <option value="128">128x128 Ultra</option>
            </select>
          </label>

          <label className="field-label">
            <span>Model Rotation Snap</span>
            <select
              className="vsc-select"
              value={options.rotation_mode}
              onChange={(e) =>
                setOptions((o) => ({
                  ...o,
                  rotation_mode: e.target.value as RotationSnapMode,
                }))
              }
            >
              <option value="Preserve">Preserve Exact Float Angles</option>
              <option value="Snap22_5">Snap 22.5&deg; Increments</option>
              <option value="Clamp45">Clamp &plusmn;45&deg;</option>
            </select>
          </label>
        </div>

        <div style={{ display: "flex", gap: 16, flexWrap: "wrap", marginTop: 12 }}>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={options.composite_grass_tga}
              onChange={(e) =>
                setOptions((o) => ({
                  ...o,
                  composite_grass_tga: e.target.checked,
                }))
              }
            />
            Composite grass_side.tga &amp; opaque grass_top
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={options.convert_pbr}
              onChange={(e) =>
                setOptions((o) => ({ ...o, convert_pbr: e.target.checked }))
              }
            />
            LabPBR (_n/_s) &rarr; Bedrock MER + texture_set.json
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={options.strip_junk_and_raw_java}
              onChange={(e) =>
                setOptions((o) => ({
                  ...o,
                  strip_junk_and_raw_java: e.target.checked,
                }))
              }
            />
            Strip OS junk &amp; non-target metadata
          </label>
        </div>

        <div style={{ display: "flex", gap: 8, marginTop: 14 }}>
          <button
            className="vsc-btn secondary"
            disabled={running}
            onClick={() => void handleRunPreflight()}
          >
            <span className="codicon codicon-checklist" />
            1. Run Preflight Scan
          </button>
          <button
            className="vsc-btn"
            disabled={running}
            onClick={() => void handleExecuteConvert()}
          >
            <span className="codicon codicon-play" />
            2. Execute Convert &amp; Audit Gate
          </button>
        </div>

        {running && progress && (
          <div style={{ marginTop: 12 }}>
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, marginBottom: 4 }}>
              <span>{progress.message}</span>
              <span>{Math.round(progress.fraction * 100)}%</span>
            </div>
            <div style={{ height: 6, background: "#333", borderRadius: 3, overflow: "hidden" }}>
              <div
                style={{
                  width: `${Math.round(progress.fraction * 100)}%`,
                  height: "100%",
                  background: "var(--vsc-status-blue)",
                  transition: "width 120ms ease",
                }}
              />
            </div>
          </div>
        )}
      </div>

      {result && (
        <div className={`banner ${result.gate_status}`}>
          <div>
            <div>
              Semantic Audit Gate: <strong>{result.gate_status.toUpperCase()}</strong> ({result.elapsed_ms} ms)
            </div>
            <div style={{ fontSize: 12, fontWeight: 400 }}>{result.gate_summary}</div>
          </div>
          {result.output_written_to && (
            <div style={{ display: "flex", gap: 6 }}>
              <button
                className="vsc-btn small"
                onClick={() => onOpenConvertedPack(result.output_written_to!)}
              >
                Open Output Pack
              </button>
              <button
                className="vsc-btn secondary small"
                onClick={() => onRevealPath(result.output_written_to!)}
              >
                Reveal
              </button>
            </div>
          )}
        </div>
      )}

      {preflight && (
        <div className="card">
          <div className={`banner ${preflight.semantic_preview_status}`}>
            <span>
              Preflight Preview ({preflight.source_edition} &rarr; {preflight.target_edition}):{" "}
              {preflight.semantic_preview_summary}
            </span>
          </div>

          <div className="stat-grid">
            <div className="stat-card">
              <div className="stat-value">{preflight.total_assets}</div>
              <div className="stat-label">Scanned Assets</div>
            </div>
            <div className="stat-card">
              <div className="stat-value" style={{ color: "var(--vsc-ok)" }}>
                {preflight.ready_count}
              </div>
              <div className="stat-label">Cleanly Mapped</div>
            </div>
            <div className="stat-card">
              <div className="stat-value" style={{ color: "#9cdcfe" }}>
                {preflight.composite_count}
              </div>
              <div className="stat-label">Grass Composites</div>
            </div>
            <div className="stat-card">
              <div className="stat-value" style={{ color: "var(--vsc-warn)" }}>
                {preflight.lossy_count}
              </div>
              <div className="stat-label">Lossy</div>
            </div>
            <div className="stat-card">
              <div className="stat-value" style={{ color: "var(--vsc-error)" }}>
                {preflight.skipped_count}
              </div>
              <div className="stat-label">Skipped</div>
            </div>
          </div>

          <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 8 }}>
            <strong>Asset Mapping Preview ({filteredRows.length})</strong>
            <input
              className="vsc-input"
              placeholder="Filter rows..."
              value={rowFilter}
              onChange={(e) => setRowFilter(e.target.value)}
            />
          </div>

          <div style={{ maxHeight: 320, overflow: "auto" }}>
            <table className="table-compact">
              <thead>
                <tr>
                  <th>Status</th>
                  <th>Source Path</th>
                  <th>Target Hint</th>
                  <th>Detail</th>
                </tr>
              </thead>
              <tbody>
                {filteredRows.slice(0, 200).map((row, idx) => (
                  <tr key={`${row.source_path}-${idx}`}>
                    <td>
                      <span
                        style={{
                          fontWeight: 600,
                          color:
                            row.status === "READY"
                              ? "var(--vsc-ok)"
                              : row.status === "ERROR" || row.status === "SKIPPED"
                                ? "var(--vsc-error)"
                                : "var(--vsc-warn)",
                        }}
                      >
                        {row.status}
                      </span>
                    </td>
                    <td style={{ fontFamily: "var(--font-mono)" }}>{row.source_path}</td>
                    <td style={{ fontFamily: "var(--font-mono)", color: "#9cdcfe" }}>
                      {row.target_hint}
                    </td>
                    <td>{row.detail}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
};
