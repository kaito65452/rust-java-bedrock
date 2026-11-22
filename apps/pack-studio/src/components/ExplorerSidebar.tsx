import React, { useEffect, useMemo, useState } from "react";
import type {
  ConversionWarning,
  FileTreeNodeDto,
  ProjectSnapshot,
} from "../types";
import {
  computeResourceId,
  fileIconClass,
  isImageAsset,
  isModelAsset,
} from "../utils/mcFormat";

interface ExplorerSidebarProps {
  project: ProjectSnapshot | null;
  selectedRelPath: string | null;
  problems: ConversionWarning[];
  filterText: string;
  onFilterTextChange: (val: string) => void;
  onSelectPath: (relPath: string) => void;
  onOpenAsset: (relPath: string, forceMode?: "code" | "texture") => void;
  onOpenBlockbenchAsset: (relPath: string) => void;
  onCreateFile: (targetRelPath: string) => void;
  onCreateDir: (targetRelPath: string) => void;
  onCreateBlankTexture: (targetDir: string) => void;
  onRenameNode: (oldRelPath: string, newName: string, isDir: boolean) => void;
  onDeleteNode: (relPath: string) => void;
  onImportFiles: (targetDir: string) => void;
  onOpenProjectPrompt: () => void;
  onOpenNewWizard: () => void;
  onRefreshProject: () => void;
  onRevealPath: (relPath: string) => void;
  onLaunchYazi: (relPath?: string) => void;
  onToast: (msg: string) => void;
}

interface ContextMenuState {
  x: number;
  y: number;
  node: FileTreeNodeDto;
}

function filterTree(
  nodes: FileTreeNodeDto[],
  query: string,
): FileTreeNodeDto[] {
  const q = query.trim().toLowerCase();
  if (!q) return nodes;
  const out: FileTreeNodeDto[] = [];
  for (const node of nodes) {
    if (node.is_dir) {
      const filteredChildren = filterTree(node.children, q);
      if (
        filteredChildren.length > 0 ||
        node.name.toLowerCase().includes(q)
      ) {
        out.push({ ...node, children: filteredChildren });
      }
    } else if (
      node.name.toLowerCase().includes(q) ||
      node.rel_path.toLowerCase().includes(q)
    ) {
      out.push(node);
    }
  }
  return out;
}

export const ExplorerSidebar: React.FC<ExplorerSidebarProps> = ({
  project,
  selectedRelPath,
  problems,
  filterText,
  onFilterTextChange,
  onSelectPath,
  onOpenAsset,
  onOpenBlockbenchAsset,
  onCreateFile,
  onCreateDir,
  onCreateBlankTexture,
  onRenameNode,
  onDeleteNode,
  onImportFiles,
  onOpenProjectPrompt,
  onOpenNewWizard,
  onRefreshProject,
  onRevealPath,
  onLaunchYazi,
  onToast,
}) => {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    assets: true,
    "assets/minecraft": true,
    textures: true,
    models: true,
  });
  const [inlineMode, setInlineMode] = useState<
    | null
    | { kind: "new_file"; parentDir: string }
    | { kind: "new_dir"; parentDir: string }
    | { kind: "rename"; node: FileTreeNodeDto }
  >(null);
  const [inlineValue, setInlineValue] = useState("");
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);

  useEffect(() => {
    const close = () => setContextMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, []);

  const severityByPath = useMemo(() => {
    const map = new Map<string, "error" | "warning">();
    for (const p of problems) {
      const path = (p.source_path || p.asset || "").replace(/\\/g, "/");
      if (!path) continue;
      if (p.severity === "error") {
        map.set(path, "error");
      } else if (p.severity === "warning" && map.get(path) !== "error") {
        map.set(path, "warning");
      }
    }
    return map;
  }, [problems]);

  const activeTargetDir = useMemo(() => {
    if (!project || !selectedRelPath) return "";
    if (!selectedRelPath.includes("/")) {
      return project.all_files.includes(selectedRelPath) ? "" : selectedRelPath;
    }
    const isFile = project.all_files.includes(selectedRelPath);
    if (!isFile) return selectedRelPath;
    const idx = selectedRelPath.lastIndexOf("/");
    return idx >= 0 ? selectedRelPath.slice(0, idx) : "";
  }, [project, selectedRelPath]);

  const filteredNodes = useMemo(
    () => (project ? filterTree(project.tree, filterText) : []),
    [project, filterText],
  );

  const toggleExpand = (relPath: string) => {
    setExpanded((prev) => ({ ...prev, [relPath]: !prev[relPath] }));
  };

  const commitInline = () => {
    const val = inlineValue.trim();
    if (!inlineMode || !val) {
      setInlineMode(null);
      return;
    }
    if (inlineMode.kind === "new_file") {
      const target = inlineMode.parentDir
        ? `${inlineMode.parentDir}/${val}`
        : val;
      onCreateFile(target);
    } else if (inlineMode.kind === "new_dir") {
      const target = inlineMode.parentDir
        ? `${inlineMode.parentDir}/${val}`
        : val;
      onCreateDir(target);
      setExpanded((prev) => ({ ...prev, [target]: true }));
    } else if (inlineMode.kind === "rename") {
      onRenameNode(inlineMode.node.rel_path, val, inlineMode.node.is_dir);
    }
    setInlineMode(null);
    setInlineValue("");
  };

  const renderNodes = (nodes: FileTreeNodeDto[], depth: number) =>
    nodes.map((node) => {
      const isExpanded =
        filterText.trim().length > 0 ? true : !!expanded[node.rel_path];
      const isSelected = selectedRelPath === node.rel_path;
      const sev = !node.is_dir ? severityByPath.get(node.rel_path) : undefined;
      const isRenamingHere =
        inlineMode?.kind === "rename" &&
        inlineMode.node.rel_path === node.rel_path;

      return (
        <React.Fragment key={node.rel_path}>
          <div
            className={`tree-row ${isSelected ? "selected" : ""}`}
            style={{ paddingLeft: `${8 + depth * 14}px` }}
            onClick={() => {
              onSelectPath(node.rel_path);
              if (node.is_dir) {
                toggleExpand(node.rel_path);
              } else {
                onOpenAsset(node.rel_path);
              }
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              onSelectPath(node.rel_path);
              setContextMenu({ x: e.clientX, y: e.clientY, node });
            }}
            title={node.rel_path}
          >
            {node.is_dir ? (
              <span
                className={`codicon ${
                  isExpanded ? "codicon-chevron-down" : "codicon-chevron-right"
                }`}
                style={{ fontSize: 12, color: "#9da5b4" }}
              />
            ) : (
              <span style={{ width: 12 }} />
            )}
            <span
              className={`codicon ${fileIconClass(
                node.rel_path,
                node.is_dir,
                isExpanded,
              )}`}
              style={{
                color: node.is_dir
                  ? "#dcb67a"
                  : isImageAsset(node.rel_path)
                    ? "#c586c0"
                    : node.rel_path.endsWith(".json") ||
                        node.rel_path.endsWith(".mcmeta")
                      ? "#f5b83d"
                      : "#9cdcfe",
              }}
            />
            {isRenamingHere ? (
              <input
                className="vsc-input"
                style={{ flex: 1, padding: "1px 5px", height: 20 }}
                autoFocus
                value={inlineValue}
                onChange={(e) => setInlineValue(e.target.value)}
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitInline();
                  if (e.key === "Escape") setInlineMode(null);
                }}
                onBlur={commitInline}
              />
            ) : (
              <span className="tree-label">{node.name}</span>
            )}
            {sev && (
              <span
                className={`prob-dot ${sev}`}
                title={`Semantic Auditor: ${sev}`}
              />
            )}
          </div>

          {node.is_dir && isExpanded && (
            <>
              {inlineMode &&
                (inlineMode.kind === "new_file" ||
                  inlineMode.kind === "new_dir") &&
                inlineMode.parentDir === node.rel_path && (
                  <div
                    className="tree-row"
                    style={{ paddingLeft: `${22 + depth * 14}px` }}
                  >
                    <span
                      className={`codicon ${
                        inlineMode.kind === "new_dir"
                          ? "codicon-folder"
                          : "codicon-new-file"
                      }`}
                    />
                    <input
                      className="vsc-input"
                      style={{ flex: 1, padding: "1px 5px", height: 20 }}
                      autoFocus
                      placeholder={
                        inlineMode.kind === "new_dir"
                          ? "folder_name"
                          : "asset_name.json"
                      }
                      value={inlineValue}
                      onChange={(e) => setInlineValue(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") commitInline();
                        if (e.key === "Escape") setInlineMode(null);
                      }}
                      onBlur={commitInline}
                    />
                  </div>
                )}
              {renderNodes(node.children, depth + 1)}
            </>
          )}
        </React.Fragment>
      );
    });

  return (
    <div className="sidebar-pane">
      <div className="sidebar-header">
        <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {project ? project.pack_name : "EXPLORER"}
        </span>
        <div className="sidebar-actions">
          <button
            className="icon-btn"
            title="New File in Selection"
            disabled={!project}
            onClick={() => {
              setInlineValue("");
              setInlineMode({ kind: "new_file", parentDir: activeTargetDir });
              if (activeTargetDir) {
                setExpanded((prev) => ({ ...prev, [activeTargetDir]: true }));
              }
            }}
          >
            <span className="codicon codicon-new-file" />
          </button>
          <button
            className="icon-btn"
            title="New Folder in Selection"
            disabled={!project}
            onClick={() => {
              setInlineValue("");
              setInlineMode({ kind: "new_dir", parentDir: activeTargetDir });
              if (activeTargetDir) {
                setExpanded((prev) => ({ ...prev, [activeTargetDir]: true }));
              }
            }}
          >
            <span className="codicon codicon-new-folder" />
          </button>
          <button
            className="icon-btn"
            title="Import Assets (Sanitized / PNG Convert)"
            disabled={!project}
            onClick={() => onImportFiles(activeTargetDir)}
          >
            <span className="codicon codicon-cloud-upload" />
          </button>
          <button
            className="icon-btn"
            title="New Blank Texture (PNG)"
            disabled={!project}
            onClick={() => onCreateBlankTexture(activeTargetDir)}
          >
            <span className="codicon codicon-file-media" />
          </button>
          <button
            className="icon-btn"
            title="Refresh Explorer"
            disabled={!project}
            onClick={onRefreshProject}
          >
            <span className="codicon codicon-refresh" />
          </button>
          <button
            className="icon-btn"
            title="Open Folder / Archive..."
            onClick={onOpenProjectPrompt}
          >
            <span className="codicon codicon-folder-opened" />
          </button>
        </div>
      </div>

      {!project ? (
        <div className="panel-scroll" style={{ padding: 14 }}>
          <p style={{ color: "var(--vsc-text-dim)", marginTop: 0 }}>
            No resource pack open yet. Open an existing folder / <code>.zip</code> / <code>.mcpack</code> or scaffold a fresh workspace.
          </p>
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <button className="vsc-btn" onClick={onOpenProjectPrompt}>
              <span className="codicon codicon-folder-opened" />
              Open Pack Folder / Archive
            </button>
            <button className="vsc-btn secondary" onClick={onOpenNewWizard}>
              <span className="codicon codicon-add" />
              New Resource Pack Wizard...
            </button>
            <button
              className="vsc-btn secondary"
              onClick={() => onLaunchYazi()}
            >
              <span className="codicon codicon-terminal" />
              Browse with Yazi (TUI)
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="tree-filter-bar">
            <input
              className="vsc-input"
              style={{ flex: 1, fontSize: 12 }}
              placeholder="Filter assets by path..."
              value={filterText}
              onChange={(e) => onFilterTextChange(e.target.value)}
            />
            {filterText && (
              <button
                className="icon-btn"
                title="Clear Filter"
                onClick={() => onFilterTextChange("")}
              >
                <span className="codicon codicon-close" />
              </button>
            )}
          </div>

          <div className="tree-scroll">
            {inlineMode &&
              (inlineMode.kind === "new_file" || inlineMode.kind === "new_dir") &&
              inlineMode.parentDir === "" && (
                <div className="tree-row" style={{ paddingLeft: "20px" }}>
                  <span
                    className={`codicon ${
                      inlineMode.kind === "new_dir"
                        ? "codicon-folder"
                        : "codicon-new-file"
                    }`}
                  />
                  <input
                    className="vsc-input"
                    style={{ flex: 1, padding: "1px 5px", height: 20 }}
                    autoFocus
                    placeholder={
                      inlineMode.kind === "new_dir"
                        ? "folder_name"
                        : "new_file.json"
                    }
                    value={inlineValue}
                    onChange={(e) => setInlineValue(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitInline();
                      if (e.key === "Escape") setInlineMode(null);
                    }}
                    onBlur={commitInline}
                  />
                </div>
              )}
            {renderNodes(filteredNodes, 0)}
          </div>
        </>
      )}

      {contextMenu && project && (
        <div
          className="context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          {!contextMenu.node.is_dir && (
            <>
              <div
                className="context-item"
                onClick={() => {
                  onOpenAsset(contextMenu.node.rel_path);
                  setContextMenu(null);
                }}
              >
                <span className="codicon codicon-go-to-file" />
                Open in Editor
              </div>
              {isImageAsset(contextMenu.node.rel_path) && (
                <div
                  className="context-item"
                  onClick={() => {
                    onOpenAsset(contextMenu.node.rel_path, "texture");
                    setContextMenu(null);
                  }}
                >
                  <span className="codicon codicon-paintcan" />
                  Edit in 2D Texture Paint
                </div>
              )}
              {(isModelAsset(contextMenu.node.rel_path) ||
                isImageAsset(contextMenu.node.rel_path)) && (
                <div
                  className="context-item"
                  onClick={() => {
                    onOpenBlockbenchAsset(contextMenu.node.rel_path);
                    setContextMenu(null);
                  }}
                >
                  <span className="codicon codicon-preview" />
                  Edit in Blockbench (3D & Paint)
                </div>
              )}
              {contextMenu.node.rel_path.endsWith(".json") ||
              contextMenu.node.rel_path.endsWith(".mcmeta") ? (
                <div
                  className="context-item"
                  onClick={() => {
                    onOpenAsset(contextMenu.node.rel_path, "code");
                    setContextMenu(null);
                  }}
                >
                  <span className="codicon codicon-code" />
                  Open as JSON Source
                </div>
              ) : null}
              <div className="context-sep" />
            </>
          )}

          {contextMenu.node.is_dir && (
            <>
              <div
                className="context-item"
                onClick={() => {
                  const target = contextMenu.node.rel_path;
                  setContextMenu(null);
                  setInlineValue("");
                  setInlineMode({ kind: "new_file", parentDir: target });
                  setExpanded((prev) => ({ ...prev, [target]: true }));
                }}
              >
                <span className="codicon codicon-new-file" />
                New File Here...
              </div>
              <div
                className="context-item"
                onClick={() => {
                  const target = contextMenu.node.rel_path;
                  setContextMenu(null);
                  onImportFiles(target);
                }}
              >
                <span className="codicon codicon-cloud-upload" />
                Import Assets Here...
              </div>
              <div className="context-sep" />
            </>
          )}

          <div
            className="context-item"
            onClick={() => {
              const id = computeResourceId(
                contextMenu.node.rel_path,
                project.edition,
              );
              void navigator.clipboard.writeText(id);
              onToast(`Copied Resource ID: ${id}`);
              setContextMenu(null);
            }}
          >
            <span className="codicon codicon-copy" />
            Copy Minecraft Resource ID
          </div>

          <div
            className="context-item"
            onClick={() => {
              onRevealPath(contextMenu.node.rel_path);
              setContextMenu(null);
            }}
          >
            <span className="codicon codicon-folder-opened" />
            Reveal in File Manager
          </div>

          <div
            className="context-item"
            onClick={() => {
              onLaunchYazi(contextMenu.node.rel_path);
              setContextMenu(null);
            }}
          >
            <span className="codicon codicon-terminal" />
            Open Folder in Yazi (PTY)
          </div>

          <div className="context-sep" />

          <div
            className="context-item"
            onClick={() => {
              setInlineValue(contextMenu.node.name);
              setInlineMode({ kind: "rename", node: contextMenu.node });
              setContextMenu(null);
            }}
          >
            <span className="codicon codicon-edit" />
            Rename
          </div>

          <div
            className="context-item danger"
            onClick={() => {
              const target = contextMenu.node.rel_path;
              setContextMenu(null);
              onDeleteNode(target);
            }}
          >
            <span className="codicon codicon-trash" />
            Delete
          </div>
        </div>
      )}
    </div>
  );
};
