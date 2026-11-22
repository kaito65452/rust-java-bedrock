/// Executable quick-action commands exposed via the VS Code-style Command Palette (`Ctrl+Shift+P` / `F1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StudioCommand {
    OpenFolder,
    OpenArchive,
    NewProjectWizard,
    ExportPackage,
    QuickOpenFile,
    ReloadWorkspace,
    ConvertJavaToBedrockPreflight,
    ConvertBedrockToJavaPreflight,
    OpenConversionDashboard,
    RunSemanticAudit,
    OpenInBlockbenchStudio,
    NewBedrockGeoInBlockbench,
    OpenActiveInExternalEditor,
    SaveActiveEditor,
    ToggleSidebar,
    ToggleBottomPanel,
    FocusExplorer,
    FocusSearch,
    FocusConverter,
    FocusMetadata,
    FocusSettings,
}

/// Static descriptor for a Command Palette entry.
#[derive(Debug, Clone, Copy)]
pub struct CommandEntry {
    pub command: StudioCommand,
    pub title: &'static str,
    pub shortcut: &'static str,
}

/// Complete registry of PackStudio Command Palette actions.
pub const COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        command: StudioCommand::NewProjectWizard,
        title: "Project: New Resource Pack / Add-On Wizard (Java & Bedrock)...",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::OpenFolder,
        title: "Pack: Open Folder...",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::OpenArchive,
        title: "Pack: Open Archive (.zip / .mcpack)...",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::ExportPackage,
        title: "Pack: Export / Package (.zip / .mcpack / .mcaddon)...",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::QuickOpenFile,
        title: "File: Quick Open File...",
        shortcut: "Ctrl+P",
    },
    CommandEntry {
        command: StudioCommand::ReloadWorkspace,
        title: "Pack: Reload Workspace",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::ConvertJavaToBedrockPreflight,
        title: "Convert: Java -> Bedrock (Run Preflight)",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::ConvertBedrockToJavaPreflight,
        title: "Convert: Bedrock -> Java (Run Preflight)",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::OpenConversionDashboard,
        title: "Convert: Open Conversion & Preflight Dashboard",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::RunSemanticAudit,
        title: "Audit: Run Semantic Validation",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::OpenInBlockbenchStudio,
        title: "Open in Blockbench (3D & Paint)",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::NewBedrockGeoInBlockbench,
        title: "New Bedrock Geometry (.geo.json) in Blockbench",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::OpenActiveInExternalEditor,
        title: "Artist: Open Active Asset in External Image Editor",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::SaveActiveEditor,
        title: "File: Save Active Editor",
        shortcut: "Ctrl+S",
    },
    CommandEntry {
        command: StudioCommand::ToggleSidebar,
        title: "View: Toggle Sidebar",
        shortcut: "Ctrl+B",
    },
    CommandEntry {
        command: StudioCommand::ToggleBottomPanel,
        title: "View: Toggle Bottom Panel",
        shortcut: "Ctrl+J",
    },
    CommandEntry {
        command: StudioCommand::FocusExplorer,
        title: "View: Show Explorer",
        shortcut: "Ctrl+Shift+E",
    },
    CommandEntry {
        command: StudioCommand::FocusSearch,
        title: "View: Show Search",
        shortcut: "Ctrl+Shift+F",
    },
    CommandEntry {
        command: StudioCommand::FocusConverter,
        title: "View: Show Converter & Preflight",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::FocusMetadata,
        title: "View: Show Pack Metadata & Packaging",
        shortcut: "",
    },
    CommandEntry {
        command: StudioCommand::FocusSettings,
        title: "Preferences: Open Settings",
        shortcut: "",
    },
];

/// Scores a fuzzy subsequence match of `query` against `candidate` (case-insensitive).
/// Higher scores indicate stronger prefix, word-boundary, or contiguous matches.
/// Returns `None` if not all query characters appear in order inside `candidate`.
#[must_use]
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Some(0);
    }

    let c_chars: Vec<char> = candidate.to_ascii_lowercase().chars().collect();
    let q_chars: Vec<char> = q.chars().collect();

    let mut score = 0i32;
    let mut c_idx = 0usize;
    let mut prev_match_idx: Option<usize> = None;

    for qc in q_chars {
        if qc.is_whitespace() {
            continue;
        }
        let mut found = None;
        while c_idx < c_chars.len() {
            if c_chars[c_idx] == qc {
                found = Some(c_idx);
                c_idx += 1;
                break;
            }
            c_idx += 1;
        }
        let pos = found?;
        score += 10;
        if pos == 0 {
            score += 20;
        } else {
            let prev = c_chars[pos - 1];
            if matches!(prev, ' ' | ':' | '-' | '_' | '/' | '(' | '.') {
                score += 15;
            }
        }
        if let Some(prev_pos) = prev_match_idx {
            if pos == prev_pos + 1 {
                score += 12;
            } else {
                let gap = pos.saturating_sub(prev_pos + 1);
                score -= (gap as i32).min(8);
            }
        }
        prev_match_idx = Some(pos);
    }

    // Substring bonus for exact phrase matches.
    if candidate.to_ascii_lowercase().contains(&q) {
        score += 30;
    }

    Some(score)
}

/// Filters and ranks the Command Palette registry by fuzzy match score.
#[must_use]
pub fn filter_commands(query: &str) -> Vec<&'static CommandEntry> {
    let mut scored: Vec<(i32, usize, &'static CommandEntry)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| fuzzy_score(query, entry.title).map(|s| (s, idx, entry)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, e)| e).collect()
}
