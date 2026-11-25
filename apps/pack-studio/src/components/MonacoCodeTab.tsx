import React, { useEffect, useMemo, useRef, useState } from "react";
import Editor, { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import { api } from "../api";
import type {
  ConversionWarning,
  ExtensionSnippetDto,
  ExtensionsSnapshotDto,
  LspDiagnosticDto,
} from "../types";

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
let lspProvidersRegistered = false;
let activeRelPathForProviders = "";
let activeLanguageForProviders = "json";
let activeSnippetsForProviders: ExtensionSnippetDto[] = [];

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

  m.languages.register({ id: "pstdr", extensions: [".pstdr"] });
  m.languages.setMonarchTokensProvider("pstdr", {
    keywords: [
      "on",
      "join",
      "leave",
      "chat",
      "right",
      "click",
      "hit",
      "break",
      "hurt",
      "die",
      "consume",
      "interact",
      "tick",
      "projectile",
      "spawn",
      "script_event",
      "entity_trigger",
      "every",
      "after",
      "cancel",
      "event",
      "send",
      "apply",
      "damage",
      "ignite",
      "play_sound",
      "particle",
      "command",
      "run_fn",
      "mcfunction",
      "state",
      "const",
      "mut",
      "func",
      "unsafe",
      "if",
      "else",
      "for",
      "in",
      "while",
      "return",
      "and",
      "or",
      "not",
      "is",
      "true",
      "false",
      "null",
      "of",
      "by",
      "with",
      "to",
      "at",
      "type",
      "bind_target",
    ],
    tokenizer: {
      root: [
        [/\/\/.*$/, "comment"],
        [/\/\*/, "comment", "@comment"],
        [/@[a-zA-Z_]\w*/, "annotation"],
        [/\b\d+(\.\d+)?(ms|s|m|t)\b/, "number.float"],
        [/\b\d+(\.\d+)?\b/, "number"],
        [/"/, "string", "@string"],
        [
          /[a-zA-Z_]\w*/,
          {
            cases: {
              "@keywords": "keyword",
              "@default": "identifier",
            },
          },
        ],
        [/[{}()\[\]]/, "@brackets"],
        [/[?:.,;]/, "delimiter"],
        [/[+\-*/%=!<>&|]+/, "operator"],
      ],
      comment: [
        [/[^/*]+/, "comment"],
        [/\*\//, "comment", "@pop"],
        [/[/*]/, "comment"],
      ],
      string: [
        [/\\./, "string.escape"],
        [/\{/, { token: "delimiter.bracket", next: "@interpolation" }],
        [/"/, "string", "@pop"],
        [/[^\\"{]+/, "string"],
      ],
      interpolation: [
        [/\}/, { token: "delimiter.bracket", next: "@pop" }],
        [/[a-zA-Z_]\w*/, "variable"],
        [/\./, "delimiter"],
        [/[^{}]+/, "variable"],
      ],
    },
  });

  m.languages.register({ id: "mcfunction", extensions: [".mcfunction"] });
  m.languages.setMonarchTokensProvider("mcfunction", {
    tokenizer: {
      root: [
        [/^\s*#.*$/, "comment"],
        [/^\s*\/?[a-z_][a-z0-9_]*/, "keyword"],
        [/@[paserncv](?=\[|\s|$)|@initiator(?=\[|\s|$)/, "variable.predefined"],
        [/[~^]-?\d*(\.\d+)?/, "number.float"],
        [/-?\d+(\.\d+)?/, "number"],
        [/"([^"\\]|\\.)*"/, "string"],
        [/[a-z0-9_]+:[a-z0-9_./-]+/, "type.identifier"],
        [/[\[\]{}()=,]/, "delimiter"],
      ],
    },
  });
}

function ensureLspProvidersRegistered(m: typeof monaco) {
  if (lspProvidersRegistered) return;
  lspProvidersRegistered = true;

  for (const langId of ["json", "mc-lang", "molang", "pstdr"]) {
    m.languages.registerCompletionItemProvider(langId, {
      triggerCharacters: [".", '"', "/", "§", " ", "@"],
      async provideCompletionItems(model, position) {
        const word = model.getWordUntilPosition(position);
        const range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn,
        };

        const suggestions: monaco.languages.CompletionItem[] = [];
        for (const snip of activeSnippetsForProviders) {
          if (snip.language === langId || (langId === "json" && snip.language === "jsonc")) {
            suggestions.push({
              label: snip.prefix,
              kind: m.languages.CompletionItemKind.Snippet,
              detail: `${snip.description} (${snip.source_extension_id})`,
              insertText: snip.body.join("\n"),
              insertTextRules: m.languages.CompletionItemInsertTextRule.InsertAsSnippet,
              range,
            });
          }
        }

        try {
          const lspItems = await api.lspCompletion(
            activeRelPathForProviders,
            activeLanguageForProviders,
            position.lineNumber,
            position.column,
            word.word,
          );
          for (const item of lspItems) {
            suggestions.push({
              label: item.label,
              kind:
                item.kind === "Function"
                  ? m.languages.CompletionItemKind.Function
                  : item.kind === "File"
                    ? m.languages.CompletionItemKind.File
                    : item.kind === "Keyword"
                      ? m.languages.CompletionItemKind.Keyword
                      : m.languages.CompletionItemKind.Property,
              detail: item.detail,
              documentation: item.documentation,
              insertText: item.insert_text,
              insertTextRules:
                item.insert_text.includes("${") || item.insert_text.includes("$0")
                  ? m.languages.CompletionItemInsertTextRule.InsertAsSnippet
                  : undefined,
              range,
            });
          }
        } catch {
          // ignore transient LSP completion failures
        }

        return { suggestions };
      },
    });

    m.languages.registerHoverProvider(langId, {
      async provideHover(model, position) {
        const word = model.getWordAtPosition(position);
        const term = word?.word ?? "";
        try {
          const hovered = await api.lspHover(
            activeRelPathForProviders,
            activeLanguageForProviders,
            position.lineNumber,
            position.column,
            term,
          );
          if (!hovered) return null;
          return {
            contents: [
              { value: hovered.contents },
              { value: `*Source: ${hovered.source}*` },
            ],
          };
        } catch {
          return null;
        }
      },
    });
  }

  m.languages.registerCodeActionProvider("pstdr", {
    async provideCodeActions(model, range) {
      try {
        const items = await api.lspCodeActions(
          activeRelPathForProviders,
          "pstdr",
          range.startLineNumber,
          range.startColumn,
          range.endLineNumber,
          range.endColumn,
        );
        const actions: monaco.languages.CodeAction[] = items.map((item) => ({
          title: item.title,
          kind: item.kind || "quickfix",
          isPreferred: item.is_preferred,
          edit: {
            edits: item.edits.map((te) => ({
              resource: model.uri,
              versionId: model.getVersionId(),
              textEdit: {
                range: {
                  startLineNumber: Math.max(1, te.line),
                  startColumn: Math.max(1, te.column),
                  endLineNumber: Math.max(1, te.end_line),
                  endColumn: Math.max(1, te.end_column),
                },
                text: te.new_text,
              },
            })),
          },
        }));
        return {
          actions,
          dispose: () => {},
        };
      } catch {
        return {
          actions: [],
          dispose: () => {},
        };
      }
    },
  });
}

function detectMonacoLanguage(relPath: string): string {
  const lower = relPath.toLowerCase();
  if (lower.endsWith(".pstdr")) return "pstdr";
  if (lower.endsWith(".mcfunction")) return "mcfunction";
  if (lower.endsWith(".js")) return "javascript";
  if (lower.endsWith(".ts")) return "typescript";
  if (lower.endsWith(".toml")) return "ini";
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
  extensions?: ExtensionsSnapshotDto | null;
  lspDiagnostics?: LspDiagnosticDto[];
  onDirtyChange: (dirty: boolean) => void;
  onSave: (relPath: string, content: string) => Promise<void>;
  onOpenBlockbench?: (relPath: string) => void;
}

export const MonacoCodeTab: React.FC<MonacoCodeTabProps> = ({
  relPath,
  initialContent,
  revealLine,
  problems,
  extensions,
  lspDiagnostics = [],
  onDirtyChange,
  onSave,
  onOpenBlockbench,
}) => {
  const [content, setContent] = useState(initialContent);
  const [savedContent, setSavedContent] = useState(initialContent);
  const [saving, setSaving] = useState(false);
  const [jsonError, setJsonError] = useState<string | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<typeof monaco | null>(null);
  const versionRef = useRef<number>(1);

  useEffect(() => {
    setContent(initialContent);
    setSavedContent(initialContent);
    versionRef.current = 1;
    onDirtyChange(false);
  }, [relPath, initialContent]);

  const lang = useMemo(() => detectMonacoLanguage(relPath), [relPath]);
  const isDirty = content !== savedContent;
  const isModel =
    relPath.toLowerCase().endsWith(".geo.json") ||
    relPath.toLowerCase().includes("/models/");

  useEffect(() => {
    activeRelPathForProviders = relPath;
    activeLanguageForProviders = lang;
    activeSnippetsForProviders = extensions?.active_snippets ?? [];
  }, [relPath, lang, extensions]);

  // Apply extension JSON schemas dynamically into Monaco jsonDefaults
  useEffect(() => {
    const m = monacoRef.current ?? monaco;
    const schemas = (extensions?.active_schemas ?? []).map((s) => ({
      uri: s.uri,
      fileMatch: s.file_match,
      schema: s.schema,
    }));
    m.languages.json.jsonDefaults.setDiagnosticsOptions({
      validate: true,
      allowComments: true,
      trailingCommas: "ignore",
      schemas,
    });
  }, [extensions?.active_schemas]);

  // Sync open & incremental changes to Rust stdio LSP + built-in Minecraft LSP
  useEffect(() => {
    void api.lspSyncDocument(relPath, lang, initialContent, 1, "open").catch(() => {});
  }, [relPath, lang, initialContent]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      versionRef.current += 1;
      void api
        .lspSyncDocument(relPath, lang, content, versionRef.current, "change")
        .catch(() => {});
    }, 280);
    return () => window.clearTimeout(timer);
  }, [relPath, lang, content]);

  // Apply live LSP diagnostics markers to the active Monaco model
  useEffect(() => {
    const ed = editorRef.current;
    const m = monacoRef.current;
    if (!ed || !m) return;
    const model = ed.getModel();
    if (!model) return;
    const markers: monaco.editor.IMarkerData[] = lspDiagnostics
      .filter((d) => d.rel_path === relPath)
      .map((d) => ({
        startLineNumber: Math.max(1, d.line),
        startColumn: Math.max(1, d.column),
        endLineNumber: Math.max(1, d.end_line),
        endColumn: Math.max(2, d.end_column),
        code: d.code ?? undefined,
        message: `[${d.source}] ${d.message}`,
        severity:
          d.severity === "error"
            ? m.MarkerSeverity.Error
            : d.severity === "warning"
              ? m.MarkerSeverity.Warning
              : m.MarkerSeverity.Info,
      }));
    m.editor.setModelMarkers(model, "packstudio-lsp", markers);
  }, [relPath, lspDiagnostics]);

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

  const fileLspDiags = useMemo(
    () => lspDiagnostics.filter((d) => d.rel_path === relPath),
    [lspDiagnostics, relPath],
  );

  const handleSave = async () => {
    if (saving) return;
    setSaving(true);
    try {
      await onSave(relPath, content);
      setSavedContent(content);
      onDirtyChange(false);
      versionRef.current += 1;
      void api
        .lspSyncDocument(relPath, lang, content, versionRef.current, "save")
        .catch(() => {});
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
          {(fileProblems.length > 0 || fileLspDiags.length > 0) && (
            <span style={{ color: "var(--vsc-warn)", fontSize: 11 }}>
              {fileProblems.length + fileLspDiags.length} diagnostic(s)
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
          path={relPath}
          theme="vs-dark"
          language={lang}
          value={content}
          beforeMount={(m) => {
            monacoRef.current = m;
            registerMinecraftLanguages(m);
            ensureLspProvidersRegistered(m);
            m.languages.json.jsonDefaults.setDiagnosticsOptions({
              validate: true,
              allowComments: true,
              trailingCommas: "ignore",
            });
          }}
          onMount={(ed, m) => {
            editorRef.current = ed;
            monacoRef.current = m;
            activeRelPathForProviders = relPath;
            activeLanguageForProviders = lang;
            activeSnippetsForProviders = extensions?.active_snippets ?? [];
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
