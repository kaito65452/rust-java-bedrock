import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { Allotment } from "allotment";
import { api } from "./api";
import { ConverterTab } from "./components/ConverterTab";
import { ExplorerSidebar } from "./components/ExplorerSidebar";
import { MetadataPackagingTab } from "./components/MetadataPackagingTab";
import { MonacoCodeTab } from "./components/MonacoCodeTab";
import { NewProjectWizardModal } from "./components/NewProjectWizardModal";
import { TerminalPanel } from "./components/TerminalPanel";
import { TexturePaintTab } from "./components/TexturePaintTab";
import type {
  ActivityView,
  BinaryAssetData,
  BlockbenchSavedEvent,
  BottomTab,
  ConversionWarning,
  DetectedFileManagers,
  EditorTab,
  NewProjectConfig,
  OutputLogLine,
  ProjectSnapshot,
  StudioSettings,
  UvAtlasGuardMode,
} from "./types";
import {
  baseName,
  computeResourceId,
  fileIconClass,
  isImageAsset,
  isModelAsset,
  isTextCodeAsset,
} from "./utils/mcFormat";

const DEFAULT_SETTINGS: StudioSettings = {
  preferredFileManager: "auto",
  useYaziPicker: true,
  autoSanitizeImports: true,
  convertImportedImagesToPng: true,
  defaultTargetResolution: null,
  defaultUvAtlasGuard: "safe_fallback2d",
  defaultCompositeGrassTga: true,
  defaultConvertPbr: false,
  externalImageEditor: "",
};

function loadInitialSettings(): StudioSettings {
  try {
    const raw = localStorage.getItem("pack_studio_settings_v1");
    if (raw) {
      return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
    }
  } catch {
    // ignore storage errors
  }
  return DEFAULT_SETTINGS;
}

export default function App() {
  const [project, setProject] = useState<ProjectSnapshot | null>(null);
  const [activity, setActivity] = useState<ActivityView>("explorer");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [bottomOpen, setBottomOpen] = useState(true);
  const [bottomTab, setBottomTab] = useState<BottomTab>("output");

  const [tabs, setTabs] = useState<EditorTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [selectedRelPath, setSelectedRelPath] = useState<string | null>(null);
  const [explorerFilter, setExplorerFilter] = useState("");
  const [searchQuery, setSearchQuery] = useState("");

  const [textCache, setTextCache] = useState<Record<string, string>>({});
  const [binaryCache, setBinaryCache] = useState<
    Record<string, BinaryAssetData>
  >({});
  const [problems, setProblems] = useState<ConversionWarning[]>([]);
  const [logs, setLogs] = useState<OutputLogLine[]>([]);
  const logSeqRef = useRef(1);

  const [settings, setSettings] = useState<StudioSettings>(loadInitialSettings);
  const [fileManagers, setFileManagers] =
    useState<DetectedFileManagers | null>(null);

  const [wizardOpen, setWizardOpen] = useState(false);
  const [paletteMode, setPaletteMode] = useState<null | "commands" | "files">(
    null,
  );
  const [paletteQuery, setPaletteQuery] = useState("");
  const [dragOver, setDragOver] = useState(false);
  const [toastMsg, setToastMsg] = useState<string | null>(null);

  const [blankTexModal, setBlankTexModal] = useState<null | {
    dir: string;
    name: string;
    size: number;
  }>(null);

  const [ptyLaunchReq, setPtyLaunchReq] = useState<{
    nonce: number;
    cmdOpt: string | null;
    cwd?: string | null;
  }>({ nonce: 0, cmdOpt: null, cwd: null });

  const projectRef = useRef<ProjectSnapshot | null>(null);
  projectRef.current = project;

  const appendLog = useCallback(
    (level: OutputLogLine["level"], message: string) => {
      const now = new Date();
      const ts = now.toTimeString().slice(0, 8);
      const id = logSeqRef.current;
      logSeqRef.current += 1;
      setLogs((prev) => [
        ...prev.slice(-300),
        { id, timestamp: ts, level, message },
      ]);
    },
    [],
  );

  const showToast = useCallback((msg: string) => {
    setToastMsg(msg);
    setTimeout(() => {
      setToastMsg((cur) => (cur === msg ? null : cur));
    }, 2800);
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem("pack_studio_settings_v1", JSON.stringify(settings));
    } catch {
      // ignore storage errors
    }
  }, [settings]);

  const refreshFileManagers = useCallback(async () => {
    try {
      const fm = await api.detectFileManagers();
      setFileManagers(fm);
    } catch (e) {
      appendLog("warn", `File manager detection error: ${String(e)}`);
    }
  }, [appendLog]);

  useEffect(() => {
    appendLog(
      "info",
      "PackStudio Tauri v2 Workbench ready. Use Ctrl+Shift+P for Command Palette.",
    );
    void refreshFileManagers();
  }, [appendLog, refreshFileManagers]);

  const refreshActiveProject = useCallback(async () => {
    const src = projectRef.current?.source_path;
    if (!src) return;
    try {
      const snap = await api.openProject(src);
      setProject(snap);
    } catch (e) {
      appendLog("error", `Failed reloading project: ${String(e)}`);
    }
  }, [appendLog]);

  useEffect(() => {
    const unlistenFs = listen<string[]>("fs://changed", (ev) => {
      appendLog("info", `Filesystem changed: ${ev.payload.join(", ")}`);
      void refreshActiveProject();
    });
    const unlistenBb = listen<BlockbenchSavedEvent>(
      "blockbench://saved",
      (ev) => {
        appendLog(
          "success",
          `Blockbench synced ${ev.payload.saved_files.length} file(s): ${ev.payload.saved_files.join(", ")}`,
        );
        showToast(
          `Blockbench saved: ${ev.payload.saved_files.map(baseName).join(", ")}`,
        );
        void refreshActiveProject();
      },
    );
    return () => {
      void unlistenFs.then((f) => f());
      void unlistenBb.then((f) => f());
    };
  }, [appendLog, refreshActiveProject, showToast]);

  const handleImportPaths = useCallback(
    async (targetDir: string, diskPaths: string[]) => {
      if (!projectRef.current || diskPaths.length === 0) return;
      try {
        const res = await api.fsImportAssets(
          targetDir,
          diskPaths,
          settings.autoSanitizeImports,
          settings.convertImportedImagesToPng,
          true,
        );
        setProject(res.snapshot);
        appendLog(
          "success",
          `Imported ${res.imported_paths.length} asset(s) into '${targetDir || "/"}'`,
        );
        if (res.skipped_paths.length > 0) {
          appendLog("warn", `Skipped: ${res.skipped_paths.join(", ")}`);
        }
      } catch (e) {
        appendLog("error", `Import failed: ${String(e)}`);
      }
    },
    [
      appendLog,
      settings.autoSanitizeImports,
      settings.convertImportedImagesToPng,
    ],
  );

  useEffect(() => {
    let unlistenDrop: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((ev) => {
        const payload = ev.payload as { type: string; paths?: string[] };
        if (payload.type === "enter" || payload.type === "over") {
          setDragOver(true);
        } else if (payload.type === "leave") {
          setDragOver(false);
        } else if (payload.type === "drop") {
          setDragOver(false);
          if (payload.paths && payload.paths.length > 0 && projectRef.current) {
            const cur = selectedRelPath ?? "";
            const isFile =
              cur && projectRef.current.all_files.includes(cur);
            const targetDir = isFile
              ? cur.slice(0, Math.max(0, cur.lastIndexOf("/")))
              : cur;
            void handleImportPaths(targetDir, payload.paths);
          }
        }
      })
      .then((fn) => {
        unlistenDrop = fn;
      })
      .catch(() => {});

    return () => {
      if (unlistenDrop) unlistenDrop();
    };
  }, [handleImportPaths, selectedRelPath]);

  const openOrFocusTab = useCallback(
    (tab: EditorTab) => {
      setTabs((prev) => {
        const exists = prev.find((t) => t.id === tab.id);
        if (exists) {
          return prev.map((t) =>
            t.id === tab.id
              ? {
                  ...t,
                  revealLine: tab.revealLine ?? t.revealLine,
                  blockbenchUrl: tab.blockbenchUrl ?? t.blockbenchUrl,
                }
              : t,
          );
        }
        return [...prev, tab];
      });
      setActiveTabId(tab.id);
    },
    [],
  );

  const openAssetTab = useCallback(
    async (
      relPath: string,
      forceMode?: "code" | "texture",
      revealLine?: number,
    ) => {
      if (!projectRef.current) return;
      const mode: "code" | "texture" =
        forceMode ??
        (isImageAsset(relPath)
          ? "texture"
          : isTextCodeAsset(relPath) || isModelAsset(relPath)
            ? "code"
            : "code");

      try {
        if (mode === "texture") {
          const bin = await api.fsReadBinaryBase64(relPath);
          setBinaryCache((prev) => ({ ...prev, [relPath]: bin }));
          openOrFocusTab({
            id: `tex:${relPath}`,
            kind: "texture",
            title: baseName(relPath),
            relPath,
          });
        } else {
          const text = await api.fsReadText(relPath);
          setTextCache((prev) => ({ ...prev, [relPath]: text }));
          openOrFocusTab({
            id: `code:${relPath}`,
            kind: "code",
            title: baseName(relPath),
            relPath,
            revealLine,
          });
        }
      } catch (e) {
        appendLog("error", `Failed opening '${relPath}': ${String(e)}`);
      }
    },
    [appendLog, openOrFocusTab],
  );

  const openProjectByPath = useCallback(
    async (path: string) => {
      try {
        const snap = await api.openProject(path);
        setProject(snap);
        setTabs([]);
        setActiveTabId(null);
        setTextCache({});
        setBinaryCache({});
        setProblems([]);
        setSelectedRelPath(null);
        appendLog(
          "success",
          `Opened ${snap.edition_label}: '${snap.pack_name}' (${snap.file_count} files, ${snap.texture_count} textures)`,
        );
        const metaFile = snap.all_files.find(
          (f) => f === "pack.mcmeta" || f === "manifest.json",
        );
        if (metaFile) {
          void openAssetTab(metaFile, "code");
        }
      } catch (e) {
        appendLog("error", `Open project failed: ${String(e)}`);
      }
    },
    [appendLog, openAssetTab],
  );

  const promptOpenProject = useCallback(async () => {
    const picked = await dialogOpen({
      multiple: false,
      directory: true,
    });
    if (typeof picked === "string") {
      await openProjectByPath(picked);
    }
  }, [openProjectByPath]);

  const promptOpenArchive = useCallback(async () => {
    const picked = await dialogOpen({
      multiple: false,
      directory: false,
      filters: [{ name: "Resource Packs", extensions: ["zip", "mcpack"] }],
    });
    if (typeof picked === "string") {
      await openProjectByPath(picked);
    }
  }, [openProjectByPath]);

  const handlePickWithYazi = useCallback(
    async (openAsProjectIfSingle = false, targetImportDir = "") => {
      try {
        const startDir = projectRef.current?.source_path ?? null;
        const picked = await api.pickWithYazi(startDir);
        if (picked.length === 0) {
          appendLog("info", "Yazi selection cancelled.");
          return;
        }
        if (openAsProjectIfSingle || !projectRef.current) {
          await openProjectByPath(picked[0]);
        } else {
          await handleImportPaths(targetImportDir, picked);
        }
      } catch (e) {
        appendLog("warn", String(e));
        setBottomOpen(true);
        setBottomTab("terminal");
        setPtyLaunchReq((r) => ({
          nonce: r.nonce + 1,
          cmdOpt: "yazi",
          cwd: projectRef.current?.source_path ?? null,
        }));
      }
    },
    [appendLog, handleImportPaths, openProjectByPath],
  );

  const handleOpenBlockbench = useCallback(
    async (relPath?: string) => {
      try {
        const info = relPath
          ? await api.blockbenchOpenAsset(relPath)
          : await api.blockbenchStart();
        openOrFocusTab({
          id: "tool:blockbench",
          kind: "blockbench",
          title: "Blockbench 3D",
          blockbenchUrl: info.url,
        });
        appendLog(
          "info",
          `Blockbench web bridge listening on loopback port ${info.port}${relPath ? ` (asset: ${relPath})` : ""}`,
        );
      } catch (e) {
        appendLog("error", `Blockbench launch error: ${String(e)}`);
      }
    },
    [appendLog, openOrFocusTab],
  );

  const handleRunAudit = useCallback(async () => {
    if (!projectRef.current) return;
    appendLog("info", "Running Semantic Auditor...");
    try {
      const audit = await api.runAudit(null);
      setProblems(audit.problems);
      setBottomOpen(true);
      setBottomTab("problems");
      appendLog(
        audit.status === "failed" ? "error" : "success",
        `Semantic Audit ${audit.status.toUpperCase()} (${audit.elapsed_ms}ms): ${audit.summary}`,
      );
    } catch (e) {
      appendLog("error", `Semantic Audit error: ${String(e)}`);
    }
  }, [appendLog]);

  const handleReveal = useCallback(
    async (relOrAbsPath: string) => {
      const override =
        settings.preferredFileManager === "auto"
          ? null
          : settings.preferredFileManager;
      try {
        await api.revealInFileManager(relOrAbsPath, override);
      } catch (e) {
        appendLog("error", `Reveal failed: ${String(e)}`);
      }
    },
    [appendLog, settings.preferredFileManager],
  );

  const handleLaunchYaziInTerminal = useCallback(
    (relPath?: string) => {
      const base = projectRef.current?.source_path ?? null;
      const resolvedCwd =
        base && relPath ? `${base}/${relPath}`.replace(/\/+/g, "/") : base;
      setBottomOpen(true);
      setBottomTab("terminal");
      setPtyLaunchReq((r) => ({
        nonce: r.nonce + 1,
        cmdOpt: "yazi",
        cwd: resolvedCwd,
      }));
    },
    [],
  );

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const ctrl = e.ctrlKey || e.metaKey;
      if (e.key === "F1" || (ctrl && e.shiftKey && e.key.toLowerCase() === "p")) {
        e.preventDefault();
        setPaletteQuery("");
        setPaletteMode("commands");
      } else if (ctrl && !e.shiftKey && e.key.toLowerCase() === "p") {
        e.preventDefault();
        setPaletteQuery("");
        setPaletteMode("files");
      } else if (ctrl && e.key.toLowerCase() === "b") {
        e.preventDefault();
        setSidebarOpen((s) => !s);
      } else if (ctrl && (e.key.toLowerCase() === "j" || e.key === "`")) {
        e.preventDefault();
        setBottomOpen((s) => !s);
      } else if (ctrl && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        setActivity("explorer");
        setSidebarOpen(true);
      } else if (ctrl && e.shiftKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setActivity("search");
        setSidebarOpen(true);
      } else if (e.key === "Escape") {
        setPaletteMode(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const searchResults = useMemo(() => {
    if (!project) return [];
    const q = searchQuery.trim().toLowerCase();
    if (!q) return project.all_files.slice(0, 80);
    return project.all_files
      .filter((f) => {
        const rid = computeResourceId(f, project.edition).toLowerCase();
        return f.toLowerCase().includes(q) || rid.includes(q);
      })
      .slice(0, 120);
  }, [project, searchQuery]);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;

  const commandActions = useMemo(
    () => [
      {
        label: "File: Open Resource Pack Folder...",
        action: () => void promptOpenProject(),
      },
      {
        label: "File: Open Archive (.zip / .mcpack)...",
        action: () => void promptOpenArchive(),
      },
      {
        label: "File: New Resource Pack / Add-On Wizard...",
        action: () => setWizardOpen(true),
      },
      {
        label: "Yazi: Pick Resource Pack or Import Assets",
        action: () => void handlePickWithYazi(false),
      },
      {
        label: "Converter: Open Converter & Preflight Studio",
        action: () =>
          openOrFocusTab({
            id: "tool:converter",
            kind: "converter",
            title: "Converter & Preflight",
          }),
      },
      {
        label: "Metadata: Open Pack Manifest & Export Studio",
        action: () =>
          openOrFocusTab({
            id: "tool:metadata",
            kind: "metadata",
            title: "Metadata & Packaging",
          }),
      },
      {
        label: "Blockbench: Launch Embedded 3D Studio",
        action: () => void handleOpenBlockbench(),
      },
      {
        label: "Audit: Run Semantic Auditor",
        action: () => void handleRunAudit(),
      },
      {
        label: "View: Toggle Primary Sidebar (Ctrl+B)",
        action: () => setSidebarOpen((s) => !s),
      },
      {
        label: "View: Toggle Bottom Panel / Terminal (Ctrl+J)",
        action: () => setBottomOpen((s) => !s),
      },
    ],
    [
      handleOpenBlockbench,
      handlePickWithYazi,
      handleRunAudit,
      openOrFocusTab,
      promptOpenArchive,
      promptOpenProject,
    ],
  );

  const filteredCommands = commandActions.filter((c) =>
    c.label.toLowerCase().includes(paletteQuery.trim().toLowerCase()),
  );
  const filteredQuickFiles = (project?.all_files ?? [])
    .filter((f) =>
      f.toLowerCase().includes(paletteQuery.trim().toLowerCase()),
    )
    .slice(0, 60);

  const errorCount = problems.filter((p) => p.severity === "error").length;
  const warnCount = problems.filter((p) => p.severity === "warning").length;

  const setTabDirty = (tabId: string, dirty: boolean) => {
    setTabs((prev) =>
      prev.map((t) => (t.id === tabId && t.dirty !== dirty ? { ...t, dirty } : t)),
    );
  };

  const closeTab = (tabId: string) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== tabId);
      if (activeTabId === tabId) {
        setActiveTabId(next.length > 0 ? next[next.length - 1].id : null);
      }
      return next;
    });
  };

  return (
    <div className={`workbench-root ${dragOver ? "drop-active" : ""}`}>
      {/* VS Code Title / Command Bar */}
      <div className="title-command-bar">
        <div className="title-left">
          <span className="brand-pill">
            <span className="codicon codicon-layers" style={{ color: "#3794ff" }} />
            PackStudio
          </span>
          {project && (
            <>
              <span style={{ fontSize: 12, color: "#cccccc" }}>
                {project.pack_name}
              </span>
              <span className="edition-badge">{project.edition_label}</span>
              {project.companion_bp_dir && (
                <span className="edition-badge" style={{ color: "#73c991" }}>
                  RP + BP
                </span>
              )}
            </>
          )}
        </div>

        <button
          className="command-trigger-btn"
          onClick={() => {
            setPaletteQuery("");
            setPaletteMode(project ? "files" : "commands");
          }}
        >
          <span className="codicon codicon-search" />
          <span>
            {project
              ? `${project.pack_name} — Ctrl+P Files | Ctrl+Shift+P Commands`
              : "Ctrl+Shift+P Command Palette"}
          </span>
        </button>

        <div className="title-actions">
          <button
            className="vsc-btn secondary small"
            onClick={() => void promptOpenProject()}
          >
            <span className="codicon codicon-folder-opened" />
            Open
          </button>
          <button
            className="vsc-btn secondary small"
            onClick={() => setWizardOpen(true)}
          >
            <span className="codicon codicon-add" />
            New
          </button>
          <button
            className="vsc-btn secondary small"
            disabled={!project}
            onClick={() => void handleRunAudit()}
          >
            <span className="codicon codicon-shield" />
            Audit
          </button>
          <button
            className="vsc-btn secondary small"
            onClick={() => void handleOpenBlockbench()}
          >
            <span className="codicon codicon-cube" />
            Blockbench
          </button>
          <button
            className="icon-btn"
            title="Toggle Sidebar (Ctrl+B)"
            onClick={() => setSidebarOpen((s) => !s)}
          >
            <span className="codicon codicon-layout-sidebar-left" />
          </button>
          <button
            className="icon-btn"
            title="Toggle Panel (Ctrl+J)"
            onClick={() => setBottomOpen((s) => !s)}
          >
            <span className="codicon codicon-layout-panel" />
          </button>
        </div>
      </div>

      {/* Main Workbench Area */}
      <div className="workbench-body">
        {/* Activity Bar */}
        <div className="activity-bar">
          <div className="activity-group">
            <button
              className={`activity-btn ${activity === "explorer" && sidebarOpen ? "active" : ""}`}
              title="Explorer (Ctrl+Shift+E)"
              onClick={() => {
                if (activity === "explorer" && sidebarOpen) setSidebarOpen(false);
                else {
                  setActivity("explorer");
                  setSidebarOpen(true);
                }
              }}
            >
              <span className="codicon codicon-files" />
              {problems.length > 0 && (
                <span className="activity-badge">{problems.length}</span>
              )}
            </button>

            <button
              className={`activity-btn ${activity === "search" && sidebarOpen ? "active" : ""}`}
              title="Search & Resource IDs (Ctrl+Shift+F)"
              onClick={() => {
                if (activity === "search" && sidebarOpen) setSidebarOpen(false);
                else {
                  setActivity("search");
                  setSidebarOpen(true);
                }
              }}
            >
              <span className="codicon codicon-search" />
            </button>

            <button
              className={`activity-btn ${activity === "converter" && sidebarOpen ? "active" : ""}`}
              title="Java ↔ Bedrock Converter"
              onClick={() => {
                setActivity("converter");
                setSidebarOpen(true);
                openOrFocusTab({
                  id: "tool:converter",
                  kind: "converter",
                  title: "Converter & Preflight",
                });
              }}
            >
              <span className="codicon codicon-arrow-swap" />
            </button>

            <button
              className={`activity-btn ${activity === "metadata" && sidebarOpen ? "active" : ""}`}
              title="Pack Metadata & Packaging"
              onClick={() => {
                setActivity("metadata");
                setSidebarOpen(true);
                openOrFocusTab({
                  id: "tool:metadata",
                  kind: "metadata",
                  title: "Metadata & Packaging",
                });
              }}
            >
              <span className="codicon codicon-package" />
            </button>
          </div>

          <div className="activity-group">
            <button
              className={`activity-btn ${activity === "settings" && sidebarOpen ? "active" : ""}`}
              title="Settings & File Managers"
              onClick={() => {
                if (activity === "settings" && sidebarOpen) setSidebarOpen(false);
                else {
                  setActivity("settings");
                  setSidebarOpen(true);
                }
              }}
            >
              <span className="codicon codicon-settings-gear" />
            </button>
          </div>
        </div>

        {/* Resizable Side Bar + Editor/Bottom Panel Split */}
        <div className="main-split-wrap sash-override">
          <Allotment>
            {sidebarOpen && (
              <Allotment.Pane preferredSize={270} minSize={180} maxSize={520}>
                {activity === "explorer" && (
                  <ExplorerSidebar
                    project={project}
                    selectedRelPath={selectedRelPath}
                    problems={problems}
                    filterText={explorerFilter}
                    onFilterTextChange={setExplorerFilter}
                    onSelectPath={setSelectedRelPath}
                    onOpenAsset={(rel, mode) => void openAssetTab(rel, mode)}
                    onOpenBlockbenchAsset={(rel) => void handleOpenBlockbench(rel)}
                    onCreateFile={(rel) => {
                      void api
                        .fsCreateFile(
                          rel,
                          rel.endsWith(".json") ? "{\n  \n}\n" : "",
                        )
                        .then((snap) => {
                          setProject(snap);
                          void openAssetTab(rel, "code");
                        })
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onCreateDir={(rel) => {
                      void api
                        .fsCreateDir(rel)
                        .then(setProject)
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onCreateBlankTexture={(dir) => {
                      setBlankTexModal({
                        dir,
                        name: "new_texture.png",
                        size: 16,
                      });
                    }}
                    onRenameNode={(oldRel, newName, isDir) => {
                      void api
                        .fsRename(oldRel, newName, isDir)
                        .then((res) => {
                          setProject(res.snapshot);
                          setTabs((prev) =>
                            prev.map((t) =>
                              t.relPath === oldRel
                                ? {
                                    ...t,
                                    relPath: res.new_rel_path,
                                    title: baseName(res.new_rel_path),
                                  }
                                : t,
                            ),
                          );
                        })
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onDeleteNode={(rel) => {
                      void api
                        .fsDelete(rel)
                        .then((snap) => {
                          setProject(snap);
                          setTabs((prev) =>
                            prev.filter((t) => t.relPath !== rel),
                          );
                        })
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onImportFiles={(dir) => {
                      if (settings.useYaziPicker && fileManagers?.yazi_available) {
                        void handlePickWithYazi(false, dir);
                        return;
                      }
                      void dialogOpen({ multiple: true }).then((picked) => {
                        const list = Array.isArray(picked)
                          ? picked
                          : typeof picked === "string"
                            ? [picked]
                            : [];
                        if (list.length > 0) {
                          void handleImportPaths(dir, list);
                        }
                      });
                    }}
                    onOpenProjectPrompt={() => void promptOpenProject()}
                    onOpenNewWizard={() => setWizardOpen(true)}
                    onRefreshProject={() => void refreshActiveProject()}
                    onRevealPath={(rel) => void handleReveal(rel)}
                    onLaunchYazi={handleLaunchYaziInTerminal}
                    onToast={showToast}
                  />
                )}

                {activity === "search" && (
                  <div className="sidebar-pane">
                    <div className="sidebar-header">
                      <span>SEARCH ASSETS &amp; IDS</span>
                    </div>
                    <div className="tree-filter-bar">
                      <input
                        className="vsc-input"
                        style={{ flex: 1 }}
                        placeholder="Search file path or minecraft:id..."
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                      />
                    </div>
                    <div className="tree-scroll">
                      {!project ? (
                        <div style={{ padding: 12, color: "var(--vsc-text-dim)" }}>
                          Open a pack first to search assets.
                        </div>
                      ) : (
                        searchResults.map((f) => (
                          <div
                            key={f}
                            className="tree-row"
                            style={{
                              paddingLeft: 10,
                              height: "auto",
                              minHeight: 28,
                              paddingTop: 4,
                              paddingBottom: 4,
                              alignItems: "flex-start",
                              flexDirection: "column",
                              gap: 1,
                            }}
                            onClick={() => void openAssetTab(f)}
                          >
                            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                              <span
                                className={`codicon ${fileIconClass(f, false)}`}
                              />
                              <span style={{ color: "#e0e0e0" }}>{baseName(f)}</span>
                            </div>
                            <span
                              style={{
                                fontSize: 11,
                                color: "#8c8c8c",
                                fontFamily: "var(--font-mono)",
                                paddingLeft: 18,
                              }}
                            >
                              {computeResourceId(f, project.edition)}
                            </span>
                          </div>
                        ))
                      )}
                    </div>
                  </div>
                )}

                {activity === "converter" && (
                  <div className="sidebar-pane">
                    <div className="sidebar-header">
                      <span>CONVERTER STUDIO</span>
                    </div>
                    <div className="panel-scroll" style={{ padding: 12 }}>
                      <p style={{ color: "var(--vsc-text-dim)", marginTop: 0 }}>
                        Run Preflight Scan and convert Java &harr; Bedrock with Semantic Audit Gate verification.
                      </p>
                      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                        <button
                          className="vsc-btn"
                          onClick={() =>
                            openOrFocusTab({
                              id: "tool:converter",
                              kind: "converter",
                              title: "Converter & Preflight",
                            })
                          }
                        >
                          Open Converter Studio Tab
                        </button>
                        <button
                          className="vsc-btn secondary"
                          disabled={!project}
                          onClick={() => void handleRunAudit()}
                        >
                          Run Semantic Audit Gate
                        </button>
                      </div>
                    </div>
                  </div>
                )}

                {activity === "metadata" && (
                  <div className="sidebar-pane">
                    <div className="sidebar-header">
                      <span>METADATA &amp; EXPORT</span>
                    </div>
                    <div className="panel-scroll" style={{ padding: 12 }}>
                      <p style={{ color: "var(--vsc-text-dim)", marginTop: 0 }}>
                        Edit pack metadata, UUIDv4, min_engine_version [1,x,x], or export .zip / .mcpack / .mcaddon.
                      </p>
                      <button
                        className="vsc-btn"
                        onClick={() =>
                          openOrFocusTab({
                            id: "tool:metadata",
                            kind: "metadata",
                            title: "Metadata & Packaging",
                          })
                        }
                      >
                        Open Metadata &amp; Packaging
                      </button>
                    </div>
                  </div>
                )}

                {activity === "settings" && (
                  <div className="sidebar-pane">
                    <div className="sidebar-header">
                      <span>SETTINGS &amp; YAZI</span>
                      <button
                        className="icon-btn"
                        title="Re-detect File Managers"
                        onClick={() => void refreshFileManagers()}
                      >
                        <span className="codicon codicon-refresh" />
                      </button>
                    </div>
                    <div className="panel-scroll" style={{ padding: 12 }}>
                      <label className="field-label" style={{ marginBottom: 10 }}>
                        <span>Preferred Reveal File Manager</span>
                        <select
                          className="vsc-select"
                          value={settings.preferredFileManager}
                          onChange={(e) =>
                            setSettings((s) => ({
                              ...s,
                              preferredFileManager: e.target.value,
                            }))
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

                      <div
                        style={{
                          fontSize: 11,
                          padding: "8px 10px",
                          background: "#1b1b1c",
                          borderRadius: 4,
                          border: "1px solid var(--vsc-border-strong)",
                          marginBottom: 12,
                        }}
                      >
                        <div>
                          <strong>Yazi Status:</strong>{" "}
                          {fileManagers?.yazi_available ? (
                            <span style={{ color: "var(--vsc-ok)" }}>
                              Detected ({fileManagers.yazi_path})
                            </span>
                          ) : (
                            <span style={{ color: "var(--vsc-warn)" }}>
                              Not found in PATH
                            </span>
                          )}
                        </div>
                        {!fileManagers?.yazi_available && fileManagers && (
                          <div style={{ marginTop: 4, color: "var(--vsc-text-dim)" }}>
                            {fileManagers.yazi_install_hint}
                          </div>
                        )}
                      </div>

                      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                        <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
                          <input
                            type="checkbox"
                            checked={settings.useYaziPicker}
                            onChange={(e) =>
                              setSettings((s) => ({
                                ...s,
                                useYaziPicker: e.target.checked,
                              }))
                            }
                          />
                          Prefer Yazi Chooser for Imports (when available)
                        </label>

                        <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
                          <input
                            type="checkbox"
                            checked={settings.autoSanitizeImports}
                            onChange={(e) =>
                              setSettings((s) => ({
                                ...s,
                                autoSanitizeImports: e.target.checked,
                              }))
                            }
                          />
                          Auto-sanitize imported filenames ([a-z0-9_-])
                        </label>

                        <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
                          <input
                            type="checkbox"
                            checked={settings.convertImportedImagesToPng}
                            onChange={(e) =>
                              setSettings((s) => ({
                                ...s,
                                convertImportedImagesToPng: e.target.checked,
                              }))
                            }
                          />
                          Convert JPG/WebP imports to PNG automatically
                        </label>

                        <label className="field-label" style={{ marginTop: 6 }}>
                          <span>Default 3D UV-Atlas Guard</span>
                          <select
                            className="vsc-select"
                            value={settings.defaultUvAtlasGuard}
                            onChange={(e) =>
                              setSettings((s) => ({
                                ...s,
                                defaultUvAtlasGuard: e.target
                                  .value as UvAtlasGuardMode,
                              }))
                            }
                          >
                            <option value="safe_fallback2d">Safe Fallback 2D</option>
                            <option value="skip">Skip Offending Models</option>
                            <option value="warn">Warn Only</option>
                          </select>
                        </label>
                      </div>
                    </div>
                  </div>
                )}
              </Allotment.Pane>
            )}

            {/* Right Column: Editor Tabs + Bottom Panel */}
            <Allotment.Pane>
              <Allotment vertical>
                <Allotment.Pane>
                  <div className="editor-shell">
                    <div className="editor-tabs-bar">
                      {tabs.map((tab) => {
                        const isActive = tab.id === activeTabId;
                        return (
                          <div
                            key={tab.id}
                            className={`editor-tab ${isActive ? "active" : ""}`}
                            onClick={() => setActiveTabId(tab.id)}
                            title={tab.relPath || tab.title}
                          >
                            <span
                              className={`codicon ${
                                tab.kind === "converter"
                                  ? "codicon-arrow-swap"
                                  : tab.kind === "metadata"
                                    ? "codicon-package"
                                    : tab.kind === "blockbench"
                                      ? "codicon-cube"
                                      : fileIconClass(tab.relPath || "", false)
                              }`}
                            />
                            <span className="editor-tab-title">{tab.title}</span>
                            {tab.dirty ? (
                              <span className="tab-dirty-dot" title="Unsaved changes" />
                            ) : null}
                            <button
                              className="icon-btn"
                              style={{ width: 18, height: 18 }}
                              onClick={(e) => {
                                e.stopPropagation();
                                closeTab(tab.id);
                              }}
                            >
                              <span
                                className="codicon codicon-close"
                                style={{ fontSize: 12 }}
                              />
                            </button>
                          </div>
                        );
                      })}
                    </div>

                    <div className="editor-content-area">
                      {!activeTab && (
                        <div
                          className="panel-scroll"
                          style={{
                            display: "flex",
                            flexDirection: "column",
                            alignItems: "center",
                            justifyContent: "center",
                            gap: 14,
                          }}
                        >
                          <span
                            className="codicon codicon-layers"
                            style={{ fontSize: 52, color: "#007acc" }}
                          />
                          <h2 style={{ margin: 0, color: "var(--vsc-text-bright)" }}>
                            PackStudio Workbench (Tauri v2)
                          </h2>
                          <p style={{ margin: 0, color: "var(--vsc-text-dim)" }}>
                            VS Code-grade Minecraft Java &amp; Bedrock Resource Pack IDE &amp; Converter
                          </p>
                          <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
                            <button
                              className="vsc-btn"
                              onClick={() => void promptOpenProject()}
                            >
                              <span className="codicon codicon-folder-opened" />
                              Open Pack Folder
                            </button>
                            <button
                              className="vsc-btn secondary"
                              onClick={() => void promptOpenArchive()}
                            >
                              <span className="codicon codicon-file-zip" />
                              Open .zip / .mcpack
                            </button>
                            <button
                              className="vsc-btn secondary"
                              onClick={() => setWizardOpen(true)}
                            >
                              <span className="codicon codicon-add" />
                              New Project Wizard
                            </button>
                            <button
                              className="vsc-btn secondary"
                              onClick={() => void handleOpenBlockbench()}
                            >
                              <span className="codicon codicon-cube" />
                              Launch Blockbench 3D
                            </button>
                          </div>
                        </div>
                      )}

                      {activeTab?.kind === "code" && activeTab.relPath && (
                        <MonacoCodeTab
                          relPath={activeTab.relPath}
                          initialContent={textCache[activeTab.relPath] ?? ""}
                          revealLine={activeTab.revealLine}
                          problems={problems}
                          onDirtyChange={(dirty) => setTabDirty(activeTab.id, dirty)}
                          onSave={async (rel, content) => {
                            const snap = await api.fsWriteText(rel, content);
                            setTextCache((c) => ({ ...c, [rel]: content }));
                            setProject(snap);
                            appendLog("success", `Saved ${rel}`);
                          }}
                          onOpenBlockbench={(rel) =>
                            void handleOpenBlockbench(rel)
                          }
                        />
                      )}

                      {activeTab?.kind === "texture" &&
                        activeTab.relPath &&
                        binaryCache[activeTab.relPath] && (
                          <TexturePaintTab
                            relPath={activeTab.relPath}
                            asset={binaryCache[activeTab.relPath]}
                            onDirtyChange={(dirty) =>
                              setTabDirty(activeTab.id, dirty)
                            }
                            onSaveBase64={async (rel, dataUrl) => {
                              const snap = await api.fsWriteBinaryBase64(
                                rel,
                                dataUrl,
                              );
                              const refreshed =
                                await api.fsReadBinaryBase64(rel);
                              setBinaryCache((c) => ({ ...c, [rel]: refreshed }));
                              setProject(snap);
                              appendLog("success", `Saved texture ${rel}`);
                            }}
                            onDownscale={async (rel, targetSize) => {
                              const res = await api.downscaleTexture(
                                rel,
                                targetSize,
                              );
                              const refreshed =
                                await api.fsReadBinaryBase64(rel);
                              setBinaryCache((c) => ({ ...c, [rel]: refreshed }));
                              setProject(res.snapshot);
                              appendLog(
                                "success",
                                `Downscaled ${rel} to ${res.width}x${res.height} via fast_image_resize Nearest-Neighbor`,
                              );
                            }}
                            onCompositeGrass={async (rel) => {
                              const overlay = project?.all_files.find((f) =>
                                f.toLowerCase().includes("grass_block_side_overlay"),
                              );
                              const outTga = "textures/blocks/grass_side.tga";
                              const res = await api.compositeGrassSide(
                                rel,
                                overlay ?? null,
                                outTga,
                              );
                              setProject(res.snapshot);
                              appendLog(
                                "success",
                                `Generated opaque grass cutout composite: ${res.path}`,
                              );
                            }}
                            onOpenBlockbench={(rel) =>
                              void handleOpenBlockbench(rel)
                            }
                            onToast={showToast}
                          />
                        )}

                      {activeTab?.kind === "converter" && (
                        <ConverterTab
                          project={project}
                          settings={settings}
                          onAuditWarnings={(warns) => {
                            setProblems(warns);
                          }}
                          onLog={appendLog}
                          onOpenConvertedPack={(path) =>
                            void openProjectByPath(path)
                          }
                          onRevealPath={(path) => void handleReveal(path)}
                        />
                      )}

                      {activeTab?.kind === "metadata" && (
                        <MetadataPackagingTab
                          project={project}
                          onProjectUpdated={setProject}
                          onLog={appendLog}
                          onRevealPath={(path) => void handleReveal(path)}
                        />
                      )}

                      {activeTab?.kind === "blockbench" &&
                        activeTab.blockbenchUrl && (
                          <div
                            style={{
                              display: "flex",
                              flexDirection: "column",
                              height: "100%",
                            }}
                          >
                            <div className="code-toolbar">
                              <div className="toolbar-group">
                                <span
                                  className="codicon codicon-cube"
                                  style={{ color: "#3794ff" }}
                                />
                                <strong>Blockbench Web Studio (2-Way Bridge)</strong>
                                <span
                                  style={{
                                    fontFamily: "var(--font-mono)",
                                    fontSize: 11,
                                    color: "var(--vsc-text-dim)",
                                  }}
                                >
                                  {activeTab.blockbenchUrl}
                                </span>
                              </div>
                              <div className="toolbar-group">
                                <button
                                  className="vsc-btn secondary small"
                                  disabled={!project}
                                  onClick={() =>
                                    void api
                                      .blockbenchNewBedrockGeo(
                                        "models/entity/custom_model.geo.json",
                                        "geometry.custom_model",
                                      )
                                      .then((info) => {
                                        openOrFocusTab({
                                          id: "tool:blockbench",
                                          kind: "blockbench",
                                          title: "Blockbench 3D",
                                          blockbenchUrl: info.url,
                                        });
                                      })
                                      .catch((e) =>
                                        appendLog("error", String(e)),
                                      )
                                  }
                                >
                                  New Bedrock .geo.json
                                </button>
                              </div>
                            </div>
                            <iframe
                              title="Blockbench 3D Studio"
                              src={activeTab.blockbenchUrl}
                              style={{
                                flex: 1,
                                border: "none",
                                width: "100%",
                                background: "#1e1e1e",
                              }}
                            />
                          </div>
                        )}
                    </div>
                  </div>
                </Allotment.Pane>

                {bottomOpen && (
                  <Allotment.Pane preferredSize={230} minSize={110} maxSize={500}>
                    <div className="bottom-panel">
                      <div className="bottom-tabs-header">
                        <div className="bottom-tabs-left">
                          <button
                            className={`bottom-tab-btn ${bottomTab === "problems" ? "active" : ""}`}
                            onClick={() => setBottomTab("problems")}
                          >
                            PROBLEMS ({problems.length})
                          </button>
                          <button
                            className={`bottom-tab-btn ${bottomTab === "output" ? "active" : ""}`}
                            onClick={() => setBottomTab("output")}
                          >
                            OUTPUT
                          </button>
                          <button
                            className={`bottom-tab-btn ${bottomTab === "terminal" ? "active" : ""}`}
                            onClick={() => setBottomTab("terminal")}
                          >
                            TERMINAL (PTY / YAZI)
                          </button>
                        </div>

                        <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
                          {bottomTab === "output" && (
                            <button
                              className="icon-btn"
                              title="Clear Output"
                              onClick={() => setLogs([])}
                            >
                              <span className="codicon codicon-clear-all" />
                            </button>
                          )}
                          <button
                            className="icon-btn"
                            title="Close Panel (Ctrl+J)"
                            onClick={() => setBottomOpen(false)}
                          >
                            <span className="codicon codicon-close" />
                          </button>
                        </div>
                      </div>

                      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
                        {bottomTab === "problems" && (
                          <div style={{ padding: "4px 8px" }}>
                            {problems.length === 0 ? (
                              <div
                                style={{
                                  padding: 12,
                                  color: "var(--vsc-text-dim)",
                                }}
                              >
                                No problems reported by Semantic Auditor. Click
                                &ldquo;Audit&rdquo; to inspect the pack.
                              </div>
                            ) : (
                              <table className="table-compact">
                                <thead>
                                  <tr>
                                    <th>Severity</th>
                                    <th>Asset</th>
                                    <th>Message &amp; Fix</th>
                                  </tr>
                                </thead>
                                <tbody>
                                  {problems.map((p, idx) => (
                                    <tr
                                      key={idx}
                                      style={{ cursor: "pointer" }}
                                      onClick={() => {
                                        const target = p.source_path || p.asset;
                                        if (
                                          target &&
                                          project?.all_files.includes(target)
                                        ) {
                                          void openAssetTab(target, undefined, 1);
                                        }
                                      }}
                                    >
                                      <td>
                                        <span
                                          style={{
                                            fontWeight: 600,
                                            color:
                                              p.severity === "error"
                                                ? "var(--vsc-error)"
                                                : p.severity === "warning"
                                                  ? "var(--vsc-warn)"
                                                  : "#9cdcfe",
                                          }}
                                        >
                                          {p.severity.toUpperCase()}
                                        </span>
                                      </td>
                                      <td style={{ fontFamily: "var(--font-mono)" }}>
                                        {p.source_path || p.asset}
                                      </td>
                                      <td>
                                        <div>{p.reason || p.message}</div>
                                        {p.actionable_fix && (
                                          <div
                                            style={{
                                              fontSize: 11,
                                              color: "var(--vsc-text-dim)",
                                            }}
                                          >
                                            Fix: {p.actionable_fix}
                                          </div>
                                        )}
                                      </td>
                                    </tr>
                                  ))}
                                </tbody>
                              </table>
                            )}
                          </div>
                        )}

                        {bottomTab === "output" && (
                          <div
                            style={{
                              padding: "6px 12px",
                              fontFamily: "var(--font-mono)",
                              fontSize: 12,
                              lineHeight: 1.5,
                              userSelect: "text",
                            }}
                          >
                            {logs.map((line) => (
                              <div
                                key={line.id}
                                style={{
                                  color:
                                    line.level === "error"
                                      ? "#ff8080"
                                      : line.level === "warn"
                                        ? "#e5c07b"
                                        : line.level === "success"
                                          ? "#8ce9ad"
                                          : "#cccccc",
                                }}
                              >
                                <span style={{ color: "#767676" }}>
                                  [{line.timestamp}]{" "}
                                </span>
                                {line.message}
                              </div>
                            ))}
                          </div>
                        )}

                        {bottomTab === "terminal" && (
                          <TerminalPanel
                            cwd={project?.source_path ?? null}
                            launchCommandRequest={ptyLaunchReq}
                            yaziAvailable={!!fileManagers?.yazi_available}
                            onLog={appendLog}
                          />
                        )}
                      </div>
                    </div>
                  </Allotment.Pane>
                )}
              </Allotment>
            </Allotment.Pane>
          </Allotment>
        </div>
      </div>

      {/* Status Bar */}
      <div className={`status-bar ${project ? "" : "empty-project"}`}>
        <div className="status-group">
          <span
            className="status-item"
            onClick={() => setSidebarOpen((s) => !s)}
          >
            <span className="codicon codicon-layers" />
            {project
              ? `${project.edition_label} (${project.dominant_resolution ?? "Native"})`
              : "No Pack Open"}
          </span>
          {project && (
            <span className="status-item">
              <span className="codicon codicon-file-media" />
              {project.file_count} files / {project.texture_count} textures
            </span>
          )}
          <span
            className="status-item"
            onClick={() => {
              setBottomOpen(true);
              setBottomTab("problems");
            }}
          >
            <span className="codicon codicon-error" /> {errorCount}{" "}
            <span className="codicon codicon-warning" /> {warnCount}
          </span>
        </div>

        <div className="status-group">
          {fileManagers && (
            <span
              className="status-item"
              onClick={() => {
                setActivity("settings");
                setSidebarOpen(true);
              }}
            >
              <span className="codicon codicon-folder" />
              FM: {fileManagers.yazi_available ? "yazi +" : ""} {fileManagers.primary}
            </span>
          )}
          <span
            className="status-item"
            onClick={() => {
              setBottomOpen(true);
              setBottomTab("terminal");
            }}
          >
            <span className="codicon codicon-terminal" />
            Terminal / Yazi
          </span>
        </div>
      </div>

      {wizardOpen && (
        <NewProjectWizardModal
          onClose={() => setWizardOpen(false)}
          onCreate={async (cfg: NewProjectConfig) => {
            const snap = await api.createProject(cfg);
            setProject(snap);
            setTabs([]);
            setActiveTabId(null);
            setProblems([]);
            appendLog(
              "success",
              `Created ${snap.edition_label} project '${snap.pack_name}'`,
            );
          }}
        />
      )}

      {blankTexModal && (
        <div
          className="modal-backdrop"
          onClick={() => setBlankTexModal(null)}
        >
          <div
            className="modal-dialog"
            style={{ width: 380 }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="sidebar-header">
              <span>Create Blank PNG Texture</span>
              <button
                className="icon-btn"
                onClick={() => setBlankTexModal(null)}
              >
                <span className="codicon codicon-close" />
              </button>
            </div>
            <div style={{ padding: 14, display: "flex", flexDirection: "column", gap: 10 }}>
              <label className="field-label">
                <span>Filename</span>
                <input
                  className="vsc-input mono"
                  value={blankTexModal.name}
                  onChange={(e) =>
                    setBlankTexModal({ ...blankTexModal, name: e.target.value })
                  }
                />
              </label>
              <label className="field-label">
                <span>Resolution Preset</span>
                <select
                  className="vsc-select"
                  value={blankTexModal.size}
                  onChange={(e) =>
                    setBlankTexModal({
                      ...blankTexModal,
                      size: Number(e.target.value),
                    })
                  }
                >
                  <option value={16}>16x16 Vanilla</option>
                  <option value={32}>32x32 HD</option>
                  <option value={64}>64x64 HD</option>
                  <option value={128}>128x128 Ultra</option>
                </select>
              </label>
              <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
                <button
                  className="vsc-btn secondary"
                  onClick={() => setBlankTexModal(null)}
                >
                  Cancel
                </button>
                <button
                  className="vsc-btn"
                  onClick={() => {
                    const rel = blankTexModal.dir
                      ? `${blankTexModal.dir}/${blankTexModal.name}`
                      : blankTexModal.name;
                    const size = blankTexModal.size;
                    setBlankTexModal(null);
                    void api
                      .createBlankTexture(rel, size, size)
                      .then((snap) => {
                        setProject(snap);
                        void openAssetTab(rel, "texture");
                      })
                      .catch((e) => appendLog("error", String(e)));
                  }}
                >
                  Create &amp; Paint
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {paletteMode && (
        <div
          className="modal-backdrop"
          onClick={() => setPaletteMode(null)}
        >
          <div className="palette-box" onClick={(e) => e.stopPropagation()}>
            <div style={{ padding: 8, borderBottom: "1px solid var(--vsc-border)" }}>
              <input
                className="vsc-input"
                style={{ width: "100%" }}
                autoFocus
                placeholder={
                  paletteMode === "commands"
                    ? "> Type a PackStudio command..."
                    : "Type asset path to quick-open..."
                }
                value={paletteQuery}
                onChange={(e) => setPaletteQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    if (paletteMode === "commands" && filteredCommands[0]) {
                      filteredCommands[0].action();
                      setPaletteMode(null);
                    } else if (
                      paletteMode === "files" &&
                      filteredQuickFiles[0]
                    ) {
                      void openAssetTab(filteredQuickFiles[0]);
                      setPaletteMode(null);
                    }
                  }
                }}
              />
            </div>
            <div style={{ maxHeight: 360, overflow: "auto", padding: "4px 0" }}>
              {paletteMode === "commands"
                ? filteredCommands.map((cmd) => (
                    <div
                      key={cmd.label}
                      className="context-item"
                      onClick={() => {
                        cmd.action();
                        setPaletteMode(null);
                      }}
                    >
                      <span className="codicon codicon-chevron-right" />
                      {cmd.label}
                    </div>
                  ))
                : filteredQuickFiles.map((f) => (
                    <div
                      key={f}
                      className="context-item"
                      onClick={() => {
                        void openAssetTab(f);
                        setPaletteMode(null);
                      }}
                    >
                      <span className={`codicon ${fileIconClass(f, false)}`} />
                      <span style={{ fontFamily: "var(--font-mono)" }}>{f}</span>
                    </div>
                  ))}
            </div>
          </div>
        </div>
      )}

      {toastMsg && (
        <div
          style={{
            position: "fixed",
            right: 16,
            bottom: 34,
            zIndex: 9500,
            background: "#252526",
            border: "1px solid var(--vsc-status-blue)",
            padding: "8px 14px",
            borderRadius: 4,
            boxShadow: "0 6px 18px rgba(0,0,0,0.5)",
            fontSize: 12,
          }}
        >
          {toastMsg}
        </div>
      )}
    </div>
  );
}
