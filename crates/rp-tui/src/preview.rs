use image::imageops::FilterType;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use rp_core::{ModelAsset, TextureAsset};

/// Renders raw PNG bytes (such as `pack.png` / `pack_icon.png`) as TrueColor ANSI half-blocks.
#[must_use]
pub fn render_png_bytes_halfblock(
    png_bytes: &[u8],
    max_cols: u16,
    max_rows: u16,
) -> Vec<Line<'static>> {
    if max_cols == 0 || max_rows == 0 {
        return Vec::new();
    }
    let decoded = match image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png) {
        Ok(img) => img.into_rgba8(),
        Err(_) => return Vec::new(),
    };
    render_rgba_halfblock(&decoded, max_cols, max_rows)
}

/// Renders a PNG `TextureAsset` as TrueColor ANSI half-block (`▀`) `ratatui` lines so textures
/// can be previewed directly inside a Linux terminal without X11/Wayland.
#[must_use]
pub fn render_texture_halfblock(
    tex: &TextureAsset,
    max_cols: u16,
    max_rows: u16,
) -> Vec<Line<'static>> {
    if max_cols == 0 || max_rows == 0 {
        return Vec::new();
    }

    let decoded = match tex.payload.decode() {
        Ok(img) => img,
        Err(err) => {
            return vec![Line::from(Span::styled(
                format!("Failed to decode PNG: {err}"),
                Style::default().fg(Color::Red),
            ))];
        }
    };

    let (orig_w, orig_h) = decoded.dimensions();
    if orig_w == 0 || orig_h == 0 {
        return Vec::new();
    }

    // If animated vertical strip, preview the first frame so it isn't squashed vertically.
    let frame_img = if let Some(anim) = &tex.animation {
        let fh = anim.frame_height.unwrap_or(orig_w).min(orig_h).max(1);
        image::imageops::crop_imm(decoded.as_ref(), 0, 0, orig_w, fh).to_image()
    } else {
        decoded.into_owned()
    };

    render_rgba_halfblock(&frame_img, max_cols, max_rows)
}

fn render_rgba_halfblock(
    frame_img: &image::RgbaImage,
    max_cols: u16,
    max_rows: u16,
) -> Vec<Line<'static>> {
    let (fw, fh) = frame_img.dimensions();
    if fw == 0 || fh == 0 {
        return Vec::new();
    }
    let max_px_w = u32::from(max_cols).min(48);
    let max_px_h = u32::from(max_rows).saturating_mul(2).min(48);

    let scale_x = max_px_w as f32 / fw as f32;
    let scale_y = max_px_h as f32 / fh as f32;
    let scale = scale_x.min(scale_y).min(2.0);
    let target_w = ((fw as f32 * scale).round() as u32).clamp(1, max_px_w);
    let target_h = ((fh as f32 * scale).round() as u32).clamp(1, max_px_h);

    let resized = if target_w == fw && target_h == fh {
        frame_img.clone()
    } else {
        image::imageops::resize(frame_img, target_w, target_h, FilterType::Nearest)
    };

    let (rw, rh) = resized.dimensions();
    let mut lines = Vec::with_capacity((rh as usize).div_ceil(2));

    let mut y = 0u32;
    while y < rh {
        let mut spans = Vec::with_capacity(rw as usize);
        for x in 0..rw {
            let top = resized.get_pixel(x, y);
            let bot = if y + 1 < rh {
                Some(resized.get_pixel(x, y + 1))
            } else {
                None
            };

            let top_col = blend_checkerboard(top.0, x, y);
            let bot_col = match bot {
                Some(p) => blend_checkerboard(p.0, x, y + 1),
                None => Color::Reset,
            };

            spans.push(Span::styled("▀", Style::default().fg(top_col).bg(bot_col)));
        }
        lines.push(Line::from(spans));
        y = y.saturating_add(2);
    }

    lines
}

fn blend_checkerboard(rgba: [u8; 4], x: u32, y: u32) -> Color {
    let alpha = f32::from(rgba[3]) / 255.0;
    let bg = if (x + y).is_multiple_of(2) {
        48.0_f32
    } else {
        68.0_f32
    };
    let r = (f32::from(rgba[0]) * alpha + bg * (1.0 - alpha)).round() as u8;
    let g = (f32::from(rgba[1]) * alpha + bg * (1.0 - alpha)).round() as u8;
    let b = (f32::from(rgba[2]) * alpha + bg * (1.0 - alpha)).round() as u8;
    Color::Rgb(r, g, b)
}

/// Formats detailed diagnostics for a 3D `ModelAsset` (bounds, rotations, UV stats, 26.3 fields).
#[must_use]
pub fn format_model_details(model: &ModelAsset) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let label = Style::default().fg(Color::Cyan);
    let val = Style::default().fg(Color::White);
    let accent = Style::default().fg(Color::Yellow);

    lines.push(Line::from(vec![
        Span::styled("Model ID:      ", label),
        Span::styled(model.id.clone(), val),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Category:      ", label),
        Span::styled(format!("{:?}", model.category), val),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Texture Size:  ", label),
        Span::styled(
            format!("{}x{}", model.texture_size[0], model.texture_size[1]),
            val,
        ),
    ]));
    if let Some(parent) = &model.parent {
        lines.push(Line::from(vec![
            Span::styled("Parent:        ", label),
            Span::styled(parent.clone(), val),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("Elements:      ", label),
        Span::styled(model.elements.len().to_string(), accent),
    ]));

    if !model.elements.is_empty() {
        let mut min_c = [f32::INFINITY; 3];
        let mut max_c = [f32::NEG_INFINITY; 3];
        let mut multi_axis = 0usize;
        let mut single_axis = 0usize;
        let mut unrotated = 0usize;
        let mut face_count = 0usize;
        let mut rotated_uvs = 0usize;
        let mut shade_overrides = 0usize;
        let mut light_emitters = 0usize;

        for el in &model.elements {
            for i in 0..3 {
                min_c[i] = min_c[i].min(el.from[i]).min(el.to[i]);
                max_c[i] = max_c[i].max(el.from[i]).max(el.to[i]);
            }
            match &el.rotation {
                Some(r) if r.extra_bedrock_euler.is_some() => multi_axis += 1,
                Some(_) => single_axis += 1,
                None => unrotated += 1,
            }
            if el.shade_direction_override.is_some() {
                shade_overrides += 1;
            }
            if el.light_emission.is_some_and(|v| v > 0) {
                light_emitters += 1;
            }
            face_count += el.faces.len();
            rotated_uvs += el.faces.values().filter(|f| f.rotation != 0).count();
        }

        let in_java_bounds = (0..3).all(|i| min_c[i] >= -16.001 && max_c[i] <= 32.001);
        let bounds_style = if in_java_bounds {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };

        lines.push(Line::from(vec![
            Span::styled("Bounds (Java): ", label),
            Span::styled(
                format!(
                    "X[{:.2}..{:.2}] Y[{:.2}..{:.2}] Z[{:.2}..{:.2}] {}",
                    min_c[0],
                    max_c[0],
                    min_c[1],
                    max_c[1],
                    min_c[2],
                    max_c[2],
                    if in_java_bounds {
                        "OK"
                    } else {
                        "OUT OF [-16,32]"
                    }
                ),
                bounds_style,
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Rotations:     ", label),
            Span::styled(
                format!(
                    "{multi_axis} multi-axis {{x,y,z}}, {single_axis} single-axis, {unrotated} unrotated"
                ),
                val,
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Faces / UVs:   ", label),
            Span::styled(
                format!("{face_count} faces ({rotated_uvs} with UV rotation)"),
                val,
            ),
        ]));
        if shade_overrides > 0 || light_emitters > 0 {
            lines.push(Line::from(vec![
                Span::styled("26.3 Props:    ", label),
                Span::styled(
                    format!(
                        "{shade_overrides} shade_direction_override, {light_emitters} light_emission"
                    ),
                    accent,
                ),
            ]));
        }
    }

    if !model.textures.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Texture Bindings:", accent)));
        for (k, v) in &model.textures {
            lines.push(Line::from(format!("  #{k} -> {v}")));
        }
    }

    if !model.display.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Display Slots:", accent)));
        for (slot, dt) in &model.display {
            lines.push(Line::from(format!(
                "  {slot:<22} rot={:?} trans={:?} scale={:?}",
                dt.rotation, dt.translation, dt.scale
            )));
        }
    }

    if !model.elements.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "First Elements Preview (up to 10):",
            accent,
        )));
        for (idx, el) in model.elements.iter().take(10).enumerate() {
            let group = el.group.as_deref().unwrap_or("-");
            let rot_desc = match &el.rotation {
                Some(r) => match r.extra_bedrock_euler {
                    Some(b) => format!("euler({:.1},{:.1},{:.1})", -b[0], -b[1], b[2]),
                    None => format!("{:?}({:.1}°)", r.axis, r.angle),
                },
                None => String::from("none"),
            };
            lines.push(Line::from(format!(
                "  [{idx:02}] {group:<10} [{:.1},{:.1},{:.1}]..[{:.1},{:.1},{:.1}] rot={rot_desc}",
                el.from[0], el.from[1], el.from[2], el.to[0], el.to[1], el.to[2]
            )));
        }
    }

    lines
}
