import React, { useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import type {
  AiAgentsStatusDto,
  AiContextPacketDto,
  ConversionWarning,
  DetectedFileManagers,
  GithubAuthStatusDto,
  OutputLogLine,
  ProjectSnapshot,
  PstdrSyncStatusDto,
  PstdrUpdateOfferDto,
  StudioSettings,
  StudioUpdateStatusDto,
  UvAtlasGuardMode,
} from "../types";

export type SettingsCategory = "updates" | "github" | "ai-agents" | "workbench";

interface SettingsViewProps {
  initialCategory?: SettingsCategory;
  project: ProjectSnapshot | null;
  activeFilePath: string | null;
  diagnostics: ConversionWarning[];
  studioSettings: StudioSettings;
  fileManagers: DetectedFileManagers | null;
  pstdrStatus: PstdrSyncStatusDto | null;
  pstdrOffer: PstdrUpdateOfferDto | null;
  githubStatus: GithubAuthStatusDto | null;
  aiAgentsStatus: AiAgentsStatusDto | null;
  onStudioSettingsChange: (next: StudioSettings) => void;
  onPstdrStatusChange: (next: PstdrSyncStatusDto) => void;
  onOpenPstdrUpdateModal: () => void;
  onGithubStatusChange: (next: GithubAuthStatusDto) => void;
  onAiAgentsStatusChange: (next: AiAgentsStatusDto) => void;
  onLog: (level: OutputLogLine["level"], message: string) => void;
}

const CATEGORIES: {
  id: SettingsCategory;
  label: string;
  icon: string;
  keywords: string;
}[] = [
  {
    id: "updates",
    label: "Updates & PSTDR LSP",
    icon: "codicon-cloud-download",
    keywords: "update studio version pstdr lsp sync ed25519 sha256 spec revert release",
  },
  {
    id: "github",
    label: "GitHub Account",
    icon: "codicon-github-inverted",
    keywords: "github account profile avatar photo pat token login connect disconnect",
  },
  {
    id: "ai-agents",
    label: "AI Coding Agents",
    icon: "codicon-sparkle",
    keywords: "ai agent claude code antigravity opencode context packet cli path bridge",
  },
  {
    id: "workbench",
    label: "Workbench & Files",
    icon: "codicon-settings-gear",
    keywords: "file manager yazi import sanitize png uv atlas guard converter defaults",
  },
];

export const SettingsView: React.FC<SettingsViewProps> = ({
  initialCategory = "updates",
  project,
  activeFilePath,
  diagnostics,
  studioSettings,
  fileManagers,
  pstdrStatus,
  pstdrOffer,
  githubStatus,
  aiAgentsStatus,
  onStudioSettingsChange,
  onPstdrStatusChange,
  onOpenPstdrUpdateModal,
  onGithubStatusChange,
  onAiAgentsStatusChange,
  onLog,
}) => {
  const [selectedCategory, setSelectedCategory] =
    useState<SettingsCategory>(initialCategory);
  const [searchQuery, setSearchQuery] = useState("");

  // Studio Update state
  const [studioUpdate, setStudioUpdate] = useState<StudioUpdateStatusDto | null>(null);
  const [checkingStudio, setCheckingStudio] = useState(false);
  const [studioUpdateErr, setStudioUpdateErr] = useState<string | null>(null);

  // PSTDR Revert state
  const [revertingPstdr, setRevertingPstdr] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  // GitHub PAT state
  const [patInput, setPatInput] = useState("");
  const [githubBusy, setGithubBusy] = useState(false);
  const [githubErr, setGithubErr] = useState<string | null>(null);
  const [avatarLoadFailed, setAvatarLoadFailed] = useState(false);

  // AI Coding Agents state
  const [aiBusy, setAiBusy] = useState(false);
  const [aiErr, setAiErr] = useState<string | null>(null);
  const [editingAgentId, setEditingAgentId] = useState<string | null>(null);
  const [customCmd, setCustomCmd] = useState("");
  const [customArgs, setCustomArgs] = useState("");
  const [contextPacket, setContextPacket] = useState<AiContextPacketDto | null>(null);

  useEffect(() => {
    setAvatarLoadFailed(false);
  }, [githubStatus?.avatar_url]);

  const copyToClipboard = (id: string, value: string) => {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      void navigator.clipboard.writeText(value).catch(() => {});
    }
    setCopiedId(id);
    window.setTimeout(() => {
      setCopiedId((cur) => (cur === id ? null : cur));
    }, 1800);
  };

  const q = searchQuery.trim().toLowerCase();
  const visibleCategories = useMemo(() => {
    if (!q) return [selectedCategory];
    const matched = CATEGORIES.filter(
      (c) =>
        c.label.toLowerCase().includes(q) || c.keywords.toLowerCase().includes(q),
    ).map((c) => c.id);
    return matched.length > 0 ? matched : [selectedCategory];
  }, [q, selectedCategory]);

  const handleCheckStudioUpdate = async () => {
    setCheckingStudio(true);
    setStudioUpdateErr(null);
    try {
      const res = await api.studioCheckUpdate();
      setStudioUpdate(res);
      if (res.update_available) {
        onLog("info", `PackStudio update available: v${res.latest_version}`);
      } else {
        onLog(
          "info",
          res.note ?? `PackStudio v${res.current_version} is up to date.`,
        );
      }
    } catch (e) {
      const msg = String(e);
      setStudioUpdateErr(msg);
      onLog("error", `Studio update check failed: ${msg}`);
    } finally {
      setCheckingStudio(false);
    }
  };

  const handleRevertPstdr = async () => {
    setRevertingPstdr(true);
    try {
      const next = await api.pstdrSyncRevertBuiltin();
      onPstdrStatusChange(next);
      onLog("info", `Reverted PSTDR runtime to built-in v${next.builtin_version}.`);
    } catch (e) {
      onLog("error", `Failed reverting PSTDR runtime: ${String(e)}`);
    } finally {
      setRevertingPstdr(false);
    }
  };

  const handleConnectGithub = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!patInput.trim() || githubBusy) return;
    setGithubBusy(true);
    setGithubErr(null);
    try {
      const next = await api.githubConnectPat(patInput.trim());
      setPatInput("");
      onGithubStatusChange(next);
      onLog("success", `Connected GitHub account @${next.login ?? "unknown"}.`);
    } catch (err) {
      const msg = String(err);
      setGithubErr(msg);
      onLog("error", `GitHub connect failed: ${msg}`);
    } finally {
      setGithubBusy(false);
    }
  };

  const handleDisconnectGithub = async () => {
    setGithubBusy(true);
    setGithubErr(null);
    try {
      const next = await api.githubDisconnect();
      onGithubStatusChange(next);
      onLog("info", "Disconnected GitHub account and removed stored token.");
    } catch (err) {
      setGithubErr(String(err));
    } finally {
      setGithubBusy(false);
    }
  };

  const handleRefreshAiAgents = async () => {
    setAiBusy(true);
    setAiErr(null);
    try {
      const next = await api.aiAgentsStatus();
      onAiAgentsStatusChange(next);
    } catch (err) {
      setAiErr(String(err));
    } finally {
      setAiBusy(false);
    }
  };

  const handleSelectAiAgent = async (agentId: string) => {
    setAiBusy(true);
    setAiErr(null);
    try {
      const next = await api.aiSetActiveAgent(agentId);
      onAiAgentsStatusChange(next);
      onLog("info", `Active AI coding agent set to '${agentId}'.`);
    } catch (err) {
      setAiErr(String(err));
    } finally {
      setAiBusy(false);
    }
  };

  const handleSaveAgentConfig = async (agentId: string) => {
    setAiBusy(true);
    setAiErr(null);
    try {
      const parsedArgs = customArgs
        .trim()
        .split(/\s+/)
        .filter((s) => s.length > 0);
      const next = await api.aiConfigureAgent(agentId, customCmd.trim(), parsedArgs);
      onAiAgentsStatusChange(next);
      setEditingAgentId(null);
      onLog("success", `Saved custom invocation for '${agentId}'.`);
    } catch (err) {
      setAiErr(String(err));
    } finally {
      setAiBusy(false);
    }
  };

  const handleBuildContextPacket = async () => {
    setAiBusy(true);
    setAiErr(null);
    try {
      const diagSummary =
        diagnostics.length > 0
          ? diagnostics
              .slice(0, 40)
              .map(
                (d) =>
                  `[${(d.severity ?? "info").toUpperCase()}] ${d.source_path || d.asset}: ${d.reason || d.message}`,
              )
              .join("\n")
          : null;
      const packet = await api.aiBuildContextPacket(
        project?.source_path ?? null,
        activeFilePath,
        diagSummary,
      );
      setContextPacket(packet);
      copyToClipboard("ai-context", packet.markdown);
      onLog(
        "success",
        `Built & copied AI context packet for ${packet.agent_name} (${packet.markdown.length} chars).`,
      );
    } catch (err) {
      const msg = String(err);
      setAiErr(msg);
      onLog("error", `Failed building AI context packet: ${msg}`);
    } finally {
      setAiBusy(false);
    }
  };

  const showSection = (id: SettingsCategory) => visibleCategories.includes(id);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        background: "var(--vsc-bg-editor, #1e1e1e)",
        color: "var(--vsc-text, #cccccc)",
      }}
    >
      {/* Top VS Code Settings Search Bar */}
      <div
        style={{
          padding: "12px 18px",
          borderBottom: "1px solid var(--vsc-border)",
          background: "#1b1b1d",
          display: "flex",
          alignItems: "center",
          gap: 10,
          flexShrink: 0,
        }}
      >
        <span className="codicon codicon-settings-gear" style={{ color: "#3794ff", fontSize: 16 }} />
        <strong style={{ fontSize: 13, color: "#ffffff", whiteSpace: "nowrap" }}>
          Settings
        </strong>
        <div style={{ position: "relative", flex: 1, maxWidth: 640 }}>
          <input
            className="vsc-input"
            style={{ width: "100%", paddingRight: 28 }}
            placeholder="Search settings (updates, pstdr, github, claude, antigravity, opencode, file manager)..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button
              className="icon-btn"
              style={{ position: "absolute", right: 4, top: 3 }}
              title="Clear search"
              onClick={() => setSearchQuery("")}
            >
              <span className="codicon codicon-close" />
            </button>
          )}
        </div>
        <span style={{ fontSize: 11, color: "var(--vsc-text-dim)", whiteSpace: "nowrap" }}>
          Ctrl+,
        </span>
      </div>

      {/* Main Split: Left Category Tree + Right Setting Cards */}
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        {/* Left Navigation Sidebar */}
        <div
          style={{
            width: 225,
            flexShrink: 0,
            borderRight: "1px solid var(--vsc-border)",
            background: "#18181a",
            padding: "10px 8px",
            display: "flex",
            flexDirection: "column",
            gap: 4,
            overflowY: "auto",
          }}
        >
          {CATEGORIES.map((cat) => {
            const isSelected = !q && selectedCategory === cat.id;
            return (
              <button
                key={cat.id}
                type="button"
                onClick={() => {
                  setSearchQuery("");
                  setSelectedCategory(cat.id);
                }}
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  gap: 8,
                  padding: "7px 10px",
                  borderRadius: 4,
                  border: "none",
                  cursor: "pointer",
                  textAlign: "left",
                  fontSize: 12,
                  background: isSelected ? "#094771" : "transparent",
                  color: isSelected ? "#ffffff" : "#cccccc",
                }}
              >
                <span style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                  <span className={`codicon ${cat.icon}`} />
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {cat.label}
                  </span>
                </span>
                {cat.id === "updates" && pstdrOffer?.update_available && (
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: "50%",
                      background: "#3fb950",
                      flexShrink: 0,
                    }}
                    title={`PSTDR v${pstdrOffer.version} available`}
                  />
                )}
                {cat.id === "github" && githubStatus?.connected && (
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: "50%",
                      background: "#3fb950",
                      flexShrink: 0,
                    }}
                    title={`Connected as @${githubStatus.login}`}
                  />
                )}
              </button>
            );
          })}
        </div>

        {/* Right Scrollable Settings Content */}
        <div
          style={{
            flex: 1,
            minWidth: 0,
            overflowY: "auto",
            padding: "18px 24px",
            display: "flex",
            flexDirection: "column",
            gap: 22,
          }}
        >
          {/* ─── SECTION 1: Updates & PSTDR LSP ─── */}
          {showSection("updates") && (
            <section style={{ display: "flex", flexDirection: "column", gap: 14 }}>
              <div style={{ borderBottom: "1px solid var(--vsc-border)", paddingBottom: 6 }}>
                <h3 style={{ margin: 0, fontSize: 15, color: "#ffffff" }}>
                  Updates &amp; PSTDR LSP
                </h3>
                <p style={{ margin: "4px 0 0", fontSize: 12, color: "var(--vsc-text-dim)" }}>
                  Manage PackStudio application releases and cryptographically verified PSTDR compiler/LSP updates.
                </p>
              </div>

              {/* Card A: PackStudio App Update */}
              <div
                style={{
                  padding: 14,
                  borderRadius: 5,
                  background: "#222326",
                  border: "1px solid var(--vsc-border)",
                  display: "flex",
                  flexDirection: "column",
                  gap: 10,
                }}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 600, color: "#ffffff" }}>
                      PackStudio App Update
                    </div>
                    <div style={{ fontSize: 12, color: "var(--vsc-text-dim)", marginTop: 2 }}>
                      Installed version:{" "}
                      <span style={{ fontFamily: "var(--font-mono)", color: "#9cdcfe" }}>
                        v{studioUpdate?.current_version ?? "0.2.3"}
                      </span>
                    </div>
                  </div>
                  <button
                    className="vsc-btn small"
                    disabled={checkingStudio}
                    onClick={() => void handleCheckStudioUpdate()}
                  >
                    <span className="codicon codicon-refresh" />
                    {checkingStudio ? "Checking..." : "Check Studio Update"}
                  </button>
                </div>

                {studioUpdateErr && (
                  <div style={{ fontSize: 11, color: "#ff8080", fontFamily: "var(--font-mono)" }}>
                    {studioUpdateErr}
                  </div>
                )}

                {studioUpdate && (
                  <div
                    style={{
                      padding: "8px 10px",
                      borderRadius: 4,
                      background: studioUpdate.update_available ? "#163825" : "#181a1d",
                      border: studioUpdate.update_available
                        ? "1px solid #3fb950"
                        : "1px solid var(--vsc-border)",
                      fontSize: 12,
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                      gap: 10,
                      flexWrap: "wrap",
                    }}
                  >
                    <span>
                      {studioUpdate.update_available
                        ? `New PackStudio release available: v${studioUpdate.latest_version}`
                        : studioUpdate.note ??
                          `Up to date (latest published release: v${studioUpdate.latest_version}).`}
                    </span>
                    <button
                      className="vsc-btn secondary small"
                      onClick={() => void openUrl(studioUpdate.release_url)}
                    >
                      <span className="codicon codicon-link-external" />
                      View Releases
                    </button>
                  </div>
                )}
              </div>

              {/* Card B: PSTDR LSP & Spec Sync */}
              <div
                style={{
                  padding: 14,
                  borderRadius: 5,
                  background: "#222326",
                  border: "1px solid var(--vsc-border)",
                  display: "flex",
                  flexDirection: "column",
                  gap: 10,
                }}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                  <div>
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span style={{ fontSize: 13, fontWeight: 600, color: "#ffffff" }}>
                        PSTDR LSP &amp; Spec Sync
                      </span>
                      <span className="edition-badge">
                        {pstdrStatus?.external_runtime_active ? "SYNCED" : "BUILT-IN"}
                      </span>
                      {pstdrOffer?.update_available && (
                        <span
                          style={{
                            padding: "2px 6px",
                            borderRadius: 3,
                            background: "#163825",
                            border: "1px solid #3fb950",
                            color: "#7ee787",
                            fontSize: 10.5,
                            fontWeight: 600,
                          }}
                        >
                          v{pstdrOffer.version} Verified
                        </span>
                      )}
                    </div>
                    <div style={{ fontSize: 12, color: "var(--vsc-text-dim)", marginTop: 3 }}>
                      Active PSTDR:{" "}
                      <span style={{ fontFamily: "var(--font-mono)", color: "#9cdcfe" }}>
                        v{pstdrStatus?.active_version ?? "0.1.0-pre-beta.1"}
                      </span>{" "}
                      &bull; Spec:{" "}
                      <strong>{pstdrStatus?.active_spec_version ?? "Bundled"}</strong>
                    </div>
                  </div>
                  <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                    {pstdrStatus?.external_runtime_active && (
                      <button
                        className="vsc-btn secondary small"
                        disabled={revertingPstdr}
                        onClick={() => void handleRevertPstdr()}
                      >
                        <span className="codicon codicon-discard" />
                        {revertingPstdr ? "Reverting..." : "Revert to Built-in"}
                      </button>
                    )}
                    <button
                      className="vsc-btn small"
                      onClick={onOpenPstdrUpdateModal}
                    >
                      <span className="codicon codicon-shield" />
                      Check &amp; Sync PSTDR LSP
                    </button>
                  </div>
                </div>

                <div
                  style={{
                    padding: "8px 10px",
                    borderRadius: 4,
                    background: "#18191c",
                    border: "1px solid var(--vsc-border)",
                    fontSize: 11,
                    display: "flex",
                    flexDirection: "column",
                    gap: 4,
                  }}
                >
                  <div style={{ color: "var(--vsc-text-dim)" }}>
                    Zero-trust verification: Ed25519 signature (hardcoded key{" "}
                    <code>{pstdrStatus?.key_id ?? "saryu-pstdr-root-2026"}</code>) + SHA-256
                    quarantine + manual approval modal.
                  </div>
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
                    <span
                      style={{
                        fontFamily: "var(--font-mono)",
                        color: "#9cdcfe",
                        wordBreak: "break-all",
                      }}
                    >
                      Fingerprint: {pstdrStatus?.pubkey_fingerprint ?? "Loading..."}
                    </span>
                    {pstdrStatus?.pubkey_configured && (
                      <button
                        className="vsc-btn secondary small"
                        onClick={() =>
                          copyToClipboard("settings-pubkey", pstdrStatus.pubkey_fingerprint)
                        }
                      >
                        <span className="codicon codicon-copy" />
                        {copiedId === "settings-pubkey" ? "Copied" : "Copy"}
                      </button>
                    )}
                  </div>
                </div>
              </div>
            </section>
          )}

          {/* ─── SECTION 2: GitHub Account ─── */}
          {showSection("github") && (
            <section style={{ display: "flex", flexDirection: "column", gap: 14 }}>
              <div style={{ borderBottom: "1px solid var(--vsc-border)", paddingBottom: 6 }}>
                <h3 style={{ margin: 0, fontSize: 15, color: "#ffffff" }}>
                  GitHub Account
                </h3>
                <p style={{ margin: "4px 0 0", fontSize: 12, color: "var(--vsc-text-dim)" }}>
                  Connect your GitHub profile for higher release API rate limits and identity display.
                </p>
              </div>

              <div
                style={{
                  padding: 16,
                  borderRadius: 5,
                  background: "#222326",
                  border: "1px solid var(--vsc-border)",
                  display: "flex",
                  flexDirection: "column",
                  gap: 12,
                }}
              >
                {githubStatus?.connected ? (
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 16, flexWrap: "wrap" }}>
                    <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
                      {githubStatus.avatar_url && !avatarLoadFailed ? (
                        <img
                          src={githubStatus.avatar_url}
                          alt={githubStatus.login ?? "GitHub avatar"}
                          onError={() => setAvatarLoadFailed(true)}
                          style={{
                            width: 56,
                            height: 56,
                            borderRadius: "50%",
                            border: "2px solid #3fb950",
                            objectFit: "cover",
                            background: "#18181a",
                          }}
                        />
                      ) : (
                        <div
                          style={{
                            width: 56,
                            height: 56,
                            borderRadius: "50%",
                            border: "2px solid #3fb950",
                            background: "#163825",
                            color: "#7ee787",
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "center",
                            fontSize: 22,
                            fontWeight: 700,
                          }}
                        >
                          {(githubStatus.login ?? "G").slice(0, 1).toUpperCase()}
                        </div>
                      )}

                      <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
                        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                          <strong style={{ fontSize: 14, color: "#ffffff" }}>
                            {githubStatus.name ?? githubStatus.login}
                          </strong>
                          <span
                            style={{
                              padding: "1px 6px",
                              borderRadius: 3,
                              background: "#163825",
                              border: "1px solid #3fb950",
                              color: "#7ee787",
                              fontSize: 10.5,
                              fontWeight: 600,
                            }}
                          >
                            Connected
                          </span>
                        </div>
                        <span style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "#9cdcfe" }}>
                          @{githubStatus.login}
                        </span>
                        {githubStatus.html_url && (
                          <button
                            type="button"
                            onClick={() => void openUrl(githubStatus.html_url!)}
                            style={{
                              background: "none",
                              border: "none",
                              padding: 0,
                              color: "#3794ff",
                              cursor: "pointer",
                              fontSize: 11.5,
                              textAlign: "left",
                            }}
                          >
                            {githubStatus.html_url}
                          </button>
                        )}
                      </div>
                    </div>

                    <button
                      className="vsc-btn secondary small"
                      disabled={githubBusy}
                      onClick={() => void handleDisconnectGithub()}
                    >
                      <span className="codicon codicon-sign-out" />
                      {githubBusy ? "Disconnecting..." : "Disconnect"}
                    </button>
                  </div>
                ) : (
                  <form
                    onSubmit={(e) => void handleConnectGithub(e)}
                    style={{ display: "flex", flexDirection: "column", gap: 10 }}
                  >
                    <div style={{ fontSize: 12, color: "#d4d4d4", lineHeight: 1.5 }}>
                      Create a GitHub Personal Access Token (fine-grained or classic with{" "}
                      <code>read:user</code>) and paste it below. PackStudio validates it against{" "}
                      <code>https://api.github.com/user</code> to load your profile avatar and username.
                    </div>
                    <div style={{ display: "flex", gap: 8 }}>
                      <input
                        type="password"
                        className="vsc-input mono"
                        style={{ flex: 1 }}
                        placeholder="ghp_... or github_pat_..."
                        value={patInput}
                        onChange={(e) => setPatInput(e.target.value)}
                        autoComplete="off"
                      />
                      <button
                        type="submit"
                        className="vsc-btn small"
                        disabled={githubBusy || !patInput.trim()}
                      >
                        <span className="codicon codicon-plug" />
                        {githubBusy ? "Connecting..." : "Connect GitHub"}
                      </button>
                    </div>
                  </form>
                )}

                {githubErr && (
                  <div
                    style={{
                      padding: "8px 10px",
                      borderRadius: 4,
                      background: "#3a1618",
                      border: "1px solid #b53d43",
                      color: "#ffb4b8",
                      fontSize: 11.5,
                    }}
                  >
                    {githubErr}
                  </div>
                )}

                <div style={{ fontSize: 11, color: "var(--vsc-text-dim)" }}>
                  <span className="codicon codicon-lock" /> Security: Token is stored locally with{" "}
                  <code>0600</code> Unix permissions (
                  <code>{githubStatus?.storage_path ?? "~/.config/pack-studio/github-auth.json"}</code>
                  ) and is never returned to the frontend WebView.
                </div>
              </div>
            </section>
          )}

          {/* ─── SECTION 3: AI Coding Agents ─── */}
          {showSection("ai-agents") && (
            <section style={{ display: "flex", flexDirection: "column", gap: 14 }}>
              <div
                style={{
                  borderBottom: "1px solid var(--vsc-border)",
                  paddingBottom: 6,
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-end",
                  gap: 10,
                }}
              >
                <div>
                  <h3 style={{ margin: 0, fontSize: 15, color: "#ffffff" }}>
                    AI Coding Agents (Claude Code, Antigravity &amp; OpenCode)
                  </h3>
                  <p style={{ margin: "4px 0 0", fontSize: 12, color: "var(--vsc-text-dim)" }}>
                    Detect installed CLI coding agents, configure preferred invocations, and export workspace + `.pstdr` context packets.
                  </p>
                </div>
                <button
                  className="vsc-btn secondary small"
                  disabled={aiBusy}
                  onClick={() => void handleRefreshAiAgents()}
                >
                  <span className="codicon codicon-refresh" />
                  Refresh Detection
                </button>
              </div>

              {aiErr && (
                <div style={{ fontSize: 12, color: "#ff8080" }}>{aiErr}</div>
              )}

              <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                {(aiAgentsStatus?.agents ?? []).map((agent) => {
                  const isActive = aiAgentsStatus?.active_agent_id === agent.id;
                  const isEditing = editingAgentId === agent.id;
                  return (
                    <div
                      key={agent.id}
                      style={{
                        padding: 12,
                        borderRadius: 5,
                        background: "#222326",
                        border: isActive
                          ? "1px solid #3794ff"
                          : "1px solid var(--vsc-border)",
                        display: "flex",
                        flexDirection: "column",
                        gap: 8,
                      }}
                    >
                      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                        <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                          <strong style={{ fontSize: 13, color: "#ffffff" }}>
                            {agent.name}
                          </strong>
                          <span
                            style={{
                              padding: "1px 6px",
                              borderRadius: 3,
                              fontSize: 10.5,
                              fontWeight: 600,
                              background: agent.installed ? "#163825" : "#332914",
                              border: agent.installed
                                ? "1px solid #3fb950"
                                : "1px solid #8a6d1d",
                              color: agent.installed ? "#7ee787" : "#f5d06f",
                            }}
                          >
                            {agent.installed ? "Installed" : "Not found in PATH"}
                          </span>
                          {isActive && (
                            <span
                              style={{
                                padding: "1px 6px",
                                borderRadius: 3,
                                fontSize: 10.5,
                                fontWeight: 600,
                                background: "#094771",
                                color: "#ffffff",
                              }}
                            >
                              Active
                            </span>
                          )}
                        </div>

                        <div style={{ display: "flex", gap: 6 }}>
                          <button
                            className="vsc-btn secondary small"
                            onClick={() => {
                              if (isEditing) {
                                setEditingAgentId(null);
                              } else {
                                setEditingAgentId(agent.id);
                                setCustomCmd(agent.command);
                                setCustomArgs(agent.args.join(" "));
                              }
                            }}
                          >
                            <span className="codicon codicon-edit" />
                            {isEditing ? "Cancel" : "Configure"}
                          </button>
                          <button
                            className={isActive ? "vsc-btn secondary small" : "vsc-btn small"}
                            disabled={aiBusy || isActive}
                            onClick={() => void handleSelectAiAgent(agent.id)}
                          >
                            {isActive ? "Selected" : "Set Active"}
                          </button>
                        </div>
                      </div>

                      <div style={{ fontSize: 11.5, color: "var(--vsc-text-dim)" }}>
                        {agent.description}
                      </div>

                      <div style={{ fontSize: 11, fontFamily: "var(--font-mono)", color: "#9cdcfe" }}>
                        Command: {agent.command}
                        {agent.args.length > 0 ? ` ${agent.args.join(" ")}` : ""}{" "}
                        {agent.resolved_path ? `→ ${agent.resolved_path}` : ""}
                      </div>

                      {isEditing && (
                        <div
                          style={{
                            display: "flex",
                            gap: 8,
                            alignItems: "center",
                            flexWrap: "wrap",
                            paddingTop: 6,
                            borderTop: "1px solid var(--vsc-border)",
                          }}
                        >
                          <input
                            className="vsc-input mono"
                            style={{ width: 180 }}
                            placeholder="Command (e.g. claude)"
                            value={customCmd}
                            onChange={(e) => setCustomCmd(e.target.value)}
                          />
                          <input
                            className="vsc-input mono"
                            style={{ flex: 1, minWidth: 180 }}
                            placeholder="Optional extra args (space-separated)"
                            value={customArgs}
                            onChange={(e) => setCustomArgs(e.target.value)}
                          />
                          <button
                            className="vsc-btn small"
                            disabled={aiBusy}
                            onClick={() => void handleSaveAgentConfig(agent.id)}
                          >
                            Save Command
                          </button>
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>

              {/* Workspace Context Bridge */}
              <div
                style={{
                  padding: 14,
                  borderRadius: 5,
                  background: "#1c1f24",
                  border: "1px solid var(--vsc-border)",
                  display: "flex",
                  flexDirection: "column",
                  gap: 10,
                }}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                  <div>
                    <div style={{ fontSize: 13, fontWeight: 600, color: "#ffffff" }}>
                      Workspace &amp; `.pstdr` Context Handoff
                    </div>
                    <div style={{ fontSize: 11.5, color: "var(--vsc-text-dim)", marginTop: 2 }}>
                      Workspace: <code>{project?.source_path ?? "None open"}</code> &bull; Active
                      file: <code>{activeFilePath ?? "None"}</code> &bull; Diagnostics:{" "}
                      <strong>{diagnostics.length}</strong>
                    </div>
                  </div>
                  <button
                    className="vsc-btn small"
                    disabled={aiBusy}
                    onClick={() => void handleBuildContextPacket()}
                  >
                    <span className="codicon codicon-copy" />
                    {copiedId === "ai-context"
                      ? "Copied Context Packet!"
                      : "Build & Copy Context Packet"}
                  </button>
                </div>

                {contextPacket && (
                  <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "space-between",
                        gap: 8,
                        padding: "6px 10px",
                        borderRadius: 4,
                        background: "#121417",
                        border: "1px solid var(--vsc-border)",
                        fontFamily: "var(--font-mono)",
                        fontSize: 11,
                        color: "#7ee787",
                      }}
                    >
                      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        $ {contextPacket.suggested_command}
                      </span>
                      <button
                        className="vsc-btn secondary small"
                        onClick={() =>
                          copyToClipboard("ai-cmd", contextPacket.suggested_command)
                        }
                      >
                        {copiedId === "ai-cmd" ? "Copied" : "Copy Cmd"}
                      </button>
                    </div>
                    <pre
                      style={{
                        margin: 0,
                        maxHeight: 220,
                        overflow: "auto",
                        padding: 10,
                        borderRadius: 4,
                        background: "#121417",
                        border: "1px solid var(--vsc-border)",
                        fontFamily: "var(--font-mono)",
                        fontSize: 11,
                        color: "#cccccc",
                        whiteSpace: "pre-wrap",
                      }}
                    >
                      {contextPacket.markdown}
                    </pre>
                  </div>
                )}
              </div>
            </section>
          )}

          {/* ─── SECTION 4: Workbench & File Manager Defaults ─── */}
          {showSection("workbench") && (
            <section style={{ display: "flex", flexDirection: "column", gap: 14 }}>
              <div style={{ borderBottom: "1px solid var(--vsc-border)", paddingBottom: 6 }}>
                <h3 style={{ margin: 0, fontSize: 15, color: "#ffffff" }}>
                  Workbench &amp; File Manager Defaults
                </h3>
                <p style={{ margin: "4px 0 0", fontSize: 12, color: "var(--vsc-text-dim)" }}>
                  Configure asset import sanitization, native file manager integration, and default converter guards.
                </p>
              </div>

              <div
                style={{
                  padding: 14,
                  borderRadius: 5,
                  background: "#222326",
                  border: "1px solid var(--vsc-border)",
                  display: "flex",
                  flexDirection: "column",
                  gap: 12,
                  maxWidth: 520,
                }}
              >
                <label className="field-label">
                  <span>Preferred File Manager</span>
                  <select
                    className="vsc-select"
                    value={studioSettings.preferredFileManager}
                    onChange={(e) =>
                      onStudioSettingsChange({
                        ...studioSettings,
                        preferredFileManager: e.target.value,
                      })
                    }
                  >
                    <option value="auto">
                      Auto ({fileManagers?.primary ?? "detecting..."})
                    </option>
                    {(fileManagers?.available ?? []).map((fm) => (
                      <option key={fm} value={fm}>
                        {fm}
                      </option>
                    ))}
                  </select>
                </label>

                <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
                  <input
                    type="checkbox"
                    checked={studioSettings.autoSanitizeImports}
                    onChange={(e) =>
                      onStudioSettingsChange({
                        ...studioSettings,
                        autoSanitizeImports: e.target.checked,
                      })
                    }
                  />
                  Auto-sanitize imported filenames ([a-z0-9_-])
                </label>

                <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
                  <input
                    type="checkbox"
                    checked={studioSettings.convertImportedImagesToPng}
                    onChange={(e) =>
                      onStudioSettingsChange({
                        ...studioSettings,
                        convertImportedImagesToPng: e.target.checked,
                      })
                    }
                  />
                  Convert JPG/WebP imports to PNG automatically
                </label>

                <label className="field-label">
                  <span>Default 3D UV-Atlas Guard</span>
                  <select
                    className="vsc-select"
                    value={studioSettings.defaultUvAtlasGuard}
                    onChange={(e) =>
                      onStudioSettingsChange({
                        ...studioSettings,
                        defaultUvAtlasGuard: e.target.value as UvAtlasGuardMode,
                      })
                    }
                  >
                    <option value="safe_fallback2d">Safe Fallback 2D</option>
                    <option value="skip">Skip Offending Models</option>
                    <option value="warn">Warn Only</option>
                  </select>
                </label>
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
};
