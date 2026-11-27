use egui::{Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};

use rp_core::{FaceDirection, ModelAsset, ModelElement, RotationAxis};
use rp_transforms::model::euler_zyx_to_mat3;

/// Blockbench-style interactive 3D orbit/pan/zoom camera state for inspecting and editing
/// `ModelAsset` cuboids, bones, grids, axes, bounding boxes, and rotation pivots.
#[derive(Debug, Clone)]
pub struct ModelViewportState {
    /// Camera yaw in degrees.
    pub yaw_deg: f32,
    /// Camera pitch in degrees.
    pub pitch_deg: f32,
    /// Zoom multiplier (`0.3..=4.0`).
    pub zoom: f32,
    /// Screen-space pan offset in normalized viewport units `[dx, dy]`.
    pub pan: [f32; 2],
    /// Draw shaded filled quads.
    pub show_shaded: bool,
    /// Draw cuboid wireframe edges.
    pub show_wireframe: bool,
    /// Draw Minecraft 16×16 block floor grid at `Y = 0`.
    pub show_block_grid: bool,
    /// Draw XYZ axis triad at `[0, 0, 0]`.
    pub show_axes: bool,
    /// Draw model overall bounding box (`min..max`).
    pub show_bounds: bool,
    /// Draw selected element's rotation pivot crosshair & tether.
    pub show_pivot: bool,
    /// Draw player-head `[4..12]^3` reference wireframe.
    pub show_head_ref: bool,
}

impl Default for ModelViewportState {
    fn default() -> Self {
        Self {
            yaw_deg: 35.0,
            pitch_deg: 24.0,
            zoom: 1.0,
            pan: [0.0, 0.0],
            show_shaded: true,
            show_wireframe: true,
            show_block_grid: true,
            show_axes: true,
            show_bounds: false,
            show_pivot: true,
            show_head_ref: false,
        }
    }
}

/// Model world-space bounds & Java `[-16, 32]` validation summary.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelBoundsInfo {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub within_java_limits: bool,
    pub out_of_bounds_elements: Vec<usize>,
}

/// Computes exact rotated world-space corners for a single `ModelElement` using `ZYX` Euler math.
#[must_use]
pub fn element_world_corners(el: &ModelElement) -> [[f64; 3]; 8] {
    let f = [
        f64::from(el.from[0]),
        f64::from(el.from[1]),
        f64::from(el.from[2]),
    ];
    let t = [
        f64::from(el.to[0]),
        f64::from(el.to[1]),
        f64::from(el.to[2]),
    ];
    let raw = [
        [f[0], f[1], f[2]],
        [t[0], f[1], f[2]],
        [t[0], t[1], f[2]],
        [f[0], t[1], f[2]],
        [f[0], f[1], t[2]],
        [t[0], f[1], t[2]],
        [t[0], t[1], t[2]],
        [f[0], t[1], t[2]],
    ];

    let Some(rot) = &el.rotation else {
        return raw;
    };
    let origin = [
        f64::from(rot.origin[0]),
        f64::from(rot.origin[1]),
        f64::from(rot.origin[2]),
    ];
    let deg = if let Some(b) = rot.extra_bedrock_euler {
        [-f64::from(b[0]), -f64::from(b[1]), f64::from(b[2])]
    } else {
        match rot.axis {
            RotationAxis::X => [f64::from(rot.angle), 0.0, 0.0],
            RotationAxis::Y => [0.0, f64::from(rot.angle), 0.0],
            RotationAxis::Z => [0.0, 0.0, f64::from(rot.angle)],
        }
    };
    let m = euler_zyx_to_mat3(deg);
    let mut out = [[0.0_f64; 3]; 8];
    for (i, p) in raw.iter().enumerate() {
        let rx = p[0] - origin[0];
        let ry = p[1] - origin[1];
        let rz = p[2] - origin[2];
        out[i] = [
            m[0][0] * rx + m[0][1] * ry + m[0][2] * rz + origin[0],
            m[1][0] * rx + m[1][1] * ry + m[1][2] * rz + origin[1],
            m[2][0] * rx + m[2][1] * ry + m[2][2] * rz + origin[2],
        ];
    }
    out
}

/// Computes exact world-space bounding box and checks Java's `[-16, 32]` element limits.
#[must_use]
pub fn compute_model_bounds(model: &ModelAsset) -> ModelBoundsInfo {
    if model.elements.is_empty() {
        return ModelBoundsInfo {
            min: [0.0; 3],
            max: [16.0; 3],
            within_java_limits: true,
            out_of_bounds_elements: Vec::new(),
        };
    }

    let mut min_c = [f64::INFINITY; 3];
    let mut max_c = [f64::NEG_INFINITY; 3];
    let mut oob = Vec::new();

    for (idx, el) in model.elements.iter().enumerate() {
        let raw_oob = el
            .from
            .iter()
            .chain(el.to.iter())
            .any(|&v| !(-16.001..=32.001).contains(&v));
        if raw_oob {
            oob.push(idx);
        }
        for c in element_world_corners(el) {
            for i in 0..3 {
                min_c[i] = min_c[i].min(c[i]);
                max_c[i] = max_c[i].max(c[i]);
            }
        }
    }

    ModelBoundsInfo {
        min: [min_c[0] as f32, min_c[1] as f32, min_c[2] as f32],
        max: [max_c[0] as f32, max_c[1] as f32, max_c[2] as f32],
        within_java_limits: oob.is_empty(),
        out_of_bounds_elements: oob,
    }
}

const CUBE_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (3, 0),
    (4, 5),
    (5, 6),
    (6, 7),
    (7, 4),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

const FACE_INDICES: [(FaceDirection, [usize; 4]); 6] = [
    (FaceDirection::North, [0, 3, 2, 1]),
    (FaceDirection::South, [4, 5, 6, 7]),
    (FaceDirection::East, [1, 2, 6, 5]),
    (FaceDirection::West, [0, 4, 7, 3]),
    (FaceDirection::Up, [3, 7, 6, 2]),
    (FaceDirection::Down, [0, 1, 5, 4]),
];

struct RenderQuad {
    pts: [Pos2; 4],
    depth: f32,
    shade: f32,
    dir: FaceDirection,
    selected: bool,
}

impl ModelViewportState {
    /// Resets camera angle, pan, and zoom to default Blockbench isometric perspective.
    pub fn reset(&mut self) {
        self.yaw_deg = 35.0;
        self.pitch_deg = 24.0;
        self.zoom = 1.0;
        self.pan = [0.0, 0.0];
    }

    /// Renders an interactive Blockbench-style 3D viewport for `model` inside `ui`.
    /// Highlights `selected_element` (cube edges + rotation pivot crosshair) when set.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &ModelAsset,
        selected_element: Option<usize>,
        height: f32,
    ) {
        // Compact camera & overlay toolbar
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_shaded, "Shaded");
            ui.checkbox(&mut self.show_wireframe, "Wire");
            ui.checkbox(&mut self.show_block_grid, "16u Grid");
            ui.checkbox(&mut self.show_axes, "Axes");
            ui.checkbox(&mut self.show_pivot, "Pivot");
            ui.checkbox(&mut self.show_bounds, "Bounds");
            ui.checkbox(&mut self.show_head_ref, "Head");
            ui.separator();
            if ui.small_button("Iso").clicked() {
                self.reset();
            }
            if ui.small_button("Front").clicked() {
                self.yaw_deg = 0.0;
                self.pitch_deg = 0.0;
            }
            if ui.small_button("Side").clicked() {
                self.yaw_deg = 90.0;
                self.pitch_deg = 0.0;
            }
            if ui.small_button("Top").clicked() {
                self.yaw_deg = 0.0;
                self.pitch_deg = 85.0;
            }
            ui.add(egui::Slider::new(&mut self.zoom, 0.3..=4.0).text("Zoom"));
        });

        let avail_w = ui.available_width().max(220.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(avail_w, height), Sense::click_and_drag());

        // Left-drag: Orbit; Right/Middle-drag or Shift+Left-drag: Pan; Wheel: Zoom
        let shift_down = ui.input(|i| i.modifiers.shift);
        if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary) && shift_down)
        {
            let delta = response.drag_delta();
            self.pan[0] += delta.x;
            self.pan[1] += delta.y;
        } else if response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            self.yaw_deg = (self.yaw_deg + delta.x * 0.65).rem_euclid(360.0);
            self.pitch_deg = (self.pitch_deg + delta.y * 0.55).clamp(-88.0, 88.0);
        }
        if response.hovered() {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.1 {
                self.zoom = (self.zoom * (1.0 + scroll_y * 0.002)).clamp(0.3, 4.0);
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::from_rgb(20, 24, 32));
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0_f32, Color32::from_rgb(58, 72, 92)),
            egui::StrokeKind::Inside,
        );

        let bounds = compute_model_bounds(model);
        let world_center = [8.0_f64, 8.0, 8.0];
        let span = f64::from(
            (bounds.max[0] - bounds.min[0])
                .max(bounds.max[1] - bounds.min[1])
                .max(bounds.max[2] - bounds.min[2])
                .max(16.0),
        );
        let radius = (span * 0.68).max(10.0);

        let (sy, cy) = f64::from(self.yaw_deg).to_radians().sin_cos();
        let (sp, cp) = f64::from(self.pitch_deg).to_radians().sin_cos();
        let screen_center = rect.center() + Vec2::new(self.pan[0], self.pan[1]);
        let scale = (rect.width().min(rect.height()) * 0.38) * self.zoom;

        let project_cam = |p: [f64; 3]| -> [f64; 3] {
            let x = (p[0] - world_center[0]) / radius;
            let y = (p[1] - world_center[1]) / radius;
            let z = (p[2] - world_center[2]) / radius;
            let x1 = cy * x + sy * z;
            let z1 = -sy * x + cy * z;
            let y2 = cp * y - sp * z1;
            let z2 = sp * y + cp * z1;
            [x1, y2, z2]
        };
        let to_screen = |cam: [f64; 3]| -> Pos2 {
            Pos2::new(
                screen_center.x + cam[0] as f32 * scale,
                screen_center.y - cam[1] as f32 * scale,
            )
        };

        // 1. 16-unit Minecraft block floor grid at Y = 0
        if self.show_block_grid {
            let minor = Stroke::new(0.8_f32, Color32::from_rgba_unmultiplied(110, 130, 160, 38));
            let major = Stroke::new(1.2_f32, Color32::from_rgba_unmultiplied(140, 175, 215, 95));
            for i in 0..=16 {
                let f = f64::from(i);
                let stroke = if i == 0 || i == 8 || i == 16 {
                    major
                } else {
                    minor
                };
                let a = to_screen(project_cam([f, 0.0, 0.0]));
                let b = to_screen(project_cam([f, 0.0, 16.0]));
                painter.line_segment([a, b], stroke);
                let c = to_screen(project_cam([0.0, 0.0, f]));
                let d = to_screen(project_cam([16.0, 0.0, f]));
                painter.line_segment([c, d], stroke);
            }
        }

        // 2. XYZ Axis triad at [0, 0, 0]
        if self.show_axes {
            let o = to_screen(project_cam([0.0, 0.0, 0.0]));
            let axes = [
                ([18.0, 0.0, 0.0], Color32::from_rgb(240, 90, 90), "+X"),
                ([0.0, 18.0, 0.0], Color32::from_rgb(95, 230, 120), "+Y"),
                ([0.0, 0.0, 18.0], Color32::from_rgb(95, 165, 250), "+Z"),
            ];
            for (end, col, label) in axes {
                let p = to_screen(project_cam(end));
                painter.line_segment([o, p], Stroke::new(1.8_f32, col));
                painter.text(
                    p + Vec2::new(4.0, -4.0),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::monospace(10.0),
                    col,
                );
            }
        }

        // 3. Optional Player Head [4..12]^3 reference wireframe
        if self.show_head_ref {
            let head_stroke = Stroke::new(1.2_f32, Color32::from_rgba_unmultiplied(235, 190, 90, 130));
            draw_box_wireframe(
                &painter,
                [4.0, 4.0, 4.0],
                [12.0, 12.0, 12.0],
                &project_cam,
                &to_screen,
                head_stroke,
            );
        }

        // 4. Optional overall model bounding box
        if self.show_bounds && !model.elements.is_empty() {
            let b_col = if bounds.within_java_limits {
                Color32::from_rgba_unmultiplied(90, 230, 160, 150)
            } else {
                Color32::from_rgba_unmultiplied(255, 95, 95, 210)
            };
            draw_box_wireframe(
                &painter,
                [
                    f64::from(bounds.min[0]),
                    f64::from(bounds.min[1]),
                    f64::from(bounds.min[2]),
                ],
                [
                    f64::from(bounds.max[0]),
                    f64::from(bounds.max[1]),
                    f64::from(bounds.max[2]),
                ],
                &project_cam,
                &to_screen,
                Stroke::new(1.4_f32, b_col),
            );
        }

        // 5. Collect and depth-sort all cuboid faces
        let light_dir = [0.38_f64, 0.78, 0.49];
        let mut quads: Vec<RenderQuad> = Vec::new();
        let mut selected_edges: Vec<(Pos2, Pos2)> = Vec::new();
        let mut all_wire_edges: Vec<(Pos2, Pos2)> = Vec::new();

        for (idx, el) in model.elements.iter().enumerate() {
            let is_sel = selected_element == Some(idx);
            let corners = element_world_corners(el);
            let mut cam = [[0.0_f64; 3]; 8];
            let mut scr = [Pos2::ZERO; 8];
            for i in 0..8 {
                cam[i] = project_cam(corners[i]);
                scr[i] = to_screen(cam[i]);
            }

            for &(a, b) in &CUBE_EDGES {
                if is_sel {
                    selected_edges.push((scr[a], scr[b]));
                } else if !self.show_shaded && self.show_wireframe {
                    all_wire_edges.push((scr[a], scr[b]));
                }
            }

            if self.show_shaded {
                for &(dir, fi) in &FACE_INDICES {
                    let p0 = cam[fi[0]];
                    let p1 = cam[fi[1]];
                    let p2 = cam[fi[2]];
                    let p3 = cam[fi[3]];
                    let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
                    let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
                    let nx = e1[1] * e2[2] - e1[2] * e2[1];
                    let ny = e1[2] * e2[0] - e1[0] * e2[2];
                    let nz = e1[0] * e2[1] - e1[1] * e2[0];
                    let nlen = (nx * nx + ny * ny + nz * nz).sqrt();
                    if nlen < 1e-8 {
                        continue;
                    }
                    let (nx, ny, nz) = (nx / nlen, ny / nlen, nz / nlen);
                    if nz < -0.05 {
                        continue;
                    }
                    let dot = (nx * light_dir[0] + ny * light_dir[1] + nz * light_dir[2]).abs();
                    let shade = (0.38 + 0.62 * dot).clamp(0.35, 1.0) as f32;
                    let depth = ((p0[2] + p1[2] + p2[2] + p3[2]) * 0.25) as f32;

                    quads.push(RenderQuad {
                        pts: [scr[fi[0]], scr[fi[1]], scr[fi[2]], scr[fi[3]]],
                        depth,
                        shade,
                        dir,
                        selected: is_sel,
                    });
                }
            }
        }

        quads.sort_by(|a, b| a.depth.total_cmp(&b.depth));

        for quad in &quads {
            let (br, bg, bb) = if quad.selected {
                (255, 195, 85)
            } else {
                face_tint(quad.dir)
            };
            let fill = Color32::from_rgba_unmultiplied(
                (f32::from(br) * quad.shade).round() as u8,
                (f32::from(bg) * quad.shade).round() as u8,
                (f32::from(bb) * quad.shade).round() as u8,
                if quad.selected { 235 } else { 215 },
            );
            let outline = if self.show_wireframe {
                Stroke::new(
                    if quad.selected { 1.4_f32 } else { 0.9_f32 },
                    if quad.selected {
                        Color32::from_rgb(255, 235, 130)
                    } else {
                        Color32::from_rgba_unmultiplied(200, 230, 255, 145)
                    },
                )
            } else {
                Stroke::NONE
            };
            painter.add(Shape::convex_polygon(quad.pts.to_vec(), fill, outline));
        }

        for &(a, b) in &all_wire_edges {
            painter.line_segment([a, b], Stroke::new(1.1_f32, Color32::from_rgb(90, 210, 255)));
        }
        // Highlight selected element edges clearly on top
        for &(a, b) in &selected_edges {
            painter.line_segment([a, b], Stroke::new(2.0_f32, Color32::from_rgb(255, 215, 70)));
        }

        // 6. Selected element rotation pivot crosshair + tether to cube center
        if self.show_pivot
            && let Some(sel_idx) = selected_element
            && let Some(el) = model.elements.get(sel_idx)
        {
            let pivot_world = el
                .rotation
                .as_ref()
                .map(|r| {
                    [
                        f64::from(r.origin[0]),
                        f64::from(r.origin[1]),
                        f64::from(r.origin[2]),
                    ]
                })
                .unwrap_or([
                    0.5 * f64::from(el.from[0] + el.to[0]),
                    0.5 * f64::from(el.from[1] + el.to[1]),
                    0.5 * f64::from(el.from[2] + el.to[2]),
                ]);
            let corners = element_world_corners(el);
            let mut cube_center = [0.0_f64; 3];
            for c in &corners {
                for i in 0..3 {
                    cube_center[i] += c[i] * 0.125;
                }
            }
            let p_scr = to_screen(project_cam(pivot_world));
            let c_scr = to_screen(project_cam(cube_center));
            let pivot_col = Color32::from_rgb(255, 95, 225);
            painter.line_segment([p_scr, c_scr], Stroke::new(1.2_f32, pivot_col));
            painter.circle_filled(p_scr, 4.5, pivot_col);
            painter.circle_stroke(p_scr, 7.0, Stroke::new(1.3_f32, Color32::WHITE));
            painter.text(
                p_scr + Vec2::new(8.0, -8.0),
                egui::Align2::LEFT_CENTER,
                format!(
                    "Pivot [{:.1}, {:.1}, {:.1}]",
                    pivot_world[0], pivot_world[1], pivot_world[2]
                ),
                egui::FontId::monospace(10.0),
                pivot_col,
            );
        }

        if model.elements.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Parent-only model (0 geometry elements)",
                egui::FontId::proportional(14.0),
                Color32::GRAY,
            );
        }

        // HUD Overlay: element count, camera angles, bounds status
        let hud_color = if bounds.within_java_limits {
            Color32::from_rgb(155, 215, 235)
        } else {
            Color32::from_rgb(255, 120, 120)
        };
        let bounds_tag = if bounds.within_java_limits {
            "Bounds OK [-16..32]"
        } else {
            "OUT OF JAVA BOUNDS [-16..32]"
        };
        painter.text(
            rect.min + Vec2::new(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{} elems | Yaw {:.0}° Pitch {:.0}° | {bounds_tag}",
                model.elements.len(),
                self.yaw_deg,
                self.pitch_deg
            ),
            egui::FontId::monospace(11.0),
            hud_color,
        );
        painter.text(
            Pos2::new(rect.left() + 8.0, rect.bottom() - 16.0),
            egui::Align2::LEFT_TOP,
            "LMB Drag: Orbit | RMB / Shift+Drag: Pan | Scroll: Zoom",
            egui::FontId::monospace(10.0),
            Color32::from_rgb(130, 145, 165),
        );
    }
}

fn draw_box_wireframe(
    painter: &egui::Painter,
    min: [f64; 3],
    max: [f64; 3],
    project_cam: &impl Fn([f64; 3]) -> [f64; 3],
    to_screen: &impl Fn([f64; 3]) -> Pos2,
    stroke: Stroke,
) {
    let corners = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ];
    let scr: Vec<Pos2> = corners.iter().map(|&c| to_screen(project_cam(c))).collect();
    for &(a, b) in &CUBE_EDGES {
        painter.line_segment([scr[a], scr[b]], stroke);
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rp_core::{ElementRotation, ModelCategory};

    use super::*;

    #[test]
    fn element_world_corners_and_bounds_math_match_zyx_rotation() {
        // Cube [10, 0, 8]..[12, 2, 10] rotated +90 deg around Y at pivot [8, 0, 8].
        // Point (10, 0, 8) relative (+2, 0, 0) rotated +90° around Y -> (0, 0, -2) => (8, 0, 6)!
        let el = ModelElement {
            group: Some(String::from("arm")),
            from: [10.0, 0.0, 8.0],
            to: [12.0, 2.0, 10.0],
            rotation: Some(ElementRotation {
                origin: [8.0, 0.0, 8.0],
                axis: RotationAxis::Y,
                angle: 90.0,
                rescale: false,
                extra_bedrock_euler: None,
            }),
            faces: BTreeMap::new(),
            shade: true,
            shade_direction_override: None,
            light_emission: None,
        };

        let corners = element_world_corners(&el);
        // Corner 0 was [10, 0, 8] -> [8, 0, 6]
        assert!((corners[0][0] - 8.0).abs() < 1e-6);
        assert!((corners[0][1] - 0.0).abs() < 1e-6);
        assert!((corners[0][2] - 6.0).abs() < 1e-6);

        let ok_model = ModelAsset {
            id: String::from("block/ok"),
            category: ModelCategory::Block,
            parent: None,
            textures: BTreeMap::new(),
            texture_size: [16, 16],
            ambient_occlusion: true,
            elements: vec![el],
            display: BTreeMap::new(),
            has_overrides: false,
            overrides_count: 0,
        };
        let bounds = compute_model_bounds(&ok_model);
        assert!(bounds.within_java_limits);
        assert!(bounds.out_of_bounds_elements.is_empty());

        // Out-of-bounds element > 32 triggers bounds warning
        let mut bad_model = ok_model;
        bad_model.elements[0].to[1] = 40.0;
        let bad_bounds = compute_model_bounds(&bad_model);
        assert!(!bad_bounds.within_java_limits);
        assert_eq!(bad_bounds.out_of_bounds_elements, vec![0]);
    }
}
