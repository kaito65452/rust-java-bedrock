use egui::{Color32, Context, CornerRadius, Margin, Stroke, Style, Visuals};
use egui_dock::Style as DockStyle;

/// VS Code Dark+ editor canvas background (`#1E1E1E`).
pub const BG_EDITOR: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x1E);
/// VS Code Dark+ primary sidebar & panel header background (`#252526`).
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x25, 0x25, 0x26);
/// VS Code Dark+ activity bar background (`#333333`).
pub const BG_ACTIVITY_BAR: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
/// VS Code Dark+ status bar blue (`#007ACC`).
pub const BG_STATUS_BAR: Color32 = Color32::from_rgb(0x00, 0x7A, 0xCC);
/// VS Code Dark+ inactive tab / border tone (`#2D2D2D`).
pub const BORDER: Color32 = Color32::from_rgb(0x2D, 0x2D, 0x2D);
/// VS Code Dark+ inactive tab fill (`#2D2D2D`).
pub const BG_TAB_INACTIVE: Color32 = Color32::from_rgb(0x2D, 0x2D, 0x2D);
/// VS Code Dark+ input / dropdown fill (`#3C3C3C`).
pub const BG_INPUT: Color32 = Color32::from_rgb(0x3C, 0x3C, 0x3C);
/// VS Code Dark+ hover highlight (`#2A2D2E`).
pub const BG_HOVER: Color32 = Color32::from_rgb(0x2A, 0x2D, 0x2E);
/// VS Code Dark+ active list selection (`#094771`).
pub const BG_SELECTION: Color32 = Color32::from_rgb(0x09, 0x47, 0x71);
/// VS Code Dark+ secondary list focus (`#37373D`).
pub const BG_LIST_ACTIVE: Color32 = Color32::from_rgb(0x37, 0x37, 0x3D);
/// VS Code Dark+ primary foreground text (`#D4D4D4`).
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xD4, 0xD4, 0xD4);
/// VS Code Dark+ muted / secondary text (`#858585`).
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x85, 0x85, 0x85);
/// Pure white text for active tabs and status bar.
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// VS Code accent blue (`#007ACC`).
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x00, 0x7A, 0xCC);
/// Diagnostic error red (`#F44747`).
pub const SEVERITY_ERROR: Color32 = Color32::from_rgb(0xF4, 0x47, 0x47);
/// Diagnostic warning amber (`#CCA700`).
pub const SEVERITY_WARNING: Color32 = Color32::from_rgb(0xCC, 0xA7, 0x00);
/// Diagnostic info blue (`#3794FF`).
pub const SEVERITY_INFO: Color32 = Color32::from_rgb(0x37, 0x94, 0xFF);
/// Audit passed green (`#89D185`).
pub const SEVERITY_OK: Color32 = Color32::from_rgb(0x89, 0xD1, 0x85);
/// Badge fill for PASSED status.
pub const BADGE_PASSED_BG: Color32 = Color32::from_rgb(0x1E, 0x6B, 0x34);
/// Badge fill for PARTIAL / WARNING status.
pub const BADGE_WARN_BG: Color32 = Color32::from_rgb(0x9A, 0x67, 0x00);
/// Badge fill for FAILED status.
pub const BADGE_FAIL_BG: Color32 = Color32::from_rgb(0xA6, 0x1E, 0x1E);

/// Applies the VS Code Dark+ theme to the given `egui::Context`.
pub fn apply_vscode_dark_plus(ctx: &Context) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = BG_EDITOR;
    visuals.window_fill = BG_SIDEBAR;
    visuals.faint_bg_color = Color32::from_rgb(0x22, 0x22, 0x23);
    visuals.extreme_bg_color = Color32::from_rgb(0x18, 0x18, 0x18);
    visuals.code_bg_color = Color32::from_rgb(0x1B, 0x1B, 0x1C);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(4);
    visuals.override_text_color = Some(TEXT_PRIMARY);
    visuals.hyperlink_color = Color32::from_rgb(0x37, 0x94, 0xFF);

    visuals.selection.bg_fill = BG_SELECTION;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_BLUE);

    visuals.widgets.noninteractive.bg_fill = BG_SIDEBAR;
    visuals.widgets.noninteractive.weak_bg_fill = BG_SIDEBAR;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(2);

    visuals.widgets.inactive.bg_fill = BG_INPUT;
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x2F, 0x2F, 0x31);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(2);

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x46, 0x46, 0x49);
    visuals.widgets.hovered.weak_bg_fill = BG_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(2);

    visuals.widgets.active.bg_fill = BG_SELECTION;
    visuals.widgets.active.weak_bg_fill = BG_SELECTION;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    visuals.widgets.active.corner_radius = CornerRadius::same(2);

    visuals.widgets.open.bg_fill = BG_LIST_ACTIVE;
    visuals.widgets.open.weak_bg_fill = BG_LIST_ACTIVE;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    visuals.widgets.open.corner_radius = CornerRadius::same(2);

    let mut style = Style {
        visuals,
        ..Style::default()
    };
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.window_margin = Margin::same(8);
    style.spacing.indent = 14.0;
    ctx.set_style(style);
}

/// Builds an `egui_dock::Style` styled to match VS Code Dark+ editor tab groups.
#[must_use]
pub fn vscode_dock_style(style: &Style) -> DockStyle {
    let mut dock = DockStyle::from_egui(style);
    dock.dock_area_padding = Some(Margin::ZERO);
    dock.main_surface_border_stroke = Stroke::new(1.0, BORDER);
    dock.main_surface_border_rounding = CornerRadius::ZERO;

    dock.tab_bar.bg_fill = BG_SIDEBAR;
    dock.tab_bar.height = 32.0;
    dock.tab_bar.hline_color = BORDER;
    dock.tab_bar.corner_radius = CornerRadius::ZERO;
    dock.tab_bar.fill_tab_bar = true;

    dock.tab.tab_body.bg_fill = BG_EDITOR;
    dock.tab.tab_body.stroke = Stroke::new(1.0, BORDER);
    dock.tab.tab_body.corner_radius = CornerRadius::ZERO;
    dock.tab.tab_body.inner_margin = Margin::same(0);

    dock.tab.active.bg_fill = BG_EDITOR;
    dock.tab.active.text_color = TEXT_BRIGHT;
    dock.tab.active.outline_color = BORDER;
    dock.tab.active.corner_radius = CornerRadius::ZERO;

    dock.tab.focused.bg_fill = BG_EDITOR;
    dock.tab.focused.text_color = TEXT_BRIGHT;
    dock.tab.focused.outline_color = ACCENT_BLUE;
    dock.tab.focused.corner_radius = CornerRadius::ZERO;

    dock.tab.inactive.bg_fill = BG_TAB_INACTIVE;
    dock.tab.inactive.text_color = TEXT_MUTED;
    dock.tab.inactive.outline_color = BORDER;
    dock.tab.inactive.corner_radius = CornerRadius::ZERO;

    dock.tab.hovered.bg_fill = BG_HOVER;
    dock.tab.hovered.text_color = TEXT_BRIGHT;
    dock.tab.hovered.outline_color = BORDER;
    dock.tab.hovered.corner_radius = CornerRadius::ZERO;

    dock.separator.color_idle = BORDER;
    dock.separator.color_hovered = ACCENT_BLUE;
    dock.separator.color_dragged = ACCENT_BLUE;
    dock.separator.width = 2.0;
    dock
}
