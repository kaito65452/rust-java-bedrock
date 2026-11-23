import React, { useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import type {
  ExtensionsSnapshotDto,
  InstalledExtensionDto,
  LspServerStatusDto,
} from "../types";

interface ExtensionsSidebarProps {
  snapshot: ExtensionsSnapshotDto | null;
  servers: LspServerStatusDto[];
  onRefresh: () => void;
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
  onOpenExtensionTab: (extId?: string) => void;
}

export const ExtensionsSidebar: React.FC<ExtensionsSidebarProps> = ({
  snapshot,
  servers,
  onRefresh,
  onInstallPath,
  onToggleEnabled,
  onUninstall,
  onStartServer,
  onStopServer,
  onRegisterCustomServer,
  onOpenExtensionTab,
}) => {
  const [query, setQuery] = useState("");
  const [addingLsp, setAddingLsp] = useState(false);
  const [newId, setNewId] = useState("custom.json-ls");
  const [newCmd, setNewCmd] = useState("vscode-json-language-server");
  const [newArgs, setNewArgs] = useState("--stdio");
  const [newLangs, setNewLangs] = useState("json");

  const pickArchive = async () => {
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

  const pickFolder = async () => {
    const picked = await dialogOpen({ multiple: false, directory: true });
    if (typeof picked === "string") {
      onInstallPath(picked);
    }
  };

  const q = query.trim().toLowerCase();
  const filteredExts = (snapshot?.extensions ?? []).filter(
    (e) =>
      !q ||
      e.display_name.toLowerCase().includes(q) ||
      e.id.toLowerCase().includes(q) ||
      e.description.toLowerCase().includes(q),
  );
  const filteredServers = servers.filter(
    (s) =>
      !q ||
      s.display_name.toLowerCase().includes(q) ||
      s.server_id.toLowerCase().includes(q) ||
      s.command.toLowerCase().includes(q),
  );

  const renderExtRow = (ext: InstalledExtensionDto) => (
    <div
      key={ext.id}
      style={{
        padding: "8px 10px",
        borderBottom: "1px solid var(--vsc-border)",
        cursor: "pointer",
      }}
      onClick={() => onOpenExtensionTab(ext.id)}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 6,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
          <span
            className={`codicon ${
              ext.builtin ? "codicon-sparkle" : "codicon-extensions"
            }`}
            style={{ color: ext.enabled ? "#3794ff" : "#8c8c8c" }}
          />
          <span
            style={{
              fontWeight: 600,
              color: ext.enabled ? "var(--vsc-text-bright)" : "var(--vsc-text-dim)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {ext.display_name}
          </span>
        </div>
        <span style={{ fontSize: 10, color: "var(--vsc-text-dim)" }}>
          v{ext.version}
        </span>
      </div>

      <div
        style={{
          fontSize: 11,
          color: "var(--vsc-text-dim)",
          marginTop: 2,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {ext.description}
      </div>

      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 6,
          marginTop: 6,
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
          {ext.builtin && <span className="edition-badge">Built-in</span>}
          {ext.schema_count > 0 && (
            <span className="edition-badge">{ext.schema_count} Schema</span>
          )}
          {ext.snippet_count > 0 && (
            <span className="edition-badge">{ext.snippet_count} Snippet</span>
          )}
          {ext.has_js_entrypoint && (
            <span
              className="edition-badge"
              style={{ color: "#e5c07b" }}
              title={ext.runtime_note ?? undefined}
            >
              JS API Skipped
            </span>
          )}
        </div>

        <div style={{ display: "flex", gap: 4 }}>
          <button
            className="vsc-btn secondary small"
            onClick={() => onToggleEnabled(ext.id, !ext.enabled)}
          >
            {ext.enabled ? "Disable" : "Enable"}
          </button>
          {!ext.builtin && (
            <button
              className="icon-btn"
              title="Uninstall Extension"
              onClick={() => onUninstall(ext.id)}
            >
              <span className="codicon codicon-trash" />
            </button>
          )}
        </div>
      </div>
    </div>
  );

  return (
    <div className="sidebar-pane">
      <div className="sidebar-header">
        <span>EXTENSIONS &amp; LSP</span>
        <div className="sidebar-actions">
          <button
            className="icon-btn"
            title="Install Extension (.vsix / .zip)"
            onClick={() => void pickArchive()}
          >
            <span className="codicon codicon-cloud-download" />
          </button>
          <button
            className="icon-btn"
            title="Install Unpacked Extension Folder"
            onClick={() => void pickFolder()}
          >
            <span className="codicon codicon-folder-opened" />
          </button>
          <button
            className="icon-btn"
            title="Open LSP & Extensions Manager"
            onClick={() => onOpenExtensionTab()}
          >
            <span className="codicon codicon-open-preview" />
          </button>
          <button
            className="icon-btn"
            title="Refresh Extensions & LSP"
            onClick={onRefresh}
          >
            <span className="codicon codicon-refresh" />
          </button>
        </div>
      </div>

      <div className="tree-filter-bar">
        <input
          className="vsc-input"
          style={{ flex: 1 }}
          placeholder="Search Extensions & LSP..."
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      <div className="tree-scroll" style={{ padding: 0 }}>
        <div
          style={{
            padding: "6px 10px",
            fontSize: 11,
            fontWeight: 700,
            color: "#bbbbbb",
            background: "#202022",
            borderBottom: "1px solid var(--vsc-border)",
          }}
        >
          INSTALLED ({filteredExts.length})
        </div>
        {filteredExts.map(renderExtRow)}

        <div
          style={{
            padding: "6px 10px",
            fontSize: 11,
            fontWeight: 700,
            color: "#bbbbbb",
            background: "#202022",
            borderBottom: "1px solid var(--vsc-border)",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <span>LANGUAGE SERVERS (LSP)</span>
          <button
            className="icon-btn"
            style={{ width: 20, height: 20 }}
            title="Register Custom Stdio LSP Binary"
            onClick={() => setAddingLsp((s) => !s)}
          >
            <span className="codicon codicon-add" />
          </button>
        </div>

        {addingLsp && (
          <div
            style={{
              padding: 10,
              background: "#1c1c1e",
              borderBottom: "1px solid var(--vsc-border)",
              display: "flex",
              flexDirection: "column",
              gap: 6,
            }}
          >
            <input
              className="vsc-input mono"
              placeholder="Server ID (e.g. json-ls)"
              value={newId}
              onChange={(e) => setNewId(e.target.value)}
            />
            <input
              className="vsc-input mono"
              placeholder="Command (e.g. vscode-json-language-server)"
              value={newCmd}
              onChange={(e) => setNewCmd(e.target.value)}
            />
            <input
              className="vsc-input mono"
              placeholder="Args (--stdio)"
              value={newArgs}
              onChange={(e) => setNewArgs(e.target.value)}
            />
            <input
              className="vsc-input mono"
              placeholder="Languages (json,mc-lang,molang)"
              value={newLangs}
              onChange={(e) => setNewLangs(e.target.value)}
            />
            <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
              <button
                className="vsc-btn secondary small"
                onClick={() => setAddingLsp(false)}
              >
                Cancel
              </button>
              <button
                className="vsc-btn small"
                onClick={() => {
                  onRegisterCustomServer(
                    newId,
                    newId,
                    newCmd,
                    newArgs.split(/\s+/).filter(Boolean),
                    newLangs
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  );
                  setAddingLsp(false);
                }}
              >
                Save LSP
              </button>
            </div>
          </div>
        )}

        {filteredServers.map((srv) => (
          <div
            key={srv.server_id}
            style={{
              padding: "8px 10px",
              borderBottom: "1px solid var(--vsc-border)",
              display: "flex",
              flexDirection: "column",
              gap: 4,
            }}
          >
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 6,
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <span
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: "50%",
                    background: srv.running
                      ? "var(--vsc-ok)"
                      : srv.binary_found
                        ? "#8c8c8c"
                        : "var(--vsc-warn)",
                  }}
                />
                <span style={{ fontWeight: 600, fontSize: 12 }}>
                  {srv.display_name}
                </span>
              </div>
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
            </div>
            <div
              style={{
                fontSize: 11,
                fontFamily: "var(--font-mono)",
                color: "var(--vsc-text-dim)",
              }}
            >
              {srv.command} {srv.args.join(" ")} [{srv.language_ids.join(", ")}]
            </div>
            {!srv.binary_found && (
              <div style={{ fontSize: 11, color: "var(--vsc-warn)" }}>
                Executable not found on PATH
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
};
