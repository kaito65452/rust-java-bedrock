use egui::{Color32, Context, CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2, Visuals};

use rp_core::Edition;

/// Material Design 3 (Material You) tonal color palette for Dark and Light themes.
#[derive(Debug, Clone, Copy)]
pub struct M3Palette {
    /// Base canvas / panel surface.
    pub surface: Color32,
    /// Deepest container (code editors, 3D canvas backdrop).
    pub surface_container_lowest: Color32,
    /// Navigation rail / side bar surface.
    pub surface_container_low: Color32,
    /// Standard card & top bar surface.
    pub surface_container: Color32,
    /// Elevated container (hovered pills, modals, active panels).
    pub surface_container_high: Color32,
    /// Highest container (text inputs, chips, buttons).
    pub surface_container_highest: Color32,

    /// M3 Primary accent (active indicators, primary icons/labels).
    pub primary: Color32,
    /// Text/icon color placed on `primary`.
    pub on_primary: Color32,
    /// Tonal primary container (selected nav pills, FABs, prominent actions).
    pub primary_container: Color32,
    /// Text/icon color placed on `primary_container`.
    pub on_primary_container: Color32,

    /// M3 Secondary accent.
    pub secondary: Color32,
    /// Tonal secondary container (segmented tabs, filter chips).
    pub secondary_container: Color32,
    /// Text/icon color placed on `secondary_container`.
    pub on_secondary_container: Color32,

    /// M3 Tertiary accent (highlights, manual override indicators).
    pub tertiary: Color32,
    pub tertiary_container: Color32,
    pub on_tertiary_container: Color32,

    /// Edition-specific M3 tonal roles for clear Java vs Bedrock separation.
    pub java_accent: Color32,
    pub java_container: Color32,
    pub on_java_container: Color32,
    pub bedrock_accent: Color32,
    pub bedrock_container: Color32,
    pub on_bedrock_container: Color32,

    /// Text & border tokens.
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,

    /// Semantic feedback tones.
    pub error: Color32,
    pub warning: Color32,
    pub success: Color32,
}

impl M3Palette {
    /// Returns the Material 3 tonal palette for dark or light mode.
    #[must_use]
    pub const fn for_mode(dark_mode: bool) -> Self {
        if dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// Material Design 3 Dark tonal palette (Dynamic Violet-Blue seed).
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            surface: Color32::from_rgb(20, 18, 24), // #141218
            surface_container_lowest: Color32::from_rgb(15, 13, 19), // #0F0D13
            surface_container_low: Color32::from_rgb(29, 27, 32), // #1D1B20
            surface_container: Color32::from_rgb(33, 31, 38), // #211F26
            surface_container_high: Color32::from_rgb(43, 41, 48), // #2B2930
            surface_container_highest: Color32::from_rgb(54, 52, 59), // #36343B

            primary: Color32::from_rgb(208, 188, 255), // #D0BCFF
            on_primary: Color32::from_rgb(56, 30, 114), // #381E72
            primary_container: Color32::from_rgb(79, 55, 139), // #4F378B
            on_primary_container: Color32::from_rgb(234, 221, 255), // #EADDFF

            secondary: Color32::from_rgb(204, 194, 220), // #CCC2DC
            secondary_container: Color32::from_rgb(74, 68, 88), // #4A4458
            on_secondary_container: Color32::from_rgb(232, 222, 248), // #E8DEF8

            tertiary: Color32::from_rgb(239, 184, 200), // #EFB8C8
            tertiary_container: Color32::from_rgb(99, 59, 72), // #633B48
            on_tertiary_container: Color32::from_rgb(255, 216, 228), // #FFD8E4

            java_accent: Color32::from_rgb(255, 183, 124), // #FFB77C
            java_container: Color32::from_rgb(78, 43, 14), // #4E2B0E
            on_java_container: Color32::from_rgb(255, 220, 194), // #FFDCC2
            bedrock_accent: Color32::from_rgb(132, 218, 165), // #84DAA5
            bedrock_container: Color32::from_rgb(18, 62, 40), // #123E28
            on_bedrock_container: Color32::from_rgb(198, 246, 214), // #C6F6D6

            on_surface: Color32::from_rgb(230, 224, 233), // #E6E0E9
            on_surface_variant: Color32::from_rgb(202, 196, 208), // #CAC4D0
            outline: Color32::from_rgb(147, 143, 153),    // #938F99
            outline_variant: Color32::from_rgb(73, 69, 79), // #49454F

            error: Color32::from_rgb(242, 184, 181), // #F2B8B5
            warning: Color32::from_rgb(255, 213, 132), // #FFD584
            success: Color32::from_rgb(132, 218, 165), // #84DAA5
        }
    }

    /// Material Design 3 Light tonal palette.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            surface: Color32::from_rgb(254, 247, 255), // #FEF7FF
            surface_container_lowest: Color32::from_rgb(255, 255, 255), // #FFFFFF
            surface_container_low: Color32::from_rgb(247, 242, 250), // #F7F2FA
            surface_container: Color32::from_rgb(243, 237, 247), // #F3EDF7
            surface_container_high: Color32::from_rgb(236, 230, 240), // #ECE6F0
            surface_container_highest: Color32::from_rgb(230, 224, 233), // #E6E0E9

            primary: Color32::from_rgb(103, 80, 164), // #6750A4
            on_primary: Color32::from_rgb(255, 255, 255), // #FFFFFF
            primary_container: Color32::from_rgb(234, 221, 255), // #EADDFF
            on_primary_container: Color32::from_rgb(33, 0, 93), // #21005D

            secondary: Color32::from_rgb(98, 91, 113), // #625B71
            secondary_container: Color32::from_rgb(232, 222, 248), // #E8DEF8
            on_secondary_container: Color32::from_rgb(29, 25, 43), // #1D192B

            tertiary: Color32::from_rgb(125, 82, 96), // #7D5260
            tertiary_container: Color32::from_rgb(255, 216, 228), // #FFD8E4
            on_tertiary_container: Color32::from_rgb(49, 17, 29), // #31111D

            java_accent: Color32::from_rgb(154, 69, 0), // #9A4500
            java_container: Color32::from_rgb(255, 219, 195), // #FFDBC3
            on_java_container: Color32::from_rgb(56, 20, 0), // #381400
            bedrock_accent: Color32::from_rgb(20, 108, 59), // #146C3B
            bedrock_container: Color32::from_rgb(196, 242, 211), // #C4F2D3
            on_bedrock_container: Color32::from_rgb(0, 33, 16), // #002110

            on_surface: Color32::from_rgb(29, 27, 32), // #1D1B20
            on_surface_variant: Color32::from_rgb(73, 69, 79), // #49454F
            outline: Color32::from_rgb(121, 116, 126), // #79747E
            outline_variant: Color32::from_rgb(202, 196, 208), // #CAC4D0

            error: Color32::from_rgb(179, 38, 30),   // #B3261E
            warning: Color32::from_rgb(154, 91, 0),  // #9A5B00
            success: Color32::from_rgb(20, 108, 59), // #146C3B
        }
    }

    /// Returns `(container_bg, on_container_fg, accent)` for Java vs Bedrock edition badges.
    #[must_use]
    pub const fn edition_colors(&self, edition: Edition) -> (Color32, Color32, Color32) {
        match edition {
            Edition::Java => (
                self.java_container,
                self.on_java_container,
                self.java_accent,
            ),
            Edition::Bedrock => (
                self.bedrock_container,
                self.on_bedrock_container,
                self.bedrock_accent,
            ),
        }
    }
}

/// Applies the Material Design 3 tonal palette, rounded pill geometry, and elevation spacing to `ctx`.
pub fn apply_material3_theme(ctx: &Context, dark_mode: bool) -> M3Palette {
    let p = M3Palette::for_mode(dark_mode);
    let mut visuals = if dark_mode {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.override_text_color = Some(p.on_surface);
    visuals.panel_fill = p.surface;
    visuals.window_fill = p.surface_container_high;
    visuals.faint_bg_color = p.surface_container_low;
    visuals.extreme_bg_color = p.surface_container_lowest;
    visuals.code_bg_color = p.surface_container_lowest;

    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.window_stroke = Stroke::new(1.0, p.outline_variant);
    visuals.menu_corner_radius = CornerRadius::same(12);
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(if dark_mode { 90 } else { 32 }),
    };
    visuals.window_shadow = visuals.popup_shadow;

    visuals.selection.bg_fill = p.primary_container;
    visuals.selection.stroke = Stroke::new(1.25, p.primary);
    visuals.hyperlink_color = p.primary;
    visuals.warn_fg_color = p.warning;
    visuals.error_fg_color = p.error;

    // Material 3 Non-interactive containers (cards, frames, dividers)
    visuals.widgets.noninteractive.bg_fill = p.surface_container;
    visuals.widgets.noninteractive.weak_bg_fill = p.surface_container_low;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.outline_variant);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.on_surface_variant);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(12);

    // Material 3 Inactive controls (tonal buttons, text inputs, chips)
    visuals.widgets.inactive.bg_fill = p.surface_container_highest;
    visuals.widgets.inactive.weak_bg_fill = p.surface_container_high;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.outline_variant);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.on_surface);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(12);

    // Material 3 Hovered controls (tonal state layer)
    visuals.widgets.hovered.bg_fill = p.secondary_container;
    visuals.widgets.hovered.weak_bg_fill = p.secondary_container;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.2, p.primary);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.2, p.on_secondary_container);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(12);
    visuals.widgets.hovered.expansion = 0.5;

    // Material 3 Active / Pressed controls
    visuals.widgets.active.bg_fill = p.primary_container;
    visuals.widgets.active.weak_bg_fill = p.primary_container;
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, p.primary);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, p.on_primary_container);
    visuals.widgets.active.corner_radius = CornerRadius::same(12);

    // Open combo boxes / menus
    visuals.widgets.open.bg_fill = p.surface_container_highest;
    visuals.widgets.open.weak_bg_fill = p.surface_container_high;
    visuals.widgets.open.bg_stroke = Stroke::new(1.2, p.primary);
    visuals.widgets.open.fg_stroke = Stroke::new(1.2, p.on_surface);
    visuals.widgets.open.corner_radius = CornerRadius::same(12);

    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.button_padding = Vec2::new(11.0, 5.5);
        style.spacing.item_spacing = Vec2::new(8.0, 7.0);
        style.spacing.window_margin = Margin::same(12);
        style.spacing.combo_width = 140.0;
        style.interaction.tooltip_delay = 0.35;
    });

    p
}

/// Returns a Material 3 elevated tonal card `Frame`.
#[must_use]
pub fn m3_card_frame(p: &M3Palette) -> Frame {
    Frame::new()
        .fill(p.surface_container)
        .stroke(Stroke::new(1.0, p.outline_variant))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(12, 10))
}

/// Returns a Material 3 low-emphasis tonal panel `Frame` (sidebars / rails).
#[must_use]
pub fn m3_rail_frame(p: &M3Palette) -> Frame {
    Frame::new()
        .fill(p.surface_container_low)
        .stroke(Stroke::new(1.0, p.outline_variant))
        .inner_margin(Margin::symmetric(8, 6))
}

/// Renders a Material 3 tonal pill badge (e.g. `☕ JAVA EDITION` or `🪨 BEDROCK EDITION`).
pub fn m3_pill_badge(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(20))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(fg).strong().small());
        });
}

/// Renders a Material 3 segmented toggle pill button.
pub fn m3_segmented_button(
    ui: &mut Ui,
    selected: bool,
    label: &str,
    selected_bg: Color32,
    selected_fg: Color32,
    p: &M3Palette,
) -> egui::Response {
    let btn = if selected {
        egui::Button::new(RichText::new(label).color(selected_fg).strong())
            .fill(selected_bg)
            .stroke(Stroke::new(1.2, p.outline))
            .corner_radius(CornerRadius::same(18))
    } else {
        egui::Button::new(RichText::new(label).color(p.on_surface_variant))
            .fill(p.surface_container_high)
            .stroke(Stroke::new(1.0, p.outline_variant))
            .corner_radius(CornerRadius::same(18))
    };
    ui.add(btn)
}

/// Renders a Material 3 tonal action button (e.g. Refresh, Export).
pub fn m3_tonal_button(ui: &mut Ui, enabled: bool, label: &str, p: &M3Palette) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .color(p.on_secondary_container)
            .strong(),
    )
    .fill(p.secondary_container)
    .stroke(Stroke::new(1.0, p.outline_variant))
    .corner_radius(CornerRadius::same(18));
    ui.add_enabled(enabled, btn)
}

/// Renders a Material 3 prominent filled primary button.
pub fn m3_filled_button(ui: &mut Ui, enabled: bool, label: &str, p: &M3Palette) -> egui::Response {
    let btn = egui::Button::new(RichText::new(label).color(p.on_primary).strong())
        .fill(p.primary)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(18));
    ui.add_enabled(enabled, btn)
}
