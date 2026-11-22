import React, { useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import type { NewProjectConfig, ProjectTemplateKind } from "../types";

interface NewProjectWizardModalProps {
  onClose: () => void;
  onCreate: (cfg: NewProjectConfig) => Promise<void>;
}

const COLOR_SNIPPETS = [
  ["§a", "#55FF55"],
  ["§b", "#55FFFF"],
  ["§e", "#FFFF55"],
  ["§6", "#FFAA00"],
  ["§d", "#FF55FF"],
  ["§l", "#FFFFFF"],
  ["§r", "#CCCCCC"],
] as const;

export const NewProjectWizardModal: React.FC<NewProjectWizardModalProps> = ({
  onClose,
  onCreate,
}) => {
  const [kind, setKind] = useState<ProjectTemplateKind>("bedrock_resource_pack");
  const [packName, setPackName] = useState("My Custom Resource Pack");
  const [description, setDescription] = useState("§bCrafted in §lPackStudio");
  const [authors, setAuthors] = useState("Artist");
  const [license, setLicense] = useState("ARR");
  const [url, setUrl] = useState("");
  const [version, setVersion] = useState<[number, number, number]>([1, 0, 0]);
  const [javaPackFormat, setJavaPackFormat] = useState(46);
  const [javaNamespace, setJavaNamespace] = useState("minecraft");
  const [bedrockMinor, setBedrockMinor] = useState(21);
  const [bedrockPatch, setBedrockPatch] = useState(50);
  const [targetParentDir, setTargetParentDir] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [errMsg, setErrMsg] = useState<string | null>(null);

  const pickParentDir = async () => {
    const picked = await dialogOpen({ multiple: false, directory: true });
    if (typeof picked === "string") {
      setTargetParentDir(picked);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!targetParentDir.trim()) {
      setErrMsg("Please choose a parent folder where the project directory will be created.");
      return;
    }
    setErrMsg(null);
    setSubmitting(true);
    try {
      await onCreate({
        kind,
        pack_name: packName.trim() || "Custom Pack",
        description: description.trim(),
        authors: authors
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
        license: license.trim(),
        url: url.trim(),
        version,
        java_pack_format: javaPackFormat,
        java_supported_formats: kind === "java_resource_pack" ? [34, javaPackFormat] : null,
        java_namespace: javaNamespace.trim() || "minecraft",
        bedrock_min_engine_version: [1, bedrockMinor, bedrockPatch],
        icon_path: null,
        target_parent_dir: targetParentDir.trim(),
      });
      onClose();
    } catch (err) {
      setErrMsg(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <form
        className="modal-dialog"
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => void handleSubmit(e)}
      >
        <div className="sidebar-header" style={{ height: 40, fontSize: 12 }}>
          <span>New Minecraft Resource Pack / Add-On Wizard</span>
          <button type="button" className="icon-btn" onClick={onClose}>
            <span className="codicon codicon-close" />
          </button>
        </div>

        <div className="panel-scroll" style={{ padding: 16 }}>
          <div style={{ display: "flex", gap: 8, marginBottom: 14 }}>
            {(
              [
                ["java_resource_pack", "Java RP", "pack.mcmeta + assets/"],
                ["bedrock_resource_pack", "Bedrock RP", "manifest.json [1,x,x]"],
                ["bedrock_addon_workspace", "Bedrock Add-On", "Linked RP + BP"],
              ] as const
            ).map(([k, title, sub]) => (
              <button
                key={k}
                type="button"
                className={`stat-card`}
                style={{
                  flex: 1,
                  textAlign: "left",
                  cursor: "pointer",
                  borderColor: kind === k ? "var(--vsc-status-blue)" : "var(--vsc-border-strong)",
                  background: kind === k ? "#094771" : "#1f1f21",
                  color: "var(--vsc-text-bright)",
                }}
                onClick={() => setKind(k)}
              >
                <div style={{ fontWeight: 700, fontSize: 13 }}>{title}</div>
                <div style={{ fontSize: 11, color: "#bbbbbb" }}>{sub}</div>
              </button>
            ))}
          </div>

          <div className="form-grid">
            <label className="field-label">
              <span>Pack Name</span>
              <input
                className="vsc-input"
                required
                value={packName}
                onChange={(e) => setPackName(e.target.value)}
              />
            </label>

            <label className="field-label">
              <span>Authors (comma separated)</span>
              <input
                className="vsc-input"
                value={authors}
                onChange={(e) => setAuthors(e.target.value)}
              />
            </label>

            <label className="field-label" style={{ gridColumn: "1 / -1" }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <span>Description</span>
                <div style={{ display: "flex", gap: 4 }}>
                  {COLOR_SNIPPETS.map(([code, col]) => (
                    <button
                      key={code}
                      type="button"
                      className="vsc-btn secondary small"
                      style={{ color: col, padding: "1px 5px" }}
                      onClick={() => setDescription((d) => d + code)}
                    >
                      {code}
                    </button>
                  ))}
                </div>
              </div>
              <input
                className="vsc-input"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
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
                    style={{ width: 68 }}
                    value={version[idx]}
                    onChange={(e) => {
                      const next: [number, number, number] = [...version] as [
                        number,
                        number,
                        number,
                      ];
                      next[idx] = Math.max(0, Number(e.target.value) || 0);
                      setVersion(next);
                    }}
                  />
                ))}
              </div>
            </label>

            <label className="field-label">
              <span>License</span>
              <input
                className="vsc-input"
                value={license}
                onChange={(e) => setLicense(e.target.value)}
              />
            </label>

            {kind === "java_resource_pack" ? (
              <>
                <label className="field-label">
                  <span>Java pack_format</span>
                  <input
                    type="number"
                    className="vsc-input"
                    value={javaPackFormat}
                    onChange={(e) =>
                      setJavaPackFormat(Math.max(1, Number(e.target.value) || 46))
                    }
                  />
                </label>
                <label className="field-label">
                  <span>Namespace</span>
                  <input
                    className="vsc-input mono"
                    value={javaNamespace}
                    onChange={(e) => setJavaNamespace(e.target.value)}
                  />
                </label>
              </>
            ) : (
              <label className="field-label">
                <span>Bedrock min_engine_version (Major locked 1)</span>
                <div style={{ display: "flex", gap: 6 }}>
                  <input className="vsc-input" style={{ width: 58 }} value={1} readOnly />
                  <input
                    type="number"
                    min={0}
                    className="vsc-input"
                    style={{ width: 68 }}
                    value={bedrockMinor}
                    onChange={(e) =>
                      setBedrockMinor(Math.max(0, Number(e.target.value) || 21))
                    }
                  />
                  <input
                    type="number"
                    min={0}
                    className="vsc-input"
                    style={{ width: 68 }}
                    value={bedrockPatch}
                    onChange={(e) =>
                      setBedrockPatch(Math.max(0, Number(e.target.value) || 0))
                    }
                  />
                </div>
              </label>
            )}

            <label className="field-label" style={{ gridColumn: "1 / -1" }}>
              <span>Parent Output Directory</span>
              <div style={{ display: "flex", gap: 6 }}>
                <input
                  className="vsc-input mono"
                  style={{ flex: 1 }}
                  value={targetParentDir}
                  onChange={(e) => setTargetParentDir(e.target.value)}
                  placeholder="/home/artist/MinecraftPacks"
                />
                <button
                  type="button"
                  className="vsc-btn secondary"
                  onClick={() => void pickParentDir()}
                >
                  Browse...
                </button>
              </div>
            </label>

            <label className="field-label" style={{ gridColumn: "1 / -1" }}>
              <span>Project URL (optional)</span>
              <input
                className="vsc-input"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://..."
              />
            </label>
          </div>

          {errMsg && (
            <div className="banner failed" style={{ marginTop: 12 }}>
              {errMsg}
            </div>
          )}
        </div>

        <div
          style={{
            padding: "10px 16px",
            borderTop: "1px solid var(--vsc-border)",
            display: "flex",
            justifyContent: "flex-end",
            gap: 8,
          }}
        >
          <button type="button" className="vsc-btn secondary" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="vsc-btn" disabled={submitting}>
            <span className="codicon codicon-add" />
            {submitting ? "Creating..." : "Create Resource Pack"}
          </button>
        </div>
      </form>
    </div>
  );
};
