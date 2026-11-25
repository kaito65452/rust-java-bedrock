import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Allotment } from "allotment";
import { api } from "./api";
import { ConverterTab } from "./components/ConverterTab";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ExplorerSidebar } from "./components/ExplorerSidebar";
import { ExtensionsSidebar } from "./components/ExtensionsSidebar";
import { ExtensionsTab } from "./components/ExtensionsTab";
import { ImportAssetsModal } from "./components/ImportAssetsModal";
import { MetadataPackagingTab } from "./components/MetadataPackagingTab";
import { MonacoCodeTab } from "./components/MonacoCodeTab";
import { NewProjectWizardModal } from "./components/NewProjectWizardModal";
import { PstdrUpdateModal } from "./components/PstdrUpdateModal";
import {
  GUIDED_PSTDR_STARTER_SOURCE,
  QuickTutorialView,
} from "./components/QuickTutorialView";
import { SettingsView, type SettingsCategory } from "./components/SettingsView";
import { TerminalPanel } from "./components/TerminalPanel";
import { TexturePaintTab } from "./components/TexturePaintTab";
import { VoxelTesseractLogo } from "./components/VoxelTesseractLogo";
import type {
  ActivityView,
  AiAgentsStatusDto,
  BinaryAssetData,
  BlockbenchSavedEvent,
  BottomTab,
  ConversionWarning,
  DetectedFileManagers,
  EditorTab,
  ExtensionsSnapshotDto,
  GithubAuthStatusDto,
  LspDiagnosticDto,
  LspDiagnosticsEvent,
  LspServerStatusDto,
  NewProjectConfig,
  OutputLogLine,
  ProjectSnapshot,
  PstdrSyncStatusDto,
  PstdrUpdateOfferDto,
  RecentProjectEntry,
  StudioSettings,
  UvAtlasGuardMode,
} from "./types";
import { safeListen, safeUnlisten } from "./utils/tauriEvents";
import { APP_DISPLAY_VERSION } from "./utils/version";
import {
  baseName,
  computeResourceId,
  fileIconClass,
  isImageAsset,
  isModelAsset,
  isTextCodeAsset,
  packTypeBadgeLabel,
} from "./utils/mcFormat";

const DEFAULT_SETTINGS: StudioSettings = {
  preferredFileManager: "auto",
  autoSanitizeImports: true,
  convertImportedImagesToPng: true,
  defaultTargetResolution: null,
  defaultUvAtlasGuard: "safe_fallback2d",
  defaultCompositeGrassTga: true,
  defaultConvertPbr: false,
  externalImageEditor: "",
  blockbenchUiScale: 0.86,
};

const RECENT_PROJECTS_KEY = "pack_studio_recent_projects_v1";

function loadRecentProjects(): RecentProjectEntry[] {
  try {
    const raw = localStorage.getItem(RECENT_PROJECTS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return parsed.filter(
          (item): item is RecentProjectEntry =>
            Boolean(item && typeof item.path === "string" && typeof item.name === "string"),
        );
      }
    }
  } catch {
    // ignore storage errors
  }
  return [];
}

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
  const [recentProjects, setRecentProjects] =
    useState<RecentProjectEntry[]>(loadRecentProjects);
  const [fileManagers, setFileManagers] =
    useState<DetectedFileManagers | null>(null);
  const [extensionsSnapshot, setExtensionsSnapshot] =
    useState<ExtensionsSnapshotDto | null>(null);
  const [lspServers, setLspServers] = useState<LspServerStatusDto[]>([]);
  const [lspDiagMap, setLspDiagMap] = useState<
    Record<string, LspDiagnosticDto[]>
  >({});

  const [wizardOpen, setWizardOpen] = useState(false);
  const [pstdrUpdateModalOpen, setPstdrUpdateModalOpen] = useState(false);
  const [pstdrSyncStatus, setPstdrSyncStatus] =
    useState<PstdrSyncStatusDto | null>(null);
  const [pstdrUpdateOffer, setPstdrUpdateOffer] =
    useState<PstdrUpdateOfferDto | null>(null);
  const [githubStatus, setGithubStatus] =
    useState<GithubAuthStatusDto | null>(null);
  const [aiAgentsStatus, setAiAgentsStatus] =
    useState<AiAgentsStatusDto | null>(null);
  const [activityMenu, setActivityMenu] = useState<null | "accounts" | "gear">(
    null,
  );
  const [settingsCategory, setSettingsCategory] =
    useState<SettingsCategory>("updates");
  const [settingsNavNonce, setSettingsNavNonce] = useState(0);
  const [activityAvatarError, setActivityAvatarError] = useState(false);
  const activityBottomRef = useRef<HTMLDivElement | null>(null);
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
  const [importModal, setImportModal] = useState<null | { dir: string }>(null);

  const blockbenchFrameRef = useRef<HTMLIFrameElement | null>(null);
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

  useEffect(() => {
    try {
      localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(recentProjects));
    } catch {
      // ignore storage errors
    }
  }, [recentProjects]);

  const recordRecentProject = useCallback((snap: ProjectSnapshot) => {
    const src = snap.source_path;
    if (!src) return;
    setRecentProjects((prev) => {
      const entry: RecentProjectEntry = {
        path: src,
        name: snap.pack_name,
        edition_label: snap.edition_label,
        pack_type: snap.pack_type,
        opened_at: Date.now(),
      };
      return [entry, ...prev.filter((item) => item.path !== src)].slice(0, 8);
    });
  }, []);

  const refreshFileManagers = useCallback(async () => {
    try {
      const fm = await api.detectFileManagers();
      setFileManagers(fm);
    } catch (e) {
      appendLog("warn", `File manager detection error: ${String(e)}`);
    }
  }, [appendLog]);

  const refreshExtensionsAndLsp = useCallback(async () => {
    try {
      const [extSnap, srvs] = await Promise.all([
        api.extList(),
        api.lspListServers(),
      ]);
      setExtensionsSnapshot(extSnap);
      setLspServers(srvs);
    } catch (e) {
      appendLog("warn", `Extension/LSP scan error: ${String(e)}`);
    }
  }, [appendLog]);

  useEffect(() => {
    appendLog(
      "info",
      `PackStudio v${APP_DISPLAY_VERSION} ready. Use Ctrl+Shift+P for Command Palette.`,
    );
    void refreshFileManagers();
    void refreshExtensionsAndLsp();
    void api
      .githubAuthStatus()
      .then(setGithubStatus)
      .catch(() => {});
    void api
      .aiAgentsStatus()
      .then(setAiAgentsStatus)
      .catch(() => {});
    void api
      .pstdrSyncStatus()
      .then((st) => {
        setPstdrSyncStatus(st);
        if (st.pubkey_configured) {
          // Passive check only: never auto-downloads or auto-applies updates.
          void api
            .pstdrSyncCheck()
            .then((offer) => {
              if (offer.update_available) {
                setPstdrUpdateOffer(offer);
                appendLog(
                  "info",
                  `Verified PSTDR update v${offer.version} available. Click the status bar badge to review fingerprints.`,
                );
              }
            })
            .catch(() => {});
        }
      })
      .catch(() => {});
  }, [appendLog, refreshFileManagers, refreshExtensionsAndLsp]);

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
    const stopFs = safeListen<string[]>("fs://changed", (ev) => {
      appendLog("info", `Filesystem changed: ${ev.payload.join(", ")}`);
      void refreshActiveProject();
    });
    const stopBb = safeListen<BlockbenchSavedEvent>(
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
    const stopLsp = safeListen<LspDiagnosticsEvent>(
      "lsp://diagnostics",
      (ev) => {
        const payload = ev.payload;
        if (!payload || typeof payload.rel_path !== "string") return;
        const safeDiags = Array.isArray(payload.diagnostics)
          ? payload.diagnostics
          : [];
        api.frontendTraceLog(
          "trace",
          "app.lsp",
          `Received lsp://diagnostics server='${payload.server_id}' path='${payload.rel_path}' count=${safeDiags.length}`,
        );
        const key = `${payload.server_id ?? "lsp"}::${payload.rel_path}`;
        setLspDiagMap((prev) => ({
          ...prev,
          [key]: safeDiags,
        }));
      },
    );
    return () => {
      stopFs();
      stopBb();
      stopLsp();
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
    let cancelled = false;
    let unlistenDrop: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((ev) => {
        if (cancelled) return;
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
        if (cancelled) {
          safeUnlisten(fn);
        } else {
          unlistenDrop = fn;
        }
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      safeUnlisten(unlistenDrop);
      unlistenDrop = undefined;
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

  useEffect(() => {
    setActivityAvatarError(false);
  }, [githubStatus?.avatar_url]);

  useEffect(() => {
    if (!activityMenu) return;
    const onPointerDown = (e: MouseEvent) => {
      if (
        activityBottomRef.current &&
        !activityBottomRef.current.contains(e.target as Node)
      ) {
        setActivityMenu(null);
      }
    };
    window.addEventListener("mousedown", onPointerDown);
    return () => window.removeEventListener("mousedown", onPointerDown);
  }, [activityMenu]);

  const openQuickTutorial = useCallback(() => {
    setActivityMenu(null);
    openOrFocusTab({
      id: "tool:tutorial",
      kind: "tutorial",
      title: "Quick Tutorial",
    });
  }, [openOrFocusTab]);

  const openSettingsCategory = useCallback(
    (category: SettingsCategory) => {
      setActivityMenu(null);
      setSettingsCategory(category);
      setSettingsNavNonce((n) => n + 1);
      openOrFocusTab({
        id: "tool:settings",
        kind: "settings",
        title: "Settings",
      });
    },
    [openOrFocusTab],
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

  const openAssetTab = useCallback(
    async (
      relPath: string,
      forceMode?: "code" | "texture",
      revealLine?: number,
    ) => {
      if (!projectRef.current) return;
      if (!forceMode && isModelAsset(relPath)) {
        await handleOpenBlockbench(relPath);
        return;
      }
      const mode: "code" | "texture" =
        forceMode ??
        (isImageAsset(relPath)
          ? "texture"
          : isTextCodeAsset(relPath) || isModelAsset(relPath)
            ? "code"
            : "code");

      api.frontendTraceLog(
        "trace",
        "app.openAssetTab",
        `Opening '${relPath}' (mode='${mode}', forceMode='${forceMode ?? "auto"}')`,
      );

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
    [appendLog, handleOpenBlockbench, openOrFocusTab],
  );

  const openProjectByPath = useCallback(
    async (path: string) => {
      try {
        const snap = await api.openProject(path);
        setProject(snap);
        recordRecentProject(snap);
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
        const metaFile =
          snap.all_files.find((f) => f === "pstdr.toml") ??
          snap.all_files.find((f) => f === "src/main.pstdr") ??
          snap.all_files.find(
            (f) =>
              f === "pack.mcmeta" ||
              f === "manifest.json" ||
              f === "BP/manifest.json" ||
              f === "RP/manifest.json",
          );
        if (metaFile) {
          void openAssetTab(metaFile, "code");
        }
      } catch (e) {
        appendLog("error", `Open project failed: ${String(e)}`);
      }
    },
    [appendLog, openAssetTab, recordRecentProject],
  );

  const handleOpenPstdrStarter = useCallback(async () => {
    const activeProject = projectRef.current;
    if (activeProject) {
      const targetRel =
        activeProject.all_files.find((f) => f === "src/tutorial_starter.pstdr") ??
        activeProject.all_files.find((f) => f.toLowerCase().endsWith(".pstdr")) ??
        "src/tutorial_starter.pstdr";
      try {
        if (!activeProject.all_files.includes(targetRel)) {
          const snap = await api.fsCreateFile(
            targetRel,
            GUIDED_PSTDR_STARTER_SOURCE,
          );
          setProject(snap);
          appendLog("success", `Created guided PSTDR starter: ${targetRel}`);
        }
        await openAssetTab(targetRel, "code");
      } catch (e) {
        appendLog("error", `Failed opening guided .pstdr starter: ${String(e)}`);
      }
      return;
    }

    // No project open yet: open a live scratch starter in Monaco so users can explore immediately
    const scratchRel = "tutorial_starter.pstdr";
    setTextCache((prev) => ({
      ...prev,
      [scratchRel]: GUIDED_PSTDR_STARTER_SOURCE,
    }));
    openOrFocusTab({
      id: `code:${scratchRel}`,
      kind: "code",
      title: scratchRel,
      relPath: scratchRel,
    });
    showToast(
      "Opened guided PSTDR Wave 3B starter — create or open a project to save & build.",
    );
  }, [appendLog, openAssetTab, openOrFocusTab, showToast]);

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
      filters: [
        { name: "Resource Packs & Add-ons", extensions: ["zip", "mcpack", "mcaddon"] },
      ],
    });
    if (typeof picked === "string") {
      await openProjectByPath(picked);
    }
  }, [openProjectByPath]);

  const applyBlockbenchScale = useCallback(() => {
    const frame = blockbenchFrameRef.current;
    if (!frame?.contentWindow) return;
    frame.contentWindow.postMessage(
      { type: "ps_set_zoom", zoom: settings.blockbenchUiScale },
      "*",
    );
    frame.contentWindow.postMessage({ type: "ps_resize" }, "*");
  }, [settings.blockbenchUiScale]);

  useEffect(() => {
    if (activeTabId === "tool:blockbench") {
      applyBlockbenchScale();
    }
  }, [applyBlockbenchScale, activeTabId]);

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

  const [buildingPstdr, setBuildingPstdr] = useState(false);

  const hasPstdrWorkspace = useMemo(
    () =>
      project?.pack_type === "bedrock_addon" &&
      project.all_files.some((f) => f.toLowerCase().endsWith(".pstdr")),
    [project],
  );

  const handleCheckPstdr = useCallback(async () => {
    if (!projectRef.current) return;
    appendLog("info", "Checking .pstdr workspace sources...");
    try {
      const res = await api.pstdrCheckWorkspace();
      setProblems(res.problems);
      setBottomOpen(true);
      setBottomTab(res.error_count > 0 ? "problems" : "output");
      appendLog(
        res.success ? "success" : "error",
        `${res.summary} (${res.elapsed_ms}ms)`,
      );
      for (const p of res.problems) {
        appendLog(
          p.severity === "error" ? "error" : "warn",
          `  ${p.source_path}: ${p.reason}`,
        );
      }
    } catch (e) {
      appendLog("error", `PSTDR check error: ${String(e)}`);
    }
  }, [appendLog]);

  const handleBuildPstdr = useCallback(async () => {
    if (!projectRef.current || buildingPstdr) return;
    setBuildingPstdr(true);
    appendLog("info", "Building .pstdr workspace into BP/scripts/generated/...");
    try {
      const res = await api.pstdrConvertWorkspace();
      if (res.snapshot) {
        setProject(res.snapshot);
      }
      setProblems(res.problems);
      setBottomOpen(true);
      setBottomTab(res.error_count > 0 ? "problems" : "output");
      appendLog(
        res.success ? "success" : "error",
        `${res.summary} (${res.elapsed_ms}ms)`,
      );
      for (const p of res.problems) {
        appendLog(
          p.severity === "error" ? "error" : "warn",
          `  ${p.source_path}: ${p.reason}`,
        );
      }
      showToast(
        res.success
          ? `Built ${res.file_count} .pstdr file(s)`
          : `PSTDR build failed (${res.error_count} error(s))`,
      );
    } catch (e) {
      appendLog("error", `PSTDR build error: ${String(e)}`);
    } finally {
      setBuildingPstdr(false);
    }
  }, [appendLog, buildingPstdr, showToast]);

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
      } else if (ctrl && e.shiftKey && e.key.toLowerCase() === "x") {
        e.preventDefault();
        setActivity("extensions");
        setSidebarOpen(true);
      } else if (ctrl && !e.shiftKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void promptOpenProject();
      } else if (ctrl && e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setWizardOpen(true);
      } else if (ctrl && !e.shiftKey && e.key === ",") {
        e.preventDefault();
        openSettingsCategory("updates");
      } else if (e.key === "Escape") {
        setPaletteMode(null);
        setActivityMenu(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [openSettingsCategory, promptOpenProject]);

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
  const blockbenchTab =
    tabs.find((t) => t.kind === "blockbench" && t.blockbenchUrl) ?? null;

  const commandActions = useMemo(
    () => [
      {
        label: "File: Open Resource Pack Folder...",
        action: () => void promptOpenProject(),
      },
      {
        label: "File: Open Archive (.zip / .mcpack / .mcaddon)...",
        action: () => void promptOpenArchive(),
      },
      {
        label: "File: New Resource Pack / Add-On Wizard...",
        action: () => setWizardOpen(true),
      },
      {
        label: "File: Import Assets into Open Pack...",
        action: () => setImportModal({ dir: "" }),
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
        label: "Extensions: Open Extensions & LSP Manager (Ctrl+Shift+X)",
        action: () => {
          setActivity("extensions");
          setSidebarOpen(true);
          openOrFocusTab({
            id: "tool:extensions",
            kind: "extensions",
            title: "Extensions & LSP",
          });
        },
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
        label: "Help: Open Quick Tutorial",
        action: () => openQuickTutorial(),
      },
      {
        label: "Preferences: Open Settings (Ctrl+,)",
        action: () =>
          openOrFocusTab({
            id: "tool:settings",
            kind: "settings",
            title: "Settings",
          }),
      },
      {
        label: "PSTDR: Check for Verified Updates...",
        action: () => setPstdrUpdateModalOpen(true),
      },
      ...(hasPstdrWorkspace
        ? [
            {
              label: "PSTDR: Build .pstdr",
              action: () => void handleBuildPstdr(),
            },
            {
              label: "PSTDR: Check .pstdr Workspace",
              action: () => void handleCheckPstdr(),
            },
          ]
        : []),
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
      handleBuildPstdr,
      handleCheckPstdr,
      handleOpenBlockbench,
      handleRunAudit,
      hasPstdrWorkspace,
      openOrFocusTab,
      openQuickTutorial,
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

  const allLspDiagnostics = useMemo(
    () => Object.values(lspDiagMap).flat(),
    [lspDiagMap],
  );

  const mergedProblems = useMemo(() => {
    const lspWarnings: ConversionWarning[] = allLspDiagnostics
      .filter((d): d is LspDiagnosticDto => Boolean(d && typeof d.rel_path === "string"))
      .map((d) => ({
        source_path: d.rel_path,
        stage: "audit",
        severity: d.severity ?? "info",
        reason: `${d.message ?? "Diagnostic"} (L${d.line ?? 1}:${d.column ?? 1})`,
        actionable_fix: `LSP: ${d.source ?? "lsp"}`,
        asset: d.rel_path,
        message: d.message ?? "Diagnostic",
      }));
    return [...problems, ...lspWarnings];
  }, [problems, allLspDiagnostics]);

  const errorCount = mergedProblems.filter((p) => p.severity === "error").length;
  const warnCount = mergedProblems.filter((p) => p.severity === "warning").length;
  const runningLspCount = lspServers.filter((s) => s.running).length;

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
          <span
            className="brand-pill"
            onClick={() => openQuickTutorial()}
            style={{ cursor: "pointer" }}
            title={`PackStudio v${APP_DISPLAY_VERSION} — Open Quick Tutorial`}
          >
            <VoxelTesseractLogo variant="badge" size={18} />
            PackStudio
          </span>
          {project && (
            <>
              <span style={{ fontSize: 12, color: "#cccccc" }}>
                {project.pack_name}
              </span>
              <span
                className="edition-badge"
                style={{
                  color:
                    project.pack_type === "java_resource_pack"
                      ? "#73c991"
                      : project.pack_type === "bedrock_addon"
                        ? "#f5b83d"
                        : "#6cb6ff",
                }}
                title={project.edition_label}
              >
                {packTypeBadgeLabel(project.pack_type)}
              </span>
              <span className="edition-badge">{project.edition_label}</span>
              {project.companion_bp_dir && project.pack_type !== "bedrock_addon" && (
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
          {hasPstdrWorkspace && (
            <button
              className="vsc-btn small"
              disabled={buildingPstdr}
              onClick={() => void handleBuildPstdr()}
              title="Compile .pstdr sources into BP/scripts/generated/ and wire custom components"
            >
              <span className="codicon codicon-play" />
              {buildingPstdr ? "Building .pstdr..." : "Build .pstdr"}
            </button>
          )}
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

            <button
              className={`activity-btn ${activity === "extensions" && sidebarOpen ? "active" : ""}`}
              title="Extensions & LSP (Ctrl+Shift+X)"
              onClick={() => {
                if (activity === "extensions" && sidebarOpen) {
                  setSidebarOpen(false);
                } else {
                  setActivity("extensions");
                  setSidebarOpen(true);
                }
              }}
            >
              <span className="codicon codicon-extensions" />
              {runningLspCount > 0 && (
                <span className="activity-badge">{runningLspCount}</span>
              )}
            </button>
          </div>

          <div
            className="activity-group"
            ref={activityBottomRef}
            style={{ position: "relative" }}
          >
            {/* Accounts / GitHub Profile Avatar */}
            <button
              className={`activity-btn ${activityMenu === "accounts" ? "active" : ""}`}
              title={
                githubStatus?.connected && githubStatus.login
                  ? `Accounts — GitHub (@${githubStatus.login})`
                  : "Accounts — Connect GitHub"
              }
              onClick={() =>
                setActivityMenu((cur) => (cur === "accounts" ? null : "accounts"))
              }
            >
              {githubStatus?.connected ? (
                <span
                  style={{
                    position: "relative",
                    display: "inline-flex",
                    alignItems: "center",
                    justifyContent: "center",
                    width: 24,
                    height: 24,
                  }}
                >
                  {githubStatus.avatar_url && !activityAvatarError ? (
                    <img
                      src={githubStatus.avatar_url}
                      alt={githubStatus.login ?? "GitHub"}
                      onError={() => setActivityAvatarError(true)}
                      style={{
                        width: 24,
                        height: 24,
                        borderRadius: "50%",
                        objectFit: "cover",
                        border: "1px solid rgba(255, 255, 255, 0.25)",
                        background: "#18181a",
                      }}
                    />
                  ) : (
                    <span
                      style={{
                        width: 24,
                        height: 24,
                        borderRadius: "50%",
                        background: "#163825",
                        border: "1px solid #3fb950",
                        color: "#7ee787",
                        fontSize: 11,
                        fontWeight: 700,
                        display: "inline-flex",
                        alignItems: "center",
                        justifyContent: "center",
                      }}
                    >
                      {(githubStatus.login ?? "G").slice(0, 1).toUpperCase()}
                    </span>
                  )}
                  <span
                    style={{
                      position: "absolute",
                      right: -1,
                      bottom: -1,
                      width: 8,
                      height: 8,
                      borderRadius: "50%",
                      background: "#3fb950",
                      border: "1.5px solid var(--vsc-activity, #18181a)",
                    }}
                  />
                </span>
              ) : (
                <span className="codicon codicon-account" />
              )}
            </button>

            {/* Manage / Settings Gear */}
            <button
              className={`activity-btn ${
                activityMenu === "gear" || activeTab?.kind === "settings"
                  ? "active"
                  : ""
              }`}
              title="Manage & Settings (Ctrl+,)"
              onClick={() =>
                setActivityMenu((cur) => (cur === "gear" ? null : "gear"))
              }
            >
              <span className="codicon codicon-settings-gear" />
              {pstdrUpdateOffer?.update_available && (
                <span className="activity-badge" style={{ background: "#2e7d53" }}>
                  1
                </span>
              )}
            </button>

            {/* Floating Accounts Popover */}
            {activityMenu === "accounts" && (
              <div
                className="menu-popover"
                style={{ left: 54, bottom: 68, minWidth: 236 }}
              >
                {githubStatus?.connected ? (
                  <>
                    <div
                      style={{
                        padding: "7px 9px",
                        display: "flex",
                        alignItems: "center",
                        gap: 9,
                        borderBottom: "1px solid rgba(255, 255, 255, 0.08)",
                        marginBottom: 4,
                      }}
                    >
                      {githubStatus.avatar_url && !activityAvatarError ? (
                        <img
                          src={githubStatus.avatar_url}
                          alt={githubStatus.login ?? "GitHub"}
                          onError={() => setActivityAvatarError(true)}
                          style={{
                            width: 28,
                            height: 28,
                            borderRadius: "50%",
                            objectFit: "cover",
                            border: "1px solid #3fb950",
                          }}
                        />
                      ) : (
                        <span
                          style={{
                            width: 28,
                            height: 28,
                            borderRadius: "50%",
                            background: "#163825",
                            border: "1px solid #3fb950",
                            color: "#7ee787",
                            fontSize: 12,
                            fontWeight: 700,
                            display: "inline-flex",
                            alignItems: "center",
                            justifyContent: "center",
                          }}
                        >
                          {(githubStatus.login ?? "G").slice(0, 1).toUpperCase()}
                        </span>
                      )}
                      <div style={{ minWidth: 0, flex: 1 }}>
                        <div
                          style={{
                            fontSize: 12,
                            fontWeight: 600,
                            color: "#ffffff",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                        >
                          @{githubStatus.login}
                        </div>
                        <div style={{ fontSize: 10.5, color: "#7ee787" }}>
                          GitHub Connected
                        </div>
                      </div>
                    </div>
                    <div
                      className="menu-item"
                      onClick={() => openSettingsCategory("github")}
                    >
                      <span className="codicon codicon-github-inverted" />
                      <span>Manage GitHub Account</span>
                    </div>
                    <div
                      className="context-item danger"
                      onClick={() => {
                        setActivityMenu(null);
                        void api
                          .githubDisconnect()
                          .then((next) => {
                            setGithubStatus(next);
                            appendLog(
                              "info",
                              "Disconnected GitHub account and removed stored token.",
                            );
                          })
                          .catch((e) =>
                            appendLog("error", `GitHub disconnect failed: ${String(e)}`),
                          );
                      }}
                    >
                      <span className="codicon codicon-sign-out" />
                      <span>Disconnect</span>
                    </div>
                  </>
                ) : (
                  <>
                    <div
                      style={{
                        padding: "6px 9px",
                        fontSize: 11,
                        color: "var(--vsc-text-dim)",
                        borderBottom: "1px solid rgba(255, 255, 255, 0.08)",
                        marginBottom: 4,
                      }}
                    >
                      Not signed in to GitHub
                    </div>
                    <div
                      className="menu-item"
                      onClick={() => openSettingsCategory("github")}
                    >
                      <span className="codicon codicon-github-inverted" />
                      <span>Connect GitHub Account...</span>
                    </div>
                  </>
                )}
              </div>
            )}

            {/* Floating Gear / Settings Popover */}
            {activityMenu === "gear" && (
              <div
                className="menu-popover"
                style={{ left: 54, bottom: 42, minWidth: 256 }}
              >
                <div
                  className="menu-item"
                  style={{ justifyContent: "space-between" }}
                  onClick={() => {
                    setActivityMenu(null);
                    setPaletteQuery("");
                    setPaletteMode("commands");
                  }}
                >
                  <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span className="codicon codicon-terminal-cmd" />
                    <span>Command Palette...</span>
                  </span>
                  <span style={{ fontSize: 10.5, color: "var(--vsc-text-dim)" }}>
                    Ctrl+Shift+P
                  </span>
                </div>

                <div
                  className="menu-item"
                  onClick={() => openQuickTutorial()}
                >
                  <span className="codicon codicon-mortar-board" />
                  <span>Quick Tutorial</span>
                </div>

                <div
                  style={{
                    height: 1,
                    background: "rgba(255, 255, 255, 0.08)",
                    margin: "4px 6px",
                  }}
                />

                <div
                  className="menu-item"
                  style={{ justifyContent: "space-between" }}
                  onClick={() => openSettingsCategory("updates")}
                >
                  <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span className="codicon codicon-settings-gear" />
                    <span>Settings</span>
                  </span>
                  <span style={{ fontSize: 10.5, color: "var(--vsc-text-dim)" }}>
                    Ctrl+,
                  </span>
                </div>

                <div
                  className="menu-item"
                  style={{ justifyContent: "space-between" }}
                  onClick={() => openSettingsCategory("updates")}
                >
                  <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span className="codicon codicon-cloud-download" />
                    <span>Updates &amp; PSTDR LSP...</span>
                  </span>
                  {pstdrUpdateOffer?.update_available && (
                    <span
                      style={{
                        padding: "1px 5px",
                        borderRadius: 3,
                        background: "#163825",
                        color: "#7ee787",
                        fontSize: 10,
                        fontWeight: 600,
                      }}
                    >
                      v{pstdrUpdateOffer.version}
                    </span>
                  )}
                </div>

                <div
                  className="menu-item"
                  onClick={() => openSettingsCategory("ai-agents")}
                >
                  <span className="codicon codicon-sparkle" />
                  <span>AI Coding Agents...</span>
                </div>
              </div>
            )}
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
                    onRequestImport={(dir) => setImportModal({ dir })}
                    onOpenProjectPrompt={() => void promptOpenProject()}
                    onOpenNewWizard={() => setWizardOpen(true)}
                    onRefreshProject={() => void refreshActiveProject()}
                    onRevealPath={(rel) => void handleReveal(rel)}
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

                {activity === "extensions" && (
                  <ExtensionsSidebar
                    snapshot={extensionsSnapshot}
                    servers={lspServers}
                    onRefresh={() => void refreshExtensionsAndLsp()}
                    onInstallPath={(src) => {
                      void api
                        .extInstall(src)
                        .then((snap) => {
                          setExtensionsSnapshot(snap);
                          void refreshExtensionsAndLsp();
                          appendLog("success", `Installed extension from ${src}`);
                        })
                        .catch((e) => appendLog("error", `Extension install failed: ${String(e)}`));
                    }}
                    onToggleEnabled={(extId, enabled) => {
                      void api
                        .extSetEnabled(extId, enabled)
                        .then((snap) => {
                          setExtensionsSnapshot(snap);
                          void refreshExtensionsAndLsp();
                          appendLog(
                            "info",
                            `Extension '${extId}' ${enabled ? "enabled" : "disabled"}`,
                          );
                        })
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onUninstall={(extId) => {
                      void api
                        .extUninstall(extId)
                        .then((snap) => {
                          setExtensionsSnapshot(snap);
                          void refreshExtensionsAndLsp();
                          appendLog("info", `Uninstalled extension '${extId}'`);
                        })
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onStartServer={(srv) => {
                      void api
                        .lspStartServer(
                          srv.server_id,
                          srv.command,
                          srv.args,
                          srv.language_ids,
                        )
                        .then(() => {
                          void refreshExtensionsAndLsp();
                          appendLog("success", `Started LSP server: ${srv.display_name}`);
                        })
                        .catch((e) => appendLog("error", `LSP start failed: ${String(e)}`));
                    }}
                    onStopServer={(sid) => {
                      void api
                        .lspStopServer(sid)
                        .then(() => {
                          void refreshExtensionsAndLsp();
                          appendLog("info", `Stopped LSP server: ${sid}`);
                        })
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onRegisterCustomServer={(id, name, cmd, args, langs) => {
                      void api
                        .lspRegisterCustomServer(id, name, cmd, args, langs)
                        .then(() => {
                          void refreshExtensionsAndLsp();
                          appendLog("success", `Registered stdio LSP '${id}' (${cmd})`);
                        })
                        .catch((e) => appendLog("error", String(e)));
                    }}
                    onOpenExtensionTab={(extId) =>
                      openOrFocusTab({
                        id: "tool:extensions",
                        kind: "extensions",
                        title: "Extensions & LSP",
                        selectedExtensionId: extId,
                      })
                    }
                  />
                )}

                {activity === "settings" && (
                  <div className="sidebar-pane">
                    <div className="sidebar-header">
                      <span>SETTINGS</span>
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

                      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
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
                                    : tab.kind === "extensions"
                                      ? "codicon-extensions"
                                      : tab.kind === "settings"
                                        ? "codicon-settings-gear"
                                        : tab.kind === "tutorial"
                                          ? "codicon-mortar-board"
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
                      {(!activeTab || activeTab.kind === "tutorial") && (
                        <QuickTutorialView
                          recentProjects={recentProjects}
                          hasOpenProject={Boolean(project)}
                          onOpenFolder={() => void promptOpenProject()}
                          onOpenArchive={() => void promptOpenArchive()}
                          onOpenWizard={() => setWizardOpen(true)}
                          onOpenBlockbench={() => void handleOpenBlockbench()}
                          onOpenConverter={() =>
                            openOrFocusTab({
                              id: "tool:converter",
                              kind: "converter",
                              title: "Converter & Preflight",
                            })
                          }
                          onOpenPstdrStarter={() => void handleOpenPstdrStarter()}
                          onOpenSettings={(cat) => openSettingsCategory(cat)}
                          onOpenRecentProject={(path) => void openProjectByPath(path)}
                          onRemoveRecentProject={(path) =>
                            setRecentProjects((prev) =>
                              prev.filter((item) => item.path !== path),
                            )
                          }
                          onClearRecentProjects={() => setRecentProjects([])}
                        />
                      )}

                      {activeTab?.kind === "code" && activeTab.relPath && (
                        <ErrorBoundary
                          scope="editor-pane"
                          title={`Code editor crashed while rendering ${activeTab.relPath}`}
                          resetKey={activeTab.relPath}
                        >
                          <MonacoCodeTab
                            relPath={activeTab.relPath}
                            initialContent={textCache[activeTab.relPath] ?? ""}
                            revealLine={activeTab.revealLine}
                            problems={problems}
                            extensions={extensionsSnapshot}
                            lspDiagnostics={allLspDiagnostics}
                            onDirtyChange={(dirty) => setTabDirty(activeTab.id, dirty)}
                            onSave={async (rel, content) => {
                              if (!projectRef.current) {
                                setTextCache((c) => ({ ...c, [rel]: content }));
                                showToast(
                                  "Open or create a PackStudio project to save this file to disk.",
                                );
                                return;
                              }
                              const snap = await api.fsWriteText(rel, content);
                              setTextCache((c) => ({ ...c, [rel]: content }));
                              setProject(snap);
                              appendLog("success", `Saved ${rel}`);
                            }}
                            onOpenBlockbench={(rel) =>
                              void handleOpenBlockbench(rel)
                            }
                          />
                        </ErrorBoundary>
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

                      {activeTab?.kind === "settings" && (
                        <SettingsView
                          initialCategory={settingsCategory}
                          categoryNonce={settingsNavNonce}
                          project={project}
                          activeFilePath={
                            tabs.find((t) => t.kind === "code" && t.relPath)?.relPath ??
                            selectedRelPath
                          }
                          diagnostics={mergedProblems}
                          studioSettings={settings}
                          fileManagers={fileManagers}
                          pstdrStatus={pstdrSyncStatus}
                          pstdrOffer={pstdrUpdateOffer}
                          githubStatus={githubStatus}
                          aiAgentsStatus={aiAgentsStatus}
                          onStudioSettingsChange={setSettings}
                          onPstdrStatusChange={(next) => {
                            setPstdrSyncStatus(next);
                            void refreshExtensionsAndLsp();
                          }}
                          onOpenPstdrUpdateModal={() => setPstdrUpdateModalOpen(true)}
                          onGithubStatusChange={setGithubStatus}
                          onAiAgentsStatusChange={setAiAgentsStatus}
                          onLog={appendLog}
                        />
                      )}

                      {activeTab?.kind === "extensions" && (
                        <ExtensionsTab
                          snapshot={extensionsSnapshot}
                          servers={lspServers}
                          selectedExtensionId={activeTab.selectedExtensionId}
                          lspDiagCount={allLspDiagnostics.length}
                          onSelectExtension={(extId) =>
                            openOrFocusTab({
                              id: "tool:extensions",
                              kind: "extensions",
                              title: "Extensions & LSP",
                              selectedExtensionId: extId,
                            })
                          }
                          onInstallPath={(src) => {
                            void api
                              .extInstall(src)
                              .then((snap) => {
                                setExtensionsSnapshot(snap);
                                void refreshExtensionsAndLsp();
                                appendLog("success", `Installed extension from ${src}`);
                              })
                              .catch((e) =>
                                appendLog("error", `Extension install failed: ${String(e)}`),
                              );
                          }}
                          onToggleEnabled={(extId, enabled) => {
                            void api
                              .extSetEnabled(extId, enabled)
                              .then((snap) => {
                                setExtensionsSnapshot(snap);
                                void refreshExtensionsAndLsp();
                                appendLog(
                                  "info",
                                  `Extension '${extId}' ${enabled ? "enabled" : "disabled"}`,
                                );
                              })
                              .catch((e) => appendLog("error", String(e)));
                          }}
                          onUninstall={(extId) => {
                            void api
                              .extUninstall(extId)
                              .then((snap) => {
                                setExtensionsSnapshot(snap);
                                void refreshExtensionsAndLsp();
                                appendLog("info", `Uninstalled extension '${extId}'`);
                              })
                              .catch((e) => appendLog("error", String(e)));
                          }}
                          onStartServer={(srv) => {
                            void api
                              .lspStartServer(
                                srv.server_id,
                                srv.command,
                                srv.args,
                                srv.language_ids,
                              )
                              .then(() => {
                                void refreshExtensionsAndLsp();
                                appendLog("success", `Started LSP server: ${srv.display_name}`);
                              })
                              .catch((e) => appendLog("error", `LSP start failed: ${String(e)}`));
                          }}
                          onStopServer={(sid) => {
                            void api
                              .lspStopServer(sid)
                              .then(() => {
                                void refreshExtensionsAndLsp();
                                appendLog("info", `Stopped LSP server: ${sid}`);
                              })
                              .catch((e) => appendLog("error", String(e)));
                          }}
                          onRegisterCustomServer={(id, name, cmd, args, langs) => {
                            void api
                              .lspRegisterCustomServer(id, name, cmd, args, langs)
                              .then(() => {
                                void refreshExtensionsAndLsp();
                                appendLog("success", `Registered stdio LSP '${id}' (${cmd})`);
                              })
                              .catch((e) => appendLog("error", String(e)));
                          }}
                          onUnregisterCustomServer={(sid) => {
                            void api
                              .lspUnregisterCustomServer(sid)
                              .then(() => {
                                void refreshExtensionsAndLsp();
                                appendLog("info", `Removed custom LSP '${sid}'`);
                              })
                              .catch((e) => appendLog("error", String(e)));
                          }}
                        />
                      )}

                      {blockbenchTab && blockbenchTab.blockbenchUrl && (
                          <div
                            className={`blockbench-shell ${
                              activeTab?.kind === "blockbench" ? "" : "is-hidden"
                            }`}
                            aria-hidden={activeTab?.kind !== "blockbench"}
                          >
                            <div className="code-toolbar">
                              <div className="toolbar-group">
                                <span
                                  className="codicon codicon-cube"
                                  style={{ color: "#3794ff" }}
                                />
                                <strong>Blockbench</strong>
                                <span
                                  className="path-hint"
                                  style={{ maxWidth: 260 }}
                                  title={blockbenchTab.blockbenchUrl}
                                >
                                  {blockbenchTab.blockbenchUrl}
                                </span>
                              </div>
                              <div className="toolbar-group">
                                <div
                                  className="zoom-control"
                                  title="Blockbench interface scale"
                                >
                                  <button
                                    className="icon-btn"
                                    title="Zoom out"
                                    onClick={() =>
                                      setSettings((s) => ({
                                        ...s,
                                        blockbenchUiScale: Math.max(
                                          0.6,
                                          Math.round((s.blockbenchUiScale - 0.04) * 100) / 100,
                                        ),
                                      }))
                                    }
                                  >
                                    <span className="codicon codicon-zoom-out" />
                                  </button>
                                  <span className="zoom-readout">
                                    {Math.round(settings.blockbenchUiScale * 100)}%
                                  </span>
                                  <button
                                    className="icon-btn"
                                    title="Zoom in"
                                    onClick={() =>
                                      setSettings((s) => ({
                                        ...s,
                                        blockbenchUiScale: Math.min(
                                          1.2,
                                          Math.round((s.blockbenchUiScale + 0.04) * 100) / 100,
                                        ),
                                      }))
                                    }
                                  >
                                    <span className="codicon codicon-zoom-in" />
                                  </button>
                                  <button
                                    className="icon-btn"
                                    title="Reset zoom to 100%"
                                    onClick={() =>
                                      setSettings((s) => ({
                                        ...s,
                                        blockbenchUiScale: 1,
                                      }))
                                    }
                                  >
                                    <span className="codicon codicon-refresh" />
                                  </button>
                                </div>
                                <button
                                  className="vsc-btn secondary small"
                                  title="Open Blockbench in your browser"
                                  onClick={() => void openUrl(blockbenchTab.blockbenchUrl!)}
                                >
                                  <span className="codicon codicon-open-preview" />
                                  Pop out
                                </button>
                                <button
                                  className="vsc-btn secondary small"
                                  disabled={!project}
                                  onClick={() =>
                                    void api
                                      .blockbenchNewBedrockGeo(
                                        project?.pack_type === "bedrock_addon"
                                          ? "RP/models/entity/custom_model.geo.json"
                                          : "models/entity/custom_model.geo.json",
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
                                  <span className="codicon codicon-add" />
                                  New .geo.json
                                </button>
                              </div>
                            </div>
                            <div className="blockbench-frame-wrap">
                              <iframe
                                ref={blockbenchFrameRef}
                                title="Blockbench 3D Studio"
                                src={blockbenchTab.blockbenchUrl}
                                className="blockbench-frame"
                                style={{
                                  width: `${100 / settings.blockbenchUiScale}%`,
                                  height: `${100 / settings.blockbenchUiScale}%`,
                                  transform: `scale(${settings.blockbenchUiScale})`,
                                }}
                                onLoad={() => applyBlockbenchScale()}
                              />
                            </div>
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
                            PROBLEMS ({mergedProblems.length})
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
                            TERMINAL
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
                            {mergedProblems.length === 0 ? (
                              <div
                                style={{
                                  padding: 12,
                                  color: "var(--vsc-text-dim)",
                                }}
                              >
                                No problems reported by LSP or Semantic Auditor. Click
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
                                  {mergedProblems.map((p, idx) => (
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
              ? `${packTypeBadgeLabel(project.pack_type)} • ${project.edition_label} (${project.dominant_resolution ?? "Native"})`
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
          {pstdrUpdateOffer?.update_available && (
            <span
              className="status-item"
              onClick={() => setPstdrUpdateModalOpen(true)}
              title={`Verified PSTDR update v${pstdrUpdateOffer.version} available — click to review fingerprints`}
              style={{ color: "#7ee787", fontWeight: 600 }}
            >
              <span className="codicon codicon-shield" />
              PSTDR v{pstdrUpdateOffer.version}
            </span>
          )}
          <span
            className="status-item"
            onClick={() => {
              setActivity("extensions");
              setSidebarOpen(true);
              openOrFocusTab({
                id: "tool:extensions",
                kind: "extensions",
                title: "Extensions & LSP",
              });
            }}
            title="Manage Extensions & Language Servers"
          >
            <span className="codicon codicon-extensions" />
            LSP: {runningLspCount}/{lspServers.length}
          </span>
          {fileManagers && (
            <span
              className="status-item"
              onClick={() => {
                setActivity("settings");
                setSidebarOpen(true);
              }}
            >
              <span className="codicon codicon-folder" />
              {fileManagers.primary}
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
            Terminal
          </span>
        </div>
      </div>

      {wizardOpen && (
        <NewProjectWizardModal
          onClose={() => setWizardOpen(false)}
          onCreate={async (cfg: NewProjectConfig) => {
            const snap = await api.createProject(cfg);
            setProject(snap);
            recordRecentProject(snap);
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

      {pstdrUpdateModalOpen && (
        <PstdrUpdateModal
          status={pstdrSyncStatus}
          initialOffer={pstdrUpdateOffer}
          onClose={() => setPstdrUpdateModalOpen(false)}
          onStatusChanged={(next) => {
            setPstdrSyncStatus(next);
            void refreshExtensionsAndLsp();
          }}
          onOfferChanged={(next) => setPstdrUpdateOffer(next)}
          onLog={appendLog}
        />
      )}

      {importModal && project && (
        <ImportAssetsModal
          project={project}
          initialTargetDir={importModal.dir}
          autoSanitize={settings.autoSanitizeImports}
          convertImagesToPng={settings.convertImportedImagesToPng}
          onToggleAutoSanitize={(value) =>
            setSettings((s) => ({ ...s, autoSanitizeImports: value }))
          }
          onToggleConvertImagesToPng={(value) =>
            setSettings((s) => ({ ...s, convertImportedImagesToPng: value }))
          }
          onClose={() => setImportModal(null)}
          onImport={handleImportPaths}
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
