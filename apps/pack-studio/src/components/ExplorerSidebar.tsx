import React, { useEffect, useMemo, useRef, useState } from "react";
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
  onRequestImport: (targetDir: string) => void;
  onOpenProjectPrompt: () => void;
  onOpenNewWizard: () => void;
  onRefreshProject: () => void;
  onRevealPath: (relPath: string) => void;
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
  onRequestImport,
  onOpenProjectPrompt,
  onOpenNewWizard,
  onRefreshProject,
  onRevealPath,
  onToast,
}) => {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    assets: true,
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
  const [newMenu, setNewMenu] = useState<{ x: number; y: number } | null>(null);
  const newButtonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    const close = () => {
      setContextMenu(null);
      setNewMenu(null);
    };
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

  const startInline = (kind: "new_file" | "new_dir", parentDir: string) => {
    setInlineValue("");
    setInlineMode({ kind, parentDir });
    if (parentDir) {
      setExpanded((prev) => ({ ...prev, [parentDir]: true }));
      onSelectPath(parentDir);
    }
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

  const openNewMenu = () => {
    const rect = newButtonRef.current?.getBoundingClientRect();
    if (!rect) return;
    setNewMenu({ x: rect.left - 140, y: rect.bottom + 4 });
  };

  const renderInlineInput = (_parentDir: string, depth: number) => (
    <div className="tree-row" style={{ paddingLeft: `${10 + depth * 13}px` }}>
      <span
        className={`codicon ${
          inlineMode?.kind === "new_dir" ? "codicon-folder" : "codicon-new-file"
        }`}
      />
      <input
        className="vsc-input"
        style={{ flex: 1, padding: "1px 5px", height: 20 }}
        autoFocus
        placeholder={
          inlineMode?.kind === "new_dir" ? "folder_name" : "asset_name.json"
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
  );

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
            style={{ paddingLeft: `${7 + depth * 13}px` }}
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
                style={{ fontSize: 11, color: "#8f96a3" }}
              />
            ) : (
              <span style={{ width: 11, flexShrink: 0 }} />
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
                inlineMode.parentDir === node.rel_path &&
                renderInlineInput(node.rel_path, depth + 1)}
              {renderNodes(node.children, depth + 1)}
            </>
          )}
        </React.Fragment>
      );
    });

  return (
    <div className="sidebar-pane">
      <div className="sidebar-header">
        <span className="sidebar-title">
          Explorer
          {project ? (
            <span className="sidebar-subtitle">{project.pack_name}</span>
          ) : null}
        </span>
        <div className="sidebar-actions">
          <button
            ref={newButtonRef}
            className={`icon-btn ${newMenu ? "active" : ""}`}
            title="New file, folder or texture"
            disabled={!project}
            onClick={(e) => {
              e.stopPropagation();
              if (newMenu) setNewMenu(null);
              else openNewMenu();
            }}
          >
            <span className="codicon codicon-add" />
          </button>
          <button
            className="icon-btn"
            title="Import assets into the selected folder"
            disabled={!project}
            onClick={() => onRequestImport(activeTargetDir)}
          >
            <span className="codicon codicon-cloud-upload" />
          </button>
          <button
            className="icon-btn"
            title="Refresh explorer"
            disabled={!project}
            onClick={onRefreshProject}
          >
            <span className="codicon codicon-refresh" />
          </button>
        </div>
      </div>

      {!project ? (
        <div className="panel-scroll" style={{ padding: 16 }}>
          <div className="empty-state" style={{ padding: "18px 6px" }}>
            <span className="codicon codicon-files" />
            <h3>No pack open</h3>
            <p>
              Open a resource pack folder, a .zip / .mcpack, or create a new
              project.
            </p>
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: 7,
                width: "100%",
                marginTop: 4,
              }}
            >
              <button className="vsc-btn" onClick={onOpenProjectPrompt}>
                <span className="codicon codicon-folder-opened" />
                Open folder or archive
              </button>
              <button className="vsc-btn secondary" onClick={onOpenNewWizard}>
                <span className="codicon codicon-add" />
                New resource pack
              </button>
            </div>
            <p style={{ marginTop: 8 }}>
              Tip: you can also drag a folder or .zip onto the window.
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="tree-filter-bar">
            <div className="filter-field">
              <span className="codicon codicon-search" />
              <input
                className="vsc-input"
                placeholder="Filter assets..."
                value={filterText}
                spellCheck={false}
                onChange={(e) => onFilterTextChange(e.target.value)}
              />
              {filterText && (
                <button
                  className="icon-btn filter-clear"
                  title="Clear filter"
                  onClick={() => onFilterTextChange("")}
                >
                  <span className="codicon codicon-close" />
                </button>
              )}
            </div>
          </div>

          <div className="tree-scroll">
            {inlineMode &&
              (inlineMode.kind === "new_file" || inlineMode.kind === "new_dir") &&
              inlineMode.parentDir === "" &&
              renderInlineInput("", 0)}
            {renderNodes(filteredNodes, 0)}
            {filteredNodes.length === 0 && (
              <div className="tree-empty">
                No files match “{filterText}”.
              </div>
            )}
          </div>

          <div className="explorer-footer" title="Drag files onto the workbench to import them">
            <span className="codicon codicon-cloud-upload" />
            Drop files anywhere to import
          </div>
        </>
      )}

      {newMenu && project && (
        <div
          className="menu-popover"
          style={{ left: Math.max(8, newMenu.x), top: newMenu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="menu-heading">Create</div>
          <div
            className="menu-item"
            onClick={() => {
              startInline("new_file", activeTargetDir);
              setNewMenu(null);
            }}
          >
            <span className="codicon codicon-new-file" />
            New file
          </div>
          <div
            className="menu-item"
            onClick={() => {
              startInline("new_dir", activeTargetDir);
              setNewMenu(null);
            }}
          >
            <span className="codicon codicon-new-folder" />
            New folder
          </div>
          <div
            className="menu-item"
            onClick={() => {
              onCreateBlankTexture(activeTargetDir);
              setNewMenu(null);
            }}
          >
            <span className="codicon codicon-file-media" />
            Blank PNG texture
          </div>
          <div className="menu-sep" />
          <div
            className="menu-item"
            onClick={() => {
              onRequestImport(activeTargetDir);
              setNewMenu(null);
            }}
          >
            <span className="codicon codicon-cloud-upload" />
            Import assets here
          </div>
          <div
            className="menu-item"
            onClick={() => {
              setExpanded({});
              setNewMenu(null);
            }}
          >
            <span className="codicon codicon-collapse-all" />
            Collapse all folders
          </div>
        </div>
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
                Open in editor
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
                  Edit in 2D texture paint
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
                  Edit in Blockbench
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
                  Open as JSON source
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
                  startInline("new_file", target);
                }}
              >
                <span className="codicon codicon-new-file" />
                New file here
              </div>
              <div
                className="context-item"
                onClick={() => {
                  const target = contextMenu.node.rel_path;
                  setContextMenu(null);
                  startInline("new_dir", target);
                }}
              >
                <span className="codicon codicon-new-folder" />
                New folder here
              </div>
              <div
                className="context-item"
                onClick={() => {
                  const target = contextMenu.node.rel_path;
                  setContextMenu(null);
                  onRequestImport(target);
                }}
              >
                <span className="codicon codicon-cloud-upload" />
                Import assets here
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
              onToast(`Copied resource ID: ${id}`);
              setContextMenu(null);
            }}
          >
            <span className="codicon codicon-copy" />
            Copy Minecraft resource ID
          </div>

          <div
            className="context-item"
            onClick={() => {
              onRevealPath(contextMenu.node.rel_path);
              setContextMenu(null);
            }}
          >
            <span className="codicon codicon-folder-opened" />
            Reveal in file manager
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
