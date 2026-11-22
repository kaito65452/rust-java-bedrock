import React, { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api } from "../api";
import type { PtyDataEvent, PtyExitEvent } from "../types";

interface TerminalPanelProps {
  cwd: string | null;
  launchCommandRequest?: { nonce: number; cmdOpt: string | null; cwd?: string | null };
  yaziAvailable: boolean;
  onLog: (level: "info" | "warn" | "error" | "success", msg: string) => void;
}

export const TerminalPanel: React.FC<TerminalPanelProps> = ({
  cwd,
  launchCommandRequest,
  yaziAvailable,
  onLog,
}) => {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionIdRef = useRef<number | null>(null);
  const [activeCmd, setActiveCmd] = useState<string>("$SHELL");
  const [alive, setAlive] = useState(false);

  const spawnSession = async (
    cmdOpt: string | null,
    overrideCwd?: string | null,
  ) => {
    if (sessionIdRef.current !== null) {
      const oldId = sessionIdRef.current;
      sessionIdRef.current = null;
      await api.ptyKill(oldId).catch(() => {});
    }
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    fit.fit();
    const cols = Math.max(20, term.cols);
    const rows = Math.max(6, term.rows);
    term.reset();
    try {
      const sid = await api.ptySpawn(
        overrideCwd ?? cwd,
        cols,
        rows,
        cmdOpt,
      );
      sessionIdRef.current = sid;
      setAlive(true);
      setActiveCmd(cmdOpt ?? "$SHELL");
      term.focus();
    } catch (e) {
      setAlive(false);
      term.writeln(`\r\n\x1b[31m[PTY Error] ${String(e)}\x1b[0m`);
      onLog("error", `Terminal spawn failed: ${String(e)}`);
    }
  };

  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;

    const term = new Terminal({
      cursorBlink: true,
      fontFamily: "var(--font-mono)",
      fontSize: 13,
      theme: {
        background: "#1e1e1e",
        foreground: "#cccccc",
        cursor: "#ffffff",
        selectionBackground: "#264f78",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;

    const dataDisposable = term.onData((chunk) => {
      if (sessionIdRef.current !== null) {
        void api.ptyWrite(sessionIdRef.current, chunk);
      }
    });

    const unlistenData = listen<PtyDataEvent>("pty://data", (ev) => {
      if (ev.payload.session_id === sessionIdRef.current) {
        term.write(ev.payload.data);
      }
    });

    const unlistenExit = listen<PtyExitEvent>("pty://exit", (ev) => {
      if (ev.payload.session_id === sessionIdRef.current) {
        setAlive(false);
        term.writeln("\r\n\x1b[33m[Process exited]\x1b[0m");
      }
    });

    const ro = new ResizeObserver(() => {
      fit.fit();
      if (sessionIdRef.current !== null) {
        void api.ptyResize(sessionIdRef.current, term.cols, term.rows);
      }
    });
    ro.observe(el);

    void spawnSession(null, cwd);

    return () => {
      ro.disconnect();
      dataDisposable.dispose();
      void unlistenData.then((fn) => fn());
      void unlistenExit.then((fn) => fn());
      if (sessionIdRef.current !== null) {
        void api.ptyKill(sessionIdRef.current);
        sessionIdRef.current = null;
      }
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!launchCommandRequest || launchCommandRequest.nonce === 0) return;
    void spawnSession(
      launchCommandRequest.cmdOpt,
      launchCommandRequest.cwd ?? cwd,
    );
  }, [launchCommandRequest?.nonce]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div
        style={{
          height: 28,
          padding: "0 10px",
          background: "#1e1e1e",
          borderBottom: "1px solid var(--vsc-border)",
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          fontSize: 11,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span
            style={{
              width: 7,
              height: 7,
              borderRadius: "50%",
              background: alive ? "var(--vsc-ok)" : "var(--vsc-warn)",
            }}
          />
          <span>PTY Session: {activeCmd}</span>
          {cwd && (
            <span style={{ color: "var(--vsc-text-dim)", fontFamily: "var(--font-mono)" }}>
              ({cwd})
            </span>
          )}
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <button
            className="vsc-btn secondary small"
            onClick={() => void spawnSession("yazi", cwd)}
            title={
              yaziAvailable
                ? "Launch Yazi TUI File Manager inside PTY"
                : "Attempt launching yazi in PTY"
            }
          >
            <span className="codicon codicon-folder" />
            Launch Yazi
          </button>
          <button
            className="vsc-btn secondary small"
            onClick={() => void spawnSession(null, cwd)}
          >
            <span className="codicon codicon-refresh" />
            New Shell
          </button>
        </div>
      </div>
      <div
        ref={hostRef}
        style={{ flex: 1, minHeight: 0, padding: "4px 6px", background: "#1e1e1e" }}
      />
    </div>
  );
};
