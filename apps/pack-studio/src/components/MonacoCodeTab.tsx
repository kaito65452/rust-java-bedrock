import React, { useEffect, useMemo, useRef, useState } from "react";
import Editor, { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import type { ConversionWarning } from "../types";

// Configure Monaco offline workers inside Tauri bundle (no external CDN requests).
if (typeof self !== "undefined") {
  const g = self as unknown as {
    MonacoEnvironment?: {
      getWorker: (workerId: string, label: string) => Worker;
    };
  };
  if (!g.MonacoEnvironment) {
    g.MonacoEnvironment = {
      getWorker(_workerId: string, label: string) {
        if (label === "json") {
          return new jsonWorker();
        }
        return new editorWorker();
      },
    };
  }
}

loader.config({ monaco });

let customLanguagesRegistered = false;

function registerMinecraftLanguages(m: typeof monaco) {
  if (customLanguagesRegistered) return;
  customLanguagesRegistered = true;

  m.languages.register({ id: "mc-lang" });
  m.languages.setMonarchTokensProvider("mc-lang", {
    tokenizer: {
      root: [
        [/##.*$/, "comment"],
        [/§[0-9a-gk-or]/i, "keyword"],
        [/^[^=\r\n]+(?==)/, "attribute.name"],
        [/=/, "delimiter"],
        [/[^\r\n§]+/, "string"],
      ],
    },
  });

  m.languages.register({ id: "molang" });
  m.languages.setMonarchTokensProvider("molang", {
    tokenizer: {
      root: [
        [/\/\/.*$/, "comment"],
        [/\b(query|q|variable|v|temp|t|context|c|math)\.[a-zA-Z0-9_]+/, "type.identifier"],
        [/\b(return|loop|for_each|break|continue|true|false)\b/, "keyword"],
        [/-?\d+(\.\d+)?/, "number"],
        [/'[^']*'/, "string"],
      ],
    },
  });
}

function detectMonacoLanguage(relPath: string): string {
  const lower = relPath.toLowerCase();
  if (lower.endsWith(".json") || lower.endsWith(".mcmeta") || lower.endsWith(".material")) {
    return "json";
  }
  if (lower.endsWith(".lang")) return "mc-lang";
  if (lower.endsWith(".molang")) return "molang";
  if (lower.endsWith(".md")) return "markdown";
  return "plaintext";
}

interface MonacoCodeTabProps {
  relPath: string;
  initialContent: string;
  revealLine?: number;
  problems: ConversionWarning[];
  onDirtyChange: (dirty: boolean) => void;
  onSave: (relPath: string, content: string) => Promise<void>;
  onOpenBlockbench?: (relPath: string) => void;
}

export const MonacoCodeTab: React.FC<MonacoCodeTabProps> = ({
  relPath,
  initialContent,
  revealLine,
  problems,
  onDirtyChange,
  onSave,
  onOpenBlockbench,
}) => {
  const [content, setContent] = useState(initialContent);
  const [savedContent, setSavedContent] = useState(initialContent);
  const [saving, setSaving] = useState(false);
  const [jsonError, setJsonError] = useState<string | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);

  useEffect(() => {
    setContent(initialContent);
    setSavedContent(initialContent);
    onDirtyChange(false);
  }, [relPath, initialContent]);

  const lang = useMemo(() => detectMonacoLanguage(relPath), [relPath]);
  const isDirty = content !== savedContent;
  const isModel =
    relPath.toLowerCase().endsWith(".geo.json") ||
    relPath.toLowerCase().includes("/models/");

  useEffect(() => {
    if (lang === "json") {
      try {
        JSON.parse(content);
        setJsonError(null);
      } catch (e) {
        setJsonError(String(e));
      }
    } else {
      setJsonError(null);
    }
  }, [content, lang]);

  useEffect(() => {
    const ed = editorRef.current;
    if (ed && revealLine && revealLine > 0) {
      ed.revealLineInCenter(revealLine);
      ed.setPosition({ lineNumber: revealLine, column: 1 });
      ed.focus();
    }
  }, [revealLine]);

  const fileProblems = useMemo(
    () =>
      problems.filter(
        (p) => p.source_path === relPath || p.asset === relPath,
      ),
    [problems, relPath],
  );

  const handleSave = async () => {
    if (saving) return;
    setSaving(true);
    try {
      await onSave(relPath, content);
      setSavedContent(content);
      onDirtyChange(false);
    } finally {
      setSaving(false);
    }
  };

  const handleFormatJson = () => {
    if (lang !== "json") return;
    try {
      const parsed = JSON.parse(content);
      const formatted = JSON.stringify(parsed, null, 2) + "\n";
      setContent(formatted);
      onDirtyChange(formatted !== savedContent);
    } catch {
      void editorRef.current?.getAction("editor.action.formatDocument")?.run();
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="code-toolbar">
        <div className="toolbar-group">
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "#9cdcfe" }}>
            {relPath}
          </span>
          <span className="edition-badge">{lang.toUpperCase()}</span>
          {jsonError ? (
            <span style={{ color: "var(--vsc-error)", fontSize: 11 }}>
              <span className="codicon codicon-error" /> {jsonError}
            </span>
          ) : lang === "json" ? (
            <span style={{ color: "var(--vsc-ok)", fontSize: 11 }}>
              <span className="codicon codicon-check" /> Valid JSON
            </span>
          ) : null}
          {fileProblems.length > 0 && (
            <span style={{ color: "var(--vsc-warn)", fontSize: 11 }}>
              {fileProblems.length} diagnostic(s)
            </span>
          )}
        </div>

        <div className="toolbar-group">
          {lang === "json" && (
            <button
              className="vsc-btn secondary small"
              onClick={handleFormatJson}
            >
              <span className="codicon codicon-list-flat" />
              Format JSON
            </button>
          )}
          {isModel && onOpenBlockbench && (
            <button
              className="vsc-btn secondary small"
              onClick={() => onOpenBlockbench(relPath)}
            >
              <span className="codicon codicon-cube" />
              Open in Blockbench
            </button>
          )}
          <button
            className="vsc-btn small"
            disabled={saving || !isDirty}
            onClick={() => void handleSave()}
          >
            <span className="codicon codicon-save" />
            {saving ? "Saving..." : isDirty ? "Save (Ctrl+S)" : "Saved"}
          </button>
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0 }}>
        <Editor
          height="100%"
          theme="vs-dark"
          language={lang}
          value={content}
          beforeMount={(m) => {
            registerMinecraftLanguages(m);
            m.languages.json.jsonDefaults.setDiagnosticsOptions({
              validate: true,
              allowComments: true,
              trailingCommas: "ignore",
            });
          }}
          onMount={(ed, m) => {
            editorRef.current = ed;
            ed.addCommand(m.KeyMod.CtrlCmd | m.KeyCode.KeyS, () => {
              void handleSave();
            });
            if (revealLine && revealLine > 0) {
              ed.revealLineInCenter(revealLine);
              ed.setPosition({ lineNumber: revealLine, column: 1 });
            }
          }}
          onChange={(val) => {
            const next = val ?? "";
            setContent(next);
            onDirtyChange(next !== savedContent);
          }}
          options={{
            fontSize: 13,
            fontFamily: "var(--font-mono)",
            tabSize: 2,
            minimap: { enabled: true },
            automaticLayout: true,
            bracketPairColorization: { enabled: true },
            renderLineHighlight: "all",
            scrollBeyondLastLine: false,
            wordWrap: lang === "mc-lang" ? "on" : "off",
          }}
        />
      </div>
    </div>
  );
};
