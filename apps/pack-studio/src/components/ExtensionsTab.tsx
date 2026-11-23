import React, { useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import type {
  ExtensionsSnapshotDto,
  LspServerStatusDto,
} from "../types";

interface ExtensionsTabProps {
  snapshot: ExtensionsSnapshotDto | null;
  servers: LspServerStatusDto[];
  selectedExtensionId?: string;
  lspDiagCount: number;
  onSelectExtension: (extId: string) => void;
  onInstallPath: (sourcePath: string) => void;
  onToggleEnabled: (extId: string, enabled: boolean) => void;
  onUninstall: (extId: string) => void;
  onStartServer: (srv: LspServerStatusDto) => void;
  onStopServer: (serverId: string) => void;
  onRegisterCustomServer: (
    serverId: string,
    displayName: string,
    command: string,
    args: string[],
    languageIds: string[],
  ) => void;
  onUnregisterCustomServer: (serverId: string) => void;
}

export const ExtensionsTab: React.FC<ExtensionsTabProps> = ({
  snapshot,
  servers,
  selectedExtensionId,
  lspDiagCount,
  onSelectExtension,
  onInstallPath,
  onToggleEnabled,
  onUninstall,
  onStartServer,
  onStopServer,
  onRegisterCustomServer,
  onUnregisterCustomServer,
}) => {
  const [serverId, setServerId] = useState("json-languageserver");
  const [displayName, setDisplayName] = useState("VS Code JSON Language Server");
  const [command, setCommand] = useState("vscode-json-language-server");
  const [argsText, setArgsText] = useState("--stdio");
  const [langsText, setLangsText] = useState("json");

  const extensions = snapshot?.extensions ?? [];
  const selectedExt =
    extensions.find((e) => e.id === selectedExtensionId) ?? extensions[0] ?? null;

  const pickVsixOrZip = async () => {
    const picked = await dialogOpen({
      multiple: false,
      directory: false,
      filters: [
        { name: "VS Code / PackStudio Extension", extensions: ["vsix", "zip"] },
      ],
    });
    if (typeof picked === "string") {
      onInstallPath(picked);
    }
  };

  return (
    <div className="panel-scroll">
      <div className="banner partial">
        <div>
          <div>
            <strong>LSP &amp; Extension Compatibility Scope</strong>
          </div>
          <div style={{ fontSize: 12, fontWeight: 400, marginTop: 2 }}>
            PackStudio runs a native Rust <strong>stdio JSON-RPC LSP client</strong> plus built-in{" "}
            <strong>Minecraft Resource Pack Intelligence</strong>. Standard VS Code{" "}
            <code>.vsix</code> / <code>.zip</code> archives load <strong>declarative</strong>{" "}
            contributions (JSON Schemas, Snippets, Grammar mappings, and{" "}
            <code>packstudioLsp</code> server configs). Imperative Node.js{" "}
            <code>vscode.*</code> API JavaScript entrypoints do not execute.
          </div>
        </div>
        <button className="vsc-btn small" onClick={() => void pickVsixOrZip()}>
          <span className="codicon codicon-cloud-download" />
          Install .vsix / .zip
        </button>
      </div>

      <div className="stat-grid">
        <div className="stat-card">
          <div className="stat-value">{extensions.length}</div>
          <div className="stat-label">Installed Extensions</div>
        </div>
        <div className="stat-card">
          <div className="stat-value" style={{ color: "#9cdcfe" }}>
            {snapshot?.active_schemas.length ?? 0}
          </div>
          <div className="stat-label">Active JSON Schemas</div>
        </div>
        <div className="stat-card">
          <div className="stat-value" style={{ color: "var(--vsc-ok)" }}>
            {snapshot?.active_snippets.length ?? 0}
          </div>
          <div className="stat-label">Active Snippets</div>
        </div>
        <div className="stat-card">
          <div className="stat-value" style={{ color: "var(--vsc-warn)" }}>
            {lspDiagCount}
          </div>
          <div className="stat-label">Live LSP Diagnostics</div>
        </div>
      </div>

      {selectedExt && (
        <div className="card">
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: 10,
              flexWrap: "wrap",
            }}
          >
            <div>
              <h3 className="card-title" style={{ marginBottom: 2 }}>
                <span className="codicon codicon-extensions" />
                {selectedExt.display_name}{" "}
                <span className="edition-badge">v{selectedExt.version}</span>
              </h3>
              <div style={{ fontSize: 12, color: "var(--vsc-text-dim)" }}>
                ID: <code>{selectedExt.id}</code> | Publisher:{" "}
                <strong>{selectedExt.publisher}</strong>
              </div>
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <select
                className="vsc-select"
                value={selectedExt.id}
                onChange={(e) => onSelectExtension(e.target.value)}
              >
                {extensions.map((ext) => (
                  <option key={ext.id} value={ext.id}>
                    {ext.display_name} ({ext.id})
                  </option>
                ))}
              </select>
              <button
                className={`vsc-btn small ${selectedExt.enabled ? "secondary" : ""}`}
                onClick={() =>
                  onToggleEnabled(selectedExt.id, !selectedExt.enabled)
                }
              >
                {selectedExt.enabled ? "Disable" : "Enable"}
              </button>
              {!selectedExt.builtin && (
                <button
                  className="vsc-btn secondary small"
                  onClick={() => onUninstall(selectedExt.id)}
                >
                  Uninstall
                </button>
              )}
            </div>
          </div>

          <p style={{ marginTop: 8, marginBottom: 8 }}>{selectedExt.description}</p>

          {selectedExt.runtime_note && (
            <div className="banner partial" style={{ marginBottom: 10 }}>
              <span style={{ fontSize: 12 }}>{selectedExt.runtime_note}</span>
            </div>
          )}

          <details>
            <summary style={{ cursor: "pointer", fontSize: 12, color: "#9cdcfe" }}>
              Inspect Manifest (<code>package.json</code>)
            </summary>
            <pre
              style={{
                background: "#181818",
                padding: 10,
                borderRadius: 4,
                overflow: "auto",
                maxHeight: 240,
                fontFamily: "var(--font-mono)",
                fontSize: 12,
                userSelect: "text",
              }}
            >
              {selectedExt.raw_manifest_json}
            </pre>
          </details>
        </div>
      )}

      <div className="card">
        <h3 className="card-title">
          <span className="codicon codicon-server-process" />
          Language Server Protocol (Stdio JSON-RPC) Manager
        </h3>

        <table className="table-compact" style={{ marginBottom: 14 }}>
          <thead>
            <tr>
              <th>Server</th>
              <th>Command</th>
              <th>Languages</th>
              <th>Status</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {servers.map((srv) => (
              <tr key={srv.server_id}>
                <td>
                  <strong>{srv.display_name}</strong>
                  <div style={{ fontSize: 11, color: "var(--vsc-text-dim)" }}>
                    {srv.server_id}
                  </div>
                </td>
                <td style={{ fontFamily: "var(--font-mono)" }}>
                  {srv.command} {srv.args.join(" ")}
                </td>
                <td>{srv.language_ids.join(", ")}</td>
                <td>
                  <span
                    style={{
                      fontWeight: 600,
                      color: srv.running
                        ? "var(--vsc-ok)"
                        : srv.binary_found
                          ? "#cccccc"
                          : "var(--vsc-warn)",
                    }}
                  >
                    {srv.running
                      ? "RUNNING"
                      : srv.binary_found
                        ? "STOPPED"
                        : "BINARY MISSING"}
                  </span>
                </td>
                <td>
                  <div style={{ display: "flex", gap: 6 }}>
                    <button
                      className={`vsc-btn small ${srv.running ? "secondary" : ""}`}
                      disabled={!srv.binary_found && !srv.running}
                      onClick={() =>
                        srv.running
                          ? onStopServer(srv.server_id)
                          : onStartServer(srv)
                      }
                    >
                      {srv.running ? "Stop" : "Start"}
                    </button>
                    {!srv.builtin && (
                      <button
                        className="vsc-btn secondary small"
                        onClick={() => onUnregisterCustomServer(srv.server_id)}
                      >
                        Remove
                      </button>
                    )}
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>

        <div className="form-grid">
          <label className="field-label">
            <span>Server ID</span>
            <input
              className="vsc-input mono"
              value={serverId}
              onChange={(e) => setServerId(e.target.value)}
            />
          </label>
          <label className="field-label">
            <span>Display Name</span>
            <input
              className="vsc-input"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
            />
          </label>
          <label className="field-label">
            <span>LSP Binary Command</span>
            <input
              className="vsc-input mono"
              value={command}
              onChange={(e) => setCommand(e.target.value)}
            />
          </label>
          <label className="field-label">
            <span>Arguments &amp; Language IDs</span>
            <div style={{ display: "flex", gap: 6 }}>
              <input
                className="vsc-input mono"
                style={{ flex: 1 }}
                value={argsText}
                onChange={(e) => setArgsText(e.target.value)}
                placeholder="--stdio"
              />
              <input
                className="vsc-input mono"
                style={{ width: 140 }}
                value={langsText}
                onChange={(e) => setLangsText(e.target.value)}
                placeholder="json,molang"
              />
            </div>
          </label>
        </div>
        <div style={{ marginTop: 10 }}>
          <button
            className="vsc-btn"
            onClick={() =>
              onRegisterCustomServer(
                serverId,
                displayName,
                command,
                argsText.split(/\s+/).filter(Boolean),
                langsText
                  .split(",")
                  .map((s) => s.trim())
                  .filter(Boolean),
              )
            }
          >
            <span className="codicon codicon-add" />
            Register Stdio LSP Server
          </button>
        </div>
      </div>
    </div>
  );
};
