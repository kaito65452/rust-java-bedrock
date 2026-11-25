import React from "react";
import { api } from "../api";

interface ErrorBoundaryProps {
  scope: string;
  title?: string;
  resetKey?: string | number | null;
  children: React.ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
  componentStack: string;
}

export class ErrorBoundary extends React.Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { error: null, componentStack: "" };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    const stackText = `${error.stack ?? ""}\n${info.componentStack ?? ""}`.trim();
    this.setState({ componentStack: info.componentStack ?? "" });
    api.frontendTraceLog(
      "error",
      `react-boundary:${this.props.scope}`,
      error.message || String(error),
      stackText,
    );
  }

  componentDidUpdate(prevProps: ErrorBoundaryProps): void {
    if (this.state.error && prevProps.resetKey !== this.props.resetKey) {
      this.setState({ error: null, componentStack: "" });
    }
  }

  private handleReload = () => {
    this.setState({ error: null, componentStack: "" });
  };

  render(): React.ReactNode {
    const { error, componentStack } = this.state;
    if (!error) {
      return this.props.children;
    }

    const fullStack = [error.stack, componentStack].filter(Boolean).join("\n\n");

    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 12,
          height: "100%",
          width: "100%",
          padding: 20,
          boxSizing: "border-box",
          background: "#1e1e1e",
          color: "#d4d4d4",
          fontFamily: "var(--font-ui, sans-serif)",
          overflow: "auto",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span
            className="codicon codicon-error"
            style={{ color: "#f48771", fontSize: 20 }}
          />
          <strong style={{ fontSize: 15, color: "#ffffff" }}>
            {this.props.title ?? "Editor encountered an unexpected error"}
          </strong>
        </div>

        <div
          style={{
            padding: "10px 12px",
            borderRadius: 4,
            background: "#2d1b1b",
            border: "1px solid #6b2b2b",
            color: "#f48771",
            fontFamily: "var(--font-mono, monospace)",
            fontSize: 12,
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
          }}
        >
          {error.message || String(error)}
        </div>

        <div style={{ display: "flex", gap: 8 }}>
          <button
            type="button"
            className="vsc-btn small"
            onClick={this.handleReload}
          >
            <span className="codicon codicon-refresh" />
            Reload Editor
          </button>
        </div>

        {fullStack && (
          <pre
            style={{
              margin: 0,
              padding: 12,
              borderRadius: 4,
              background: "#141414",
              border: "1px solid #333333",
              color: "#aaaaaa",
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 11,
              lineHeight: 1.45,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              overflow: "auto",
              flex: 1,
            }}
          >
            {fullStack}
          </pre>
        )}
      </div>
    );
  }
}
