import { useEffect, useState } from "react";
import type { RecentProjectEntry } from "../types";
import { packTypeBadgeLabel } from "../utils/mcFormat";
import { APP_DISPLAY_VERSION } from "../utils/version";
import type { SettingsCategory } from "./SettingsView";
import { VoxelTesseractLogo } from "./VoxelTesseractLogo";

const TUTORIAL_STORAGE_KEY = "pack_studio_tutorial_done_v1";

/**
 * Guided Wave 3B PSTDR-DSL starter demonstrating `@addon`, `on cmd`, `ui action`, and `scoreboard`.
 */
export const GUIDED_PSTDR_STARTER_SOURCE = `// PackStudio Quick Tutorial — PSTDR-DSL Wave 3B Starter
// Targets Bedrock 26.60 (@minecraft/server 2.11.0); switch api to "1.17.0" for 1.x LTS.

@addon(target = "26.60", api = "2.11.0")

on join of player {
    scoreboard show "tokens" title "§bTokens" slot sidebar
    after 50ms {
        score["tokens"][player] += 10
        send "§a[PackStudio] Welcome! Run §e/studio:hub§a to open the Action UI." to player
    }
}

on cmd "studio:hub" description "Open the PackStudio Quick Tutorial hub" permission any () by player {
    ui action "PackStudio Hub" body "Balance: {score[\\"tokens\\"][player]} tokens" to player {
        button "Claim +10 Tokens" {
            score["tokens"][player] += 10
            player.play_sound("random.orb")
            send "§a+10 Tokens awarded!" to player
        }
        button "Speed Boost (5 Tokens)" {
            if score["tokens"][player] >= 5 {
                score["tokens"][player] -= 5
                apply speed 2 to player for 30s
                player.play_sound("random.levelup")
                send "§bSpeed II active for 30s!" to player
            } else {
                send "§cNot enough tokens (need 5)." to player
            }
        }
    }
}
`;

export interface QuickTutorialViewProps {
  recentProjects: RecentProjectEntry[];
  hasOpenProject: boolean;
  onOpenFolder: () => void;
  onOpenArchive: () => void;
  onOpenWizard: () => void;
  onOpenBlockbench: () => void;
  onOpenConverter: () => void;
  onOpenPstdrStarter: () => void;
  onOpenSettings: (category: SettingsCategory) => void;
  onOpenRecentProject: (path: string) => void;
  onRemoveRecentProject: (path: string) => void;
  onClearRecentProjects: () => void;
}

interface TutorialStepDef {
  id: string;
  number: number;
  icon: string;
  accent: string;
  title: string;
  description: string;
  actionLabel: string;
  tag: string;
}

const TUTORIAL_STEPS: TutorialStepDef[] = [
  {
    id: "create_pack",
    number: 1,
    icon: "codicon-new-folder",
    accent: "#38bdf8",
    title: "Create or Open a Resource/Behavior Pack",
    description:
      "Scaffold a Java Resource Pack, Bedrock Resource Pack, or RP + BP Add-on workspace with valid manifests and UUIDs.",
    actionLabel: "Open Project Wizard",
    tag: "Scaffold",
  },
  {
    id: "convert_audit",
    number: 2,
    icon: "codicon-arrow-swap",
    accent: "#34d399",
    title: "Convert Java ↔ Bedrock with Semantic Audit Gate",
    description:
      "Inspect asset mappings, run preflight diagnostics, and convert packs with automated geometry & UV checks.",
    actionLabel: "Open Converter Studio",
    tag: "Audit Gate",
  },
  {
    id: "pstdr_dsl",
    number: 3,
    icon: "codicon-code",
    accent: "#a78bfa",
    title: "Write Add-ons with PSTDR-DSL (2.11.0 & 1.x LTS)",
    description:
      "Open a guided .pstdr starter featuring on cmd, ui action, and scoreboard with live built-in LSP diagnostics.",
    actionLabel: "Open Guided .pstdr Starter",
    tag: "Wave 3B LSP",
  },
  {
    id: "blockbench_3d",
    number: 4,
    icon: "codicon-cube",
    accent: "#f59e0b",
    title: "Edit 3D Models in Embedded Blockbench",
    description:
      "Model .geo.json and Java block/item JSON directly inside PackStudio with live two-way workspace save sync.",
    actionLabel: "Launch Blockbench 3D",
    tag: "3D Bridge",
  },
  {
    id: "sync_ai_settings",
    number: 5,
    icon: "codicon-shield",
    accent: "#ec4899",
    title: "Connect GitHub, Verified Updates & AI Agents",
    description:
      "Configure Ed25519 + SHA-256 verified PSTDR updates, connect your GitHub profile, and bridge Claude Code / Antigravity / OpenCode.",
    actionLabel: "Open Settings",
    tag: "Zero-Trust + AI",
  },
];

function loadCompletedSteps(): string[] {
  try {
    const raw = localStorage.getItem(TUTORIAL_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return parsed.filter((x): x is string => typeof x === "string");
      }
    }
  } catch {
    // ignore storage errors
  }
  return [];
}

function formatRelativeTime(ts: number): string {
  const diffSec = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (diffSec < 60) return "just now";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return `${Math.floor(diffSec / 86400)}d ago`;
}

export function QuickTutorialView({
  recentProjects,
  hasOpenProject,
  onOpenFolder,
  onOpenArchive,
  onOpenWizard,
  onOpenBlockbench,
  onOpenConverter,
  onOpenPstdrStarter,
  onOpenSettings,
  onOpenRecentProject,
  onRemoveRecentProject,
  onClearRecentProjects,
}: QuickTutorialViewProps) {
  const [completedIds, setCompletedIds] = useState<string[]>(loadCompletedSteps);

  useEffect(() => {
    try {
      localStorage.setItem(TUTORIAL_STORAGE_KEY, JSON.stringify(completedIds));
    } catch {
      // ignore storage errors
    }
  }, [completedIds]);

  const markCompleted = (id: string) => {
    setCompletedIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
  };

  const toggleCompleted = (id: string) => {
    setCompletedIds((prev) =>
      prev.includes(id) ? prev.filter((item) => item !== id) : [...prev, id],
    );
  };

  const handleStepAction = (id: string) => {
    markCompleted(id);
    switch (id) {
      case "create_pack":
        onOpenWizard();
        break;
      case "convert_audit":
        onOpenConverter();
        break;
      case "pstdr_dsl":
        onOpenPstdrStarter();
        break;
      case "blockbench_3d":
        onOpenBlockbench();
        break;
      case "sync_ai_settings":
        onOpenSettings("updates");
        break;
    }
  };

  const completedCount = TUTORIAL_STEPS.filter((s) =>
    completedIds.includes(s.id),
  ).length;
  const progressPct = Math.round((completedCount / TUTORIAL_STEPS.length) * 100);

  return (
    <div className="welcome-screen">
      <div className="welcome-container">
        {/* Hero Header */}
        <header className="welcome-hero">
          <VoxelTesseractLogo variant="hero" size={86} />
          <div className="welcome-hero-copy">
            <div className="welcome-title-row">
              <h1 className="welcome-title">PackStudio</h1>
              <span
                className="welcome-version-pill"
                title={`PackStudio version ${APP_DISPLAY_VERSION}`}
              >
                v{APP_DISPLAY_VERSION}
              </span>
            </div>
            <p className="welcome-sub">
              Minecraft Java &amp; Bedrock Resource Pack, Add-on &amp; PSTDR-DSL Workbench
            </p>
          </div>
        </header>

        {/* Balanced 2-Column VS Code Start Layout */}
        <div className="welcome-grid">
          {/* Left Column: Start & Recent Packs */}
          <section className="welcome-col">
            <div className="welcome-section">
              <div className="welcome-section-header">
                <h2>Start</h2>
              </div>
              <div className="welcome-start-list">
                <button className="welcome-start-btn primary" onClick={onOpenFolder}>
                  <span className="welcome-start-main">
                    <span className="codicon codicon-folder-opened" />
                    <span>Open Pack Folder</span>
                  </span>
                  <span className="kbd">Ctrl+O</span>
                </button>

                <button className="welcome-start-btn" onClick={onOpenArchive}>
                  <span className="welcome-start-main">
                    <span className="codicon codicon-file-zip" />
                    <span>Open Archive (.zip / .mcpack)</span>
                  </span>
                  <span className="welcome-start-hint">.mcaddon</span>
                </button>

                <button className="welcome-start-btn" onClick={onOpenWizard}>
                  <span className="welcome-start-main">
                    <span className="codicon codicon-add" />
                    <span>New Project Wizard</span>
                  </span>
                  <span className="kbd">Ctrl+Shift+N</span>
                </button>

                <button className="welcome-start-btn" onClick={onOpenBlockbench}>
                  <span className="welcome-start-main">
                    <span className="codicon codicon-cube" />
                    <span>Launch Blockbench 3D</span>
                  </span>
                  <span className="welcome-start-hint">Embedded</span>
                </button>
              </div>
            </div>

            <div className="welcome-section">
              <div className="welcome-section-header">
                <h2>Recent Packs</h2>
                {recentProjects.length > 0 && (
                  <button
                    className="welcome-link-btn"
                    onClick={onClearRecentProjects}
                  >
                    Clear
                  </button>
                )}
              </div>

              {recentProjects.length === 0 ? (
                <div className="welcome-recent-empty">
                  <span className="codicon codicon-history" />
                  <span>
                    No recent packs yet. Open a folder, import an archive, or run
                    the New Project Wizard to get started.
                  </span>
                </div>
              ) : (
                <div className="welcome-recent-list">
                  {recentProjects.map((item) => (
                    <div
                      key={item.path}
                      className="recent-pack-item"
                      onClick={() => onOpenRecentProject(item.path)}
                      title={item.path}
                    >
                      <div className="recent-pack-info">
                        <div className="recent-pack-top">
                          <span className="recent-pack-name">{item.name}</span>
                          <span className="recent-pack-badge">
                            {packTypeBadgeLabel(item.pack_type)}
                          </span>
                        </div>
                        <div className="recent-pack-meta">
                          <span className="recent-pack-path">{item.path}</span>
                          <span className="recent-pack-time">
                            {formatRelativeTime(item.opened_at)}
                          </span>
                        </div>
                      </div>
                      <button
                        className="icon-btn recent-remove-btn"
                        title="Remove from Recent Packs"
                        onClick={(e) => {
                          e.stopPropagation();
                          onRemoveRecentProject(item.path);
                        }}
                      >
                        <span className="codicon codicon-close" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="welcome-shortcuts-bar">
              <span>
                <span className="kbd">Ctrl+Shift+P</span> Commands
              </span>
              <span>
                <span className="kbd">Ctrl+P</span> Quick Open
              </span>
              <span>
                <span className="kbd">Ctrl+,</span> Settings
              </span>
            </div>
          </section>

          {/* Right Column: Quick Tutorial — Interactive Walkthrough */}
          <section className="welcome-col">
            <div className="welcome-section">
              <div className="welcome-section-header">
                <h2>Quick Tutorial — Interactive Walkthrough</h2>
                <div className="tutorial-progress-meta">
                  <span>
                    {completedCount}/{TUTORIAL_STEPS.length} done
                  </span>
                  {completedCount > 0 && (
                    <button
                      className="welcome-link-btn"
                      onClick={() => setCompletedIds([])}
                    >
                      Reset
                    </button>
                  )}
                </div>
              </div>

              <div className="tutorial-progress-track" aria-label={`Walkthrough progress ${progressPct}%`}>
                <div
                  className="tutorial-progress-fill"
                  style={{ width: `${progressPct}%` }}
                />
              </div>

              <div className="tutorial-steps-list">
                {TUTORIAL_STEPS.map((step) => {
                  const isDone = completedIds.includes(step.id);
                  return (
                    <div
                      key={step.id}
                      className={`tutorial-step-card ${isDone ? "is-done" : ""}`}
                      onClick={() => handleStepAction(step.id)}
                    >
                      <button
                        type="button"
                        className={`tutorial-step-check ${isDone ? "checked" : ""}`}
                        title={isDone ? "Mark step incomplete" : "Mark step complete"}
                        onClick={(e) => {
                          e.stopPropagation();
                          toggleCompleted(step.id);
                        }}
                      >
                        {isDone ? (
                          <span className="codicon codicon-check" />
                        ) : (
                          <span>{step.number}</span>
                        )}
                      </button>

                      <div className="tutorial-step-body">
                        <div className="tutorial-step-top">
                          <span className="tutorial-step-title">
                            <span
                              className={`codicon ${step.icon}`}
                              style={{ color: step.accent }}
                            />
                            {step.title}
                          </span>
                          <span className="tutorial-step-tag">{step.tag}</span>
                        </div>
                        <p className="tutorial-step-desc">{step.description}</p>
                        <div className="tutorial-step-footer">
                          <span className="tutorial-step-cta">
                            {step.actionLabel}
                            <span className="codicon codicon-arrow-right" />
                          </span>
                          {step.id === "pstdr_dsl" && !hasOpenProject && (
                            <span className="tutorial-step-note">
                              Opens live scratch starter
                            </span>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
