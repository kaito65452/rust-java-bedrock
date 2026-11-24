import React, { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api } from "../api";
import type { PtyDataEvent, PtyExitEvent } from "../types";

interface TerminalPanelProps {
  cwd: string | null;
  onLog: (level: "info" | "warn" | "error" | "success", msg: string) => void;
}

export const TerminalPanel: React.FC<TerminalPanelProps> = ({ cwd, onLog }) => {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionIdRef = useRef<number | null>(null);
  const [alive, setAlive] = useState(false);

  const spawnSession = async (overrideCwd?: string | null) => {
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
      const sid = await api.ptySpawn(overrideCwd ?? cwd, cols, rows, null);
      sessionIdRef.current = sid;
      setAlive(true);
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
      fontSize: 12,
      lineHeight: 1.2,
      scrollback: 5000,
      theme: {
        background: "#1b1b1d",
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

    void spawnSession(cwd);

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

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div
        style={{
          height: 26,
          padding: "0 9px",
          background: "#18181a",
          borderBottom: "1px solid var(--vsc-border)",
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          fontSize: 10.5,
          flexShrink: 0,
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 7,
            minWidth: 0,
          }}
        >
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: alive ? "var(--vsc-ok)" : "var(--vsc-warn)",
              flexShrink: 0,
            }}
          />
          <span style={{ color: "var(--vsc-text-dim)" }}>Shell</span>
          {cwd && (
            <span
              className="path-hint"
              style={{ maxWidth: 420 }}
              title={cwd}
            >
              {cwd}
            </span>
          )}
        </div>
        <div style={{ display: "flex", gap: 5 }}>
          <button
            className="icon-btn"
            title="Clear terminal"
            onClick={() => termRef.current?.clear()}
          >
            <span className="codicon codicon-clear-all" />
          </button>
          <button
            className="icon-btn"
            title="Restart shell"
            onClick={() => void spawnSession(cwd)}
          >
            <span className="codicon codicon-refresh" />
          </button>
        </div>
      </div>
      <div
        ref={hostRef}
        style={{
          flex: 1,
          minHeight: 0,
          padding: "4px 6px",
          background: "#1b1b1d",
        }}
      />
    </div>
  );
};
