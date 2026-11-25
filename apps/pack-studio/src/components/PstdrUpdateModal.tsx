import React, { useEffect, useState } from "react";
import { api } from "../api";
import type {
  OutputLogLine,
  PstdrSyncStatusDto,
  PstdrUpdateOfferDto,
  QuarantineVerificationReportDto,
} from "../types";
import { formatBytes } from "../utils/mcFormat";

interface PstdrUpdateModalProps {
  status: PstdrSyncStatusDto | null;
  initialOffer: PstdrUpdateOfferDto | null;
  onClose: () => void;
  onStatusChanged: (next: PstdrSyncStatusDto) => void;
  onOfferChanged: (next: PstdrUpdateOfferDto | null) => void;
  onLog: (level: OutputLogLine["level"], message: string) => void;
}

export const PstdrUpdateModal: React.FC<PstdrUpdateModalProps> = ({
  status,
  initialOffer,
  onClose,
  onStatusChanged,
  onOfferChanged,
  onLog,
}) => {
  const [offer, setOffer] = useState<PstdrUpdateOfferDto | null>(initialOffer);
  const [quarantine, setQuarantine] =
    useState<QuarantineVerificationReportDto | null>(null);
  const [busyAction, setBusyAction] = useState<
    null | "check" | "download" | "apply" | "reject" | "revert"
  >(null);
  const [securityError, setSecurityError] = useState<string | null>(null);
  const [infoBanner, setInfoBanner] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busyAction) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busyAction, onClose]);

  const copyText = (key: string, value: string) => {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      void navigator.clipboard.writeText(value).catch(() => {});
    }
    setCopiedKey(key);
    window.setTimeout(() => {
      setCopiedKey((cur) => (cur === key ? null : cur));
    }, 1800);
  };

  const handleCheck = async () => {
    setBusyAction("check");
    setSecurityError(null);
    setInfoBanner(null);
    setQuarantine(null);
    try {
      const next = await api.pstdrSyncCheck();
      setOffer(next);
      onOfferChanged(next.update_available ? next : null);
      if (next.update_available) {
        onLog(
          "info",
          `Verified PSTDR update available: v${next.version} (${next.spec_version})`,
        );
      } else {
        setInfoBanner(
          `Already up to date: installed v${next.current_version} matches or exceeds latest signed release v${next.version}.`,
        );
        onLog(
          "info",
          `PSTDR is up to date (installed v${next.current_version}, latest v${next.version}).`,
        );
      }
    } catch (e) {
      const msg = String(e);
      setSecurityError(msg);
      onLog("error", `PSTDR update check blocked: ${msg}`);
    } finally {
      setBusyAction(null);
    }
  };

  const handleDownloadQuarantine = async () => {
    if (!offer) return;
    setBusyAction("download");
    setSecurityError(null);
    setInfoBanner(null);
    try {
      const report = await api.pstdrSyncDownloadQuarantine(offer.version);
      setQuarantine(report);
      onLog(
        "success",
        `Quarantined & SHA-256 verified PSTDR v${report.version} (${report.artifacts.length} artifact(s)). Awaiting manual approval.`,
      );
    } catch (e) {
      const msg = String(e);
      setQuarantine(null);
      setSecurityError(msg);
      onLog("error", `PSTDR quarantine verification failed: ${msg}`);
    } finally {
      setBusyAction(null);
    }
  };

  const handleApproveAndApply = async () => {
    if (!quarantine) return;
    setBusyAction("apply");
    setSecurityError(null);
    setInfoBanner(null);
    try {
      const nextStatus = await api.pstdrSyncApproveAndApply(quarantine.version);
      onStatusChanged(nextStatus);
      onOfferChanged(null);
      setOffer(null);
      setQuarantine(null);
      setInfoBanner(
        `Applied verified PSTDR v${nextStatus.active_version} (${nextStatus.active_spec_version}).`,
      );
      onLog(
        "success",
        `Applied verified PSTDR runtime v${nextStatus.active_version} (${nextStatus.active_spec_version}).`,
      );
    } catch (e) {
      const msg = String(e);
      setSecurityError(msg);
      onLog("error", `PSTDR update apply failed: ${msg}`);
    } finally {
      setBusyAction(null);
    }
  };

  const handleRejectQuarantine = async () => {
    const ver = quarantine?.version ?? offer?.version;
    if (!ver) return;
    setBusyAction("reject");
    setSecurityError(null);
    try {
      await api.pstdrSyncRejectQuarantine(ver);
      setQuarantine(null);
      setInfoBanner(`Rejected and deleted quarantined files for PSTDR v${ver}.`);
      onLog("info", `Rejected and purged PSTDR v${ver} quarantine.`);
    } catch (e) {
      setSecurityError(String(e));
    } finally {
      setBusyAction(null);
    }
  };

  const handleRevertBuiltin = async () => {
    setBusyAction("revert");
    setSecurityError(null);
    setInfoBanner(null);
    try {
      const nextStatus = await api.pstdrSyncRevertBuiltin();
      onStatusChanged(nextStatus);
      setQuarantine(null);
      setInfoBanner(
        `Reverted to built-in compiled PSTDR engine (v${nextStatus.builtin_version}).`,
      );
      onLog(
        "info",
        `Reverted PSTDR runtime to built-in v${nextStatus.builtin_version}.`,
      );
    } catch (e) {
      const msg = String(e);
      setSecurityError(msg);
      onLog("error", `Failed reverting PSTDR runtime: ${msg}`);
    } finally {
      setBusyAction(null);
    }
  };

  const activeFingerprint =
    quarantine?.pubkey_fingerprint ??
    offer?.pubkey_fingerprint ??
    status?.pubkey_fingerprint ??
    "UNCONFIGURED";

  return (
    <div
      className="modal-backdrop"
      onClick={() => {
        if (!busyAction) onClose();
      }}
    >
      <div
        className="modal-dialog"
        style={{ width: 640, maxWidth: "94vw", maxHeight: "88vh", display: "flex", flexDirection: "column" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sidebar-header">
          <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span className="codicon codicon-shield" style={{ color: "#73c991" }} />
            <span>PSTDR Verified Update Sync (Ed25519 + SHA-256)</span>
          </span>
          <button
            className="icon-btn"
            disabled={Boolean(busyAction)}
            onClick={onClose}
            title="Close"
          >
            <span className="codicon codicon-close" />
          </button>
        </div>

        <div
          style={{
            padding: 14,
            display: "flex",
            flexDirection: "column",
            gap: 12,
            overflow: "auto",
          }}
        >
          {/* Active Runtime Summary */}
          <div
            style={{
              padding: "10px 12px",
              borderRadius: 4,
              background: "var(--vsc-bg-panel)",
              border: "1px solid var(--vsc-border)",
              display: "flex",
              flexDirection: "column",
              gap: 6,
            }}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
              <span style={{ fontSize: 12, color: "var(--vsc-text-bright)", fontWeight: 600 }}>
                Installed PSTDR Runtime:{" "}
                <span style={{ fontFamily: "var(--font-mono)", color: "#9cdcfe" }}>
                  v{status?.active_version ?? "0.1.0-pre-beta.1"}
                </span>{" "}
                <span className="edition-badge">
                  {status?.external_runtime_active ? "SYNCED" : "BUILT-IN"}
                </span>
              </span>
              <div style={{ display: "flex", gap: 6 }}>
                {status?.external_runtime_active && (
                  <button
                    className="vsc-btn secondary small"
                    disabled={Boolean(busyAction)}
                    onClick={() => void handleRevertBuiltin()}
                  >
                    <span className="codicon codicon-discard" />
                    {busyAction === "revert" ? "Reverting..." : "Revert to Built-in PSTDR"}
                  </button>
                )}
                <button
                  className="vsc-btn small"
                  disabled={Boolean(busyAction)}
                  onClick={() => void handleCheck()}
                >
                  <span className="codicon codicon-cloud-download" />
                  {busyAction === "check" ? "Checking..." : "Check PSTDR Updates"}
                </button>
              </div>
            </div>
            <div style={{ fontSize: 11, color: "var(--vsc-text-dim)" }}>
              Spec: <strong>{status?.active_spec_version ?? "Bundled"}</strong> &bull; Key ID:{" "}
              <span style={{ fontFamily: "var(--font-mono)" }}>
                {status?.key_id ?? "saryu-pstdr-root-2026"}
              </span>
            </div>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 8,
                fontSize: 11,
                color: "var(--vsc-text-dim)",
              }}
            >
              <span
                style={{
                  fontFamily: "var(--font-mono)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
                title={activeFingerprint}
              >
                Pubkey Fingerprint: {activeFingerprint}
              </span>
              {status?.pubkey_configured && (
                <button
                  className="vsc-btn secondary small"
                  onClick={() => copyText("pubkey", activeFingerprint)}
                  title="Copy public key SHA-256 fingerprint"
                >
                  <span className="codicon codicon-copy" />
                  {copiedKey === "pubkey" ? "Copied" : "Copy"}
                </button>
              )}
            </div>
          </div>

          {status && !status.pubkey_configured && (
            <div
              style={{
                padding: "10px 12px",
                borderRadius: 4,
                background: "#332914",
                border: "1px solid #8a6d1d",
                color: "#f5d06f",
                fontSize: 12,
              }}
            >
              <strong>Hardcoded Signing Key Not Configured:</strong> Paste your 64-character
              Ed25519 public key hex into <code>PSTDR_SIGNING_PUBKEY_HEX</code> in{" "}
              <code>pstdr_sync.rs</code> before enabling signed release sync.
            </div>
          )}

          {/* Red Security Warning Banner */}
          {securityError && (
            <div
              style={{
                padding: "10px 12px",
                borderRadius: 4,
                background: "#3a1618",
                border: "1px solid #b53d43",
                color: "#ffb4b8",
                fontSize: 12,
                display: "flex",
                flexDirection: "column",
                gap: 4,
              }}
            >
              <strong style={{ display: "flex", alignItems: "center", gap: 6, color: "#ff7a82" }}>
                <span className="codicon codicon-error" />
                Security Verification Blocked Update
              </strong>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, wordBreak: "break-word" }}>
                {securityError}
              </span>
            </div>
          )}

          {infoBanner && (
            <div
              style={{
                padding: "8px 12px",
                borderRadius: 4,
                background: "#183326",
                border: "1px solid #2e7d53",
                color: "#9ae6b4",
                fontSize: 12,
              }}
            >
              {infoBanner}
            </div>
          )}

          {/* Verified Release Offer Details */}
          {offer && offer.update_available && (
            <div
              style={{
                padding: 12,
                borderRadius: 4,
                background: "#191c1f",
                border: "1px solid var(--vsc-border)",
                display: "flex",
                flexDirection: "column",
                gap: 10,
              }}
            >
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: "#ffffff" }}>
                  Update Ready:{" "}
                  <span style={{ fontFamily: "var(--font-mono)", color: "#9cdcfe" }}>
                    v{offer.current_version} &rarr; v{offer.version}
                  </span>
                </div>
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 5,
                    padding: "3px 8px",
                    borderRadius: 4,
                    background: "#163825",
                    border: "1px solid #3fb950",
                    color: "#7ee787",
                    fontSize: 11,
                    fontWeight: 700,
                  }}
                >
                  <span className="codicon codicon-verified-filled" />
                  Ed25519 Signature: VERIFIED
                </span>
              </div>

              <div style={{ fontSize: 11, color: "var(--vsc-text-dim)" }}>
                Spec Version: <strong>{offer.spec_version}</strong> &bull; Released:{" "}
                <span style={{ fontFamily: "var(--font-mono)" }}>{offer.released_at}</span>
              </div>

              {offer.changelog.length > 0 && (
                <div>
                  <div style={{ fontSize: 11, fontWeight: 600, color: "#cccccc", marginBottom: 4 }}>
                    Changelog
                  </div>
                  <ul
                    style={{
                      margin: 0,
                      paddingLeft: 18,
                      fontSize: 12,
                      color: "#d4d4d4",
                      lineHeight: 1.5,
                    }}
                  >
                    {offer.changelog.map((line, i) => (
                      <li key={i}>{line}</li>
                    ))}
                  </ul>
                </div>
              )}

              {/* Artifact SHA-256 Fingerprints */}
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <div style={{ fontSize: 11, fontWeight: 600, color: "#cccccc" }}>
                  Signed Artifacts &amp; SHA-256 Fingerprints
                </div>
                {offer.artifacts.map((art) => {
                  const verified = quarantine?.artifacts.find((q) => q.name === art.name);
                  const hashToShow = verified?.computed_sha256 ?? art.expected_sha256;
                  return (
                    <div
                      key={art.name}
                      style={{
                        padding: "8px 10px",
                        borderRadius: 4,
                        background: "#121417",
                        border: verified
                          ? "1px solid #2e7d53"
                          : "1px solid var(--vsc-border)",
                        display: "flex",
                        flexDirection: "column",
                        gap: 4,
                      }}
                    >
                      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
                        <span style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "#e6edf3" }}>
                          {art.name}{" "}
                          <span style={{ color: "var(--vsc-text-dim)" }}>
                            ({formatBytes(verified?.size_bytes ?? art.size)})
                          </span>
                        </span>
                        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                          {verified?.sha256_verified && (
                            <span style={{ color: "#7ee787", fontSize: 11, fontWeight: 600 }}>
                              <span className="codicon codicon-check" /> SHA-256 Verified (Quarantined)
                            </span>
                          )}
                          <button
                            className="vsc-btn secondary small"
                            onClick={() => copyText(art.name, hashToShow)}
                          >
                            <span className="codicon codicon-copy" />
                            {copiedKey === art.name ? "Copied" : "Copy SHA-256"}
                          </button>
                        </div>
                      </div>
                      <div
                        style={{
                          fontFamily: "var(--font-mono)",
                          fontSize: 11,
                          color: "#9cdcfe",
                          wordBreak: "break-all",
                        }}
                      >
                        {hashToShow}
                      </div>
                    </div>
                  );
                })}
              </div>

              {/* Manual 2-Step Approval Controls */}
              <div
                style={{
                  display: "flex",
                  justifyContent: "flex-end",
                  alignItems: "center",
                  gap: 8,
                  paddingTop: 6,
                  borderTop: "1px solid var(--vsc-border)",
                }}
              >
                {!quarantine ? (
                  <button
                    className="vsc-btn"
                    disabled={Boolean(busyAction)}
                    onClick={() => void handleDownloadQuarantine()}
                  >
                    <span className="codicon codicon-shield" />
                    {busyAction === "download"
                      ? "Downloading & Verifying SHA-256..."
                      : "Download & Verify SHA-256"}
                  </button>
                ) : (
                  <>
                    <button
                      className="vsc-btn secondary"
                      disabled={Boolean(busyAction)}
                      onClick={() => void handleRejectQuarantine()}
                    >
                      <span className="codicon codicon-trash" />
                      {busyAction === "reject" ? "Deleting..." : "Reject & Delete"}
                    </button>
                    <button
                      className="vsc-btn"
                      disabled={Boolean(busyAction) || !quarantine.all_hashes_verified}
                      onClick={() => void handleApproveAndApply()}
                    >
                      <span className="codicon codicon-check-all" />
                      {busyAction === "apply" ? "Applying..." : "Approve & Apply Update"}
                    </button>
                  </>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
