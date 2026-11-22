use egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use rp_cli::manager::project_model_3d;
use rp_core::{FaceDirection, ModelAsset};

/// Interactive 3D orbit camera state for inspecting `ModelAsset` cuboids in `rp-gui`.
#[derive(Debug, Clone)]
pub struct ModelViewportState {
    /// Camera yaw in degrees.
    pub yaw_deg: f32,
    /// Camera pitch in degrees.
    pub pitch_deg: f32,
    /// Zoom multiplier (`0.4..=3.0`).
    pub zoom: f32,
    /// Draw shaded filled quads.
    pub show_shaded: bool,
    /// Draw crisp cuboid wireframe outlines.
    pub show_wireframe: bool,
    /// Draw player-head `[4..12]^3` reference wireframe.
    pub show_head_ref: bool,
}

impl Default for ModelViewportState {
    fn default() -> Self {
        Self {
            yaw_deg: 35.0,
            pitch_deg: 22.0,
            zoom: 1.0,
            show_shaded: true,
            show_wireframe: true,
            show_head_ref: true,
        }
    }
}

impl ModelViewportState {
    /// Resets camera angle and zoom to default isometric perspective.
    pub fn reset(&mut self) {
        self.yaw_deg = 35.0;
        self.pitch_deg = 22.0;
        self.zoom = 1.0;
    }

    /// Renders an interactive 3D viewport for `model` inside `ui`.
    pub fn show(&mut self, ui: &mut Ui, model: &ModelAsset, height: f32) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_shaded, "Shaded");
            ui.checkbox(&mut self.show_wireframe, "Wireframe");
            ui.checkbox(&mut self.show_head_ref, "Head Box [4..12]");
            ui.add(egui::Slider::new(&mut self.zoom, 0.4..=3.0).text("Zoom"));
            if ui.button("Reset View").clicked() {
                self.reset();
            }
            ui.label(format!(
                "Yaw: {:.0}° | Pitch: {:.0}° (drag to orbit)",
                self.yaw_deg, self.pitch_deg
            ));
        });

        let avail_w = ui.available_width().max(200.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(avail_w, height), Sense::click_and_drag());

        if response.dragged() {
            let delta = response.drag_delta();
            self.yaw_deg = (self.yaw_deg + delta.x * 0.65).rem_euclid(360.0);
            self.pitch_deg = (self.pitch_deg + delta.y * 0.55).clamp(-88.0, 88.0);
        }
        if response.hovered() {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.1 {
                self.zoom = (self.zoom * (1.0 + scroll_y * 0.002)).clamp(0.4, 3.0);
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::from_rgb(20, 24, 32));
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, Color32::from_rgb(58, 72, 92)),
            egui::StrokeKind::Inside,
        );

        // Subtle viewport grid lines
        let center = rect.center();
        let grid_col = Color32::from_rgba_unmultiplied(120, 140, 170, 28);
        painter.line_segment(
            [
                Pos2::new(rect.left() + 8.0, center.y),
                Pos2::new(rect.right() - 8.0, center.y),
            ],
            Stroke::new(1.0, grid_col),
        );
        painter.line_segment(
            [
                Pos2::new(center.x, rect.top() + 8.0),
                Pos2::new(center.x, rect.bottom() - 8.0),
            ],
            Stroke::new(1.0, grid_col),
        );

        let scene = project_model_3d(model, self.yaw_deg, self.pitch_deg, self.show_head_ref);
        let scale = (rect.width().min(rect.height()) * 0.36) * self.zoom;
        let to_screen =
            |p: [f32; 2]| -> Pos2 { Pos2::new(center.x + p[0] * scale, center.y - p[1] * scale) };

        // Draw optional player-head reference wireframe behind the model
        if self.show_head_ref {
            let head_stroke = Stroke::new(1.1, Color32::from_rgba_unmultiplied(235, 190, 90, 120));
            for &(a, b) in &scene.head_ref_edges {
                painter.line_segment([to_screen(a), to_screen(b)], head_stroke);
            }
        }

        // Draw depth-sorted shaded quads
        if self.show_shaded {
            for quad in &scene.quads {
                let (br, bg, bb) = face_tint(quad.dir);
                let fill = Color32::from_rgba_unmultiplied(
                    (f32::from(br) * quad.shade).round() as u8,
                    (f32::from(bg) * quad.shade).round() as u8,
                    (f32::from(bb) * quad.shade).round() as u8,
                    225,
                );
                let outline = if self.show_wireframe {
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(210, 235, 255, 160))
                } else {
                    Stroke::NONE
                };
                let pts = vec![
                    to_screen(quad.pts[0]),
                    to_screen(quad.pts[1]),
                    to_screen(quad.pts[2]),
                    to_screen(quad.pts[3]),
                ];
                painter.add(Shape::convex_polygon(pts, fill, outline));
            }
        } else if self.show_wireframe {
            let wire_stroke = Stroke::new(1.2, Color32::from_rgb(90, 215, 255));
            for &(a, b) in &scene.edges {
                painter.line_segment([to_screen(a), to_screen(b)], wire_stroke);
            }
        }

        if model.elements.is_empty() {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "Parent-only model (0 geometry elements)",
                egui::FontId::proportional(14.0),
                Color32::GRAY,
            );
        }

        // Corner legend badge
        let legend_rect =
            Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(220.0, 20.0));
        painter.text(
            legend_rect.min,
            egui::Align2::LEFT_TOP,
            format!(
                "{} elements | {} quads",
                model.elements.len(),
                scene.quads.len()
            ),
            egui::FontId::monospace(11.0),
            Color32::from_rgb(165, 195, 225),
        );
    }
}

fn face_tint(dir: FaceDirection) -> (u8, u8, u8) {
    match dir {
        FaceDirection::Up => (120, 215, 245),
        FaceDirection::Down => (80, 145, 190),
        FaceDirection::North => (105, 195, 235),
        FaceDirection::South => (95, 180, 220),
        FaceDirection::East => (115, 205, 225),
        FaceDirection::West => (90, 175, 215),
    }
}
