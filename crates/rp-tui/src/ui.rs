use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table,
    TableState, Tabs, Wrap,
};

use rp_core::{BedrockGeoVersion, Edition, RotationSnapMode};

use crate::app::{ActiveTab, App, ExplorerItem, InputMode};
use crate::preview::format_model_details;
use crate::scanner::{format_bytes, project_model_3d};

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Renders the entire `rp-tui` interface for a single terminal frame.
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header + Tabs
            Constraint::Min(10),   // Main content
            Constraint::Length(3), // Status bar + key hints
        ])
        .split(frame.area());

    render_header(frame, app, chunks[0]);

    match app.active_tab {
        ActiveTab::Packs => render_packs_tab(frame, app, chunks[1]),
        ActiveTab::Explorer => render_explorer_tab(frame, app, chunks[1]),
        ActiveTab::Convert => render_convert_tab(frame, app, chunks[1]),
        ActiveTab::BatchAudit => render_batch_tab(frame, app, chunks[1]),
        ActiveTab::Compare => render_compare_tab(frame, app, chunks[1]),
        ActiveTab::VersionsLogs => render_versions_tab(frame, app, chunks[1]),
    }

    render_footer(frame, app, chunks[2]);

    if app.show_help {
        render_help_overlay(frame);
    } else if app.input_mode != InputMode::Normal {
        render_input_overlay(frame, &app.input_mode);
    }
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let titles: Vec<Line<'_>> = ActiveTab::ALL
        .iter()
        .map(|tab| Line::from(Span::raw(tab.title())))
        .collect();

    let title_str = if let Some(msg) = &app.busy_message {
        let spin = SPINNER_FRAMES[(app.tick_count as usize) % SPINNER_FRAMES.len()];
        format!(" rp-tui [Linux] — {spin} {msg} ")
    } else {
        format!(
            " rp-tui [Linux] — {} [Sort: {}] ",
            app.scan_dir.display(),
            app.pack_sort.label()
        )
    };

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    title_str,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .select(app.active_tab.index())
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, area);
}

fn render_packs_tab(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    let filtered = app.filtered_pack_indices();
    let header = Row::new(vec![
        Cell::from("Sel"),
        Cell::from("Pack"),
        Cell::from("Edition"),
        Cell::from("Version"),
        Cell::from("Tex"),
        Cell::from("3D"),
        Cell::from("Size"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row<'_>> = filtered
        .iter()
        .filter_map(|&idx| app.packs.get(idx))
        .map(|pack| {
            let sel = if pack.selected { "[x]" } else { "[ ]" };
            let name = if pack.is_dir {
                format!("{}/", pack.file_name)
            } else {
                pack.file_name.clone()
            };
            let (ed_cell, ver_str, tex_str, model_str) = match &pack.summary {
                Ok(s) => {
                    let ed_style = match s.edition {
                        Edition::Java => Style::default().fg(Color::Cyan),
                        Edition::Bedrock => Style::default().fg(Color::Green),
                    };
                    let ver = match s.edition {
                        Edition::Java => s
                            .java_pack_format
                            .map(|f| format!("fmt {f}"))
                            .unwrap_or_else(|| String::from("-")),
                        Edition::Bedrock => s
                            .bedrock_min_engine_version
                            .map(|v| format!("{}.{}.{}", v[0], v[1], v[2]))
                            .unwrap_or_else(|| String::from("-")),
                    };
                    (
                        Cell::from(s.edition.to_string()).style(ed_style),
                        ver,
                        s.texture_count.to_string(),
                        s.model_count.to_string(),
                    )
                }
                Err(_) => (
                    Cell::from("ERROR").style(Style::default().fg(Color::Red)),
                    String::from("-"),
                    String::from("-"),
                    String::from("-"),
                ),
            };
            Row::new(vec![
                Cell::from(sel),
                Cell::from(name),
                ed_cell,
                Cell::from(ver_str),
                Cell::from(tex_str),
                Cell::from(model_str),
                Cell::from(format_bytes(pack.size_bytes)),
            ])
        })
        .collect();

    let title = if app.pack_filter.is_empty() {
        format!(" Resource Packs ({}) ", filtered.len())
    } else {
        format!(
            " Resource Packs ({} | filter: '{}') ",
            filtered.len(),
            app.pack_filter
        )
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Min(18),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");

    let mut state = TableState::default();
    if !filtered.is_empty() {
        state.select(Some(app.selected_pack_idx.min(filtered.len() - 1)));
    }
    frame.render_stateful_widget(table, cols[0], &mut state);

    // Right pane: Live Summary + Pack Icon Half-Block Preview
    let detail_lines = if let Some(pack) = app.current_pack() {
        match &pack.summary {
            Ok(s) => {
                let mut lines = vec![
                    Line::from(vec![
                        Span::styled("Name:        ", Style::default().fg(Color::Cyan)),
                        Span::raw(s.name.clone()),
                    ]),
                    Line::from(vec![
                        Span::styled("File:        ", Style::default().fg(Color::Cyan)),
                        Span::raw(pack.file_name.clone()),
                    ]),
                    Line::from(vec![
                        Span::styled("Edition:     ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            s.edition.to_string(),
                            Style::default().fg(match s.edition {
                                Edition::Java => Color::Cyan,
                                Edition::Bedrock => Color::Green,
                            }),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Description: ", Style::default().fg(Color::Cyan)),
                        Span::raw(s.description.clone()),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("Files:       ", Style::default().fg(Color::Yellow)),
                        Span::raw(format!(
                            "{} ({})",
                            s.total_files,
                            format_bytes(pack.size_bytes)
                        )),
                    ]),
                    Line::from(vec![
                        Span::styled("Textures:    ", Style::default().fg(Color::Yellow)),
                        Span::raw(format!(
                            "{} ({} animated)",
                            s.texture_count, s.animated_texture_count
                        )),
                    ]),
                    Line::from(vec![
                        Span::styled("3D Models:   ", Style::default().fg(Color::Yellow)),
                        Span::raw(s.model_count.to_string()),
                    ]),
                    Line::from(vec![
                        Span::styled("Sounds:      ", Style::default().fg(Color::Yellow)),
                        Span::raw(format!(
                            "{} events / {} .ogg files",
                            s.sound_event_count, s.sound_file_count
                        )),
                    ]),
                    Line::from(vec![
                        Span::styled("Locales:     ", Style::default().fg(Color::Yellow)),
                        Span::raw(s.locale_count.to_string()),
                    ]),
                    Line::from(vec![
                        Span::styled("Skipped:     ", Style::default().fg(Color::Yellow)),
                        Span::raw(s.unsupported_asset_count.to_string()),
                    ]),
                    Line::from(vec![
                        Span::styled("Warnings:    ", Style::default().fg(Color::Yellow)),
                        Span::raw(s.warnings.len().to_string()),
                    ]),
                ];
                if !app.cached_icon_lines.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "Pack Icon:",
                        Style::default().fg(Color::Magenta),
                    )));
                    lines.extend(app.cached_icon_lines.iter().cloned());
                }
                if !s.warnings.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "Recent Diagnostics:",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    for w in s.warnings.iter().take(4) {
                        lines.push(Line::from(format!(" • {}: {}", w.asset, w.message)));
                    }
                }
                lines
            }
            Err(err) => vec![
                Line::from(Span::styled(
                    "Failed to parse pack:",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(err.clone()),
            ],
        }
    } else {
        vec![
            Line::from("No resource packs found in this directory."),
            Line::from(""),
            Line::from("Press 'd' to switch directory or 'r' to rescan."),
        ]
    };

    let summary = Paragraph::new(detail_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Pack Details "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(summary, cols[1]);
}

fn render_explorer_tab(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(loaded) = &app.loaded_pack else {
        let empty = Paragraph::new(vec![
            Line::from("No pack loaded in Explorer yet."),
            Line::from(""),
            Line::from("Go to [1:Packs], highlight a resource pack, and press Enter (or 'i')."),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Asset Explorer "),
        );
        frame.render_widget(empty, area);
        return;
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    let items: Vec<ListItem<'_>> = app
        .explorer_items
        .iter()
        .map(|it| {
            let badge_color = match it {
                ExplorerItem::Texture(_) => Color::Magenta,
                ExplorerItem::Model(_) => Color::Cyan,
                ExplorerItem::Attachable(_) => Color::Yellow,
                ExplorerItem::SoundFile(_) | ExplorerItem::SoundEvent(_) => Color::Green,
                ExplorerItem::Locale(_) => Color::Blue,
                ExplorerItem::Passthrough(_) => Color::Gray,
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", it.badge()),
                    Style::default()
                        .fg(badge_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(it.id().to_string()),
            ]))
        })
        .collect();

    let list_title = format!(
        " Assets ({}) [Filter: {} | '{}'] ",
        app.explorer_items.len(),
        app.explorer_category.label(),
        app.explorer_filter
    );
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(list_title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    if !app.explorer_items.is_empty() {
        state.select(Some(
            app.selected_asset_idx.min(app.explorer_items.len() - 1),
        ));
    }
    frame.render_stateful_widget(list, cols[0], &mut state);

    // Special split for 3D Models: Top = Live Braille 3D Wireframe Canvas, Bottom = Metadata!
    if let Some(ExplorerItem::Model(id)) = app.explorer_items.get(app.selected_asset_idx)
        && let Some(model) = loaded.ir.models.get(id)
        && !model.elements.is_empty()
    {
        let right_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(cols[1]);

        let scene = project_model_3d(
            model,
            app.model_yaw_deg,
            app.model_pitch_deg,
            app.show_head_ref,
        );
        let canvas_title = format!(
            " 3D Wireframe [yaw={:.0}° pitch={:.0}° | a/d/w/s:Orbit | h:HeadRef={}] ",
            app.model_yaw_deg,
            app.model_pitch_deg,
            if app.show_head_ref { "ON" } else { "OFF" }
        );
        let canvas = Canvas::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(canvas_title),
            )
            .marker(Marker::Braille)
            .x_bounds([-1.4, 1.4])
            .y_bounds([-1.2, 1.2])
            .paint(move |ctx| {
                for &(a, b) in &scene.head_ref_edges {
                    ctx.draw(&CanvasLine {
                        x1: f64::from(a[0]),
                        y1: f64::from(a[1]),
                        x2: f64::from(b[0]),
                        y2: f64::from(b[1]),
                        color: Color::DarkGray,
                    });
                }
                for &(a, b) in &scene.edges {
                    ctx.draw(&CanvasLine {
                        x1: f64::from(a[0]),
                        y1: f64::from(a[1]),
                        x2: f64::from(b[0]),
                        y2: f64::from(b[1]),
                        color: Color::Cyan,
                    });
                }
            });
        frame.render_widget(canvas, right_split[0]);

        let detail = Paragraph::new(format_model_details(model))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 3D Model Diagnostics (PgUp/PgDn to scroll) "),
            )
            .scroll((app.detail_scroll, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(detail, right_split[1]);
        return;
    }

    // Standard right pane for Textures, parent-only Models, Attachables, Sounds, Locales, Raw
    let mut detail_lines: Vec<Line<'static>> = Vec::new();
    match app.explorer_items.get(app.selected_asset_idx) {
        Some(ExplorerItem::Texture(id)) => {
            if let Some(tex) = loaded.ir.textures.get(id) {
                let dims = match (tex.width, tex.height) {
                    (Some(w), Some(h)) => format!("{w}x{h}"),
                    _ => tex
                        .payload
                        .dimensions()
                        .map(|(w, h)| format!("{w}x{h}"))
                        .unwrap_or_else(|_| String::from("?")),
                };
                let tier_label = tex
                    .resolution_tier
                    .map(|t| format!(" [{t}]"))
                    .unwrap_or_default();
                detail_lines.push(Line::from(vec![
                    Span::styled("Texture ID:  ", Style::default().fg(Color::Cyan)),
                    Span::raw(tex.id.clone()),
                ]));
                detail_lines.push(Line::from(vec![
                    Span::styled("Dimensions:  ", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{dims}{tier_label} ({:?})", tex.category)),
                ]));
                if let Some(key) = &tex.atlas_key {
                    detail_lines.push(Line::from(vec![
                        Span::styled("Atlas Key:   ", Style::default().fg(Color::Cyan)),
                        Span::raw(key.clone()),
                    ]));
                }
                if let Some(anim) = &tex.animation {
                    detail_lines.push(Line::from(vec![
                        Span::styled("Animation:   ", Style::default().fg(Color::Yellow)),
                        Span::raw(format!(
                            "frametime={} ticks, interpolate={}, explicit_frames={}",
                            anim.frametime,
                            anim.interpolate,
                            anim.frames.len()
                        )),
                    ]));
                }
                detail_lines.push(Line::from(""));
                detail_lines.push(Line::from(Span::styled(
                    "Terminal TrueColor Preview (half-block):",
                    Style::default().fg(Color::Magenta),
                )));
                detail_lines.extend(app.cached_preview_lines.iter().cloned());
            }
        }
        Some(ExplorerItem::Model(id)) => {
            if let Some(model) = loaded.ir.models.get(id) {
                detail_lines = format_model_details(model);
            }
        }
        Some(ExplorerItem::Attachable(id)) => {
            if let Some(att) = loaded.ir.attachables.get(id) {
                detail_lines.push(Line::from(format!("Attachable ID:    {}", att.id)));
                detail_lines.push(Line::from(format!(
                    "Item Identifier:  {}",
                    att.item_identifier
                )));
                detail_lines.push(Line::from(format!(
                    "Default Texture:  {}",
                    att.default_texture.as_deref().unwrap_or("-")
                )));
                detail_lines.push(Line::from(format!(
                    "Default Geometry: {}",
                    att.default_geometry.as_deref().unwrap_or("-")
                )));
            }
        }
        Some(ExplorerItem::SoundFile(id)) => {
            if let Some(sf) = loaded.ir.sounds.files.get(id) {
                detail_lines.push(Line::from(format!("Sound File: {}", sf.id)));
                detail_lines.push(Line::from(format!(
                    "OGG Size:   {}",
                    format_bytes(sf.data.len() as u64)
                )));
            }
        }
        Some(ExplorerItem::SoundEvent(id)) => {
            if let Some(ev) = loaded.ir.sounds.events.get(id) {
                detail_lines.push(Line::from(format!("Sound Event: {id}")));
                detail_lines.push(Line::from(format!(
                    "Category:    {}",
                    ev.category.as_deref().unwrap_or("-")
                )));
                detail_lines.push(Line::from(format!("Variants:    {}", ev.sounds.len())));
                for s in &ev.sounds {
                    detail_lines.push(Line::from(format!(
                        "  • {} (vol={}, pitch={}, w={})",
                        s.name, s.volume, s.pitch, s.weight
                    )));
                }
            }
        }
        Some(ExplorerItem::Locale(code)) => {
            if let Some(table) = loaded.ir.locales.get(code) {
                detail_lines.push(Line::from(format!(
                    "Locale: {code} ({} entries)",
                    table.entries.len()
                )));
                detail_lines.push(Line::from(""));
                let mut pairs: Vec<(&String, &String)> = table.entries.iter().collect();
                pairs.sort_by(|a, b| a.0.cmp(b.0));
                for (k, v) in pairs.into_iter().take(40) {
                    detail_lines.push(Line::from(format!("  {k} = {v}")));
                }
            }
        }
        Some(ExplorerItem::Passthrough(path)) => {
            if let Some(bytes) = loaded.ir.passthrough_files.get(path) {
                detail_lines.push(Line::from(format!("Passthrough File: {path}")));
                detail_lines.push(Line::from(format!(
                    "Size: {}",
                    format_bytes(bytes.len() as u64)
                )));
                if let Ok(txt) = std::str::from_utf8(bytes) {
                    detail_lines.push(Line::from(""));
                    for line in txt.lines().take(35) {
                        detail_lines.push(Line::from(line.to_string()));
                    }
                }
            }
        }
        None => {
            detail_lines.push(Line::from("No matching assets for current filter."));
        }
    }

    let detail = Paragraph::new(detail_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Inspector — {} ", loaded.ir.metadata.name)),
        )
        .scroll((app.detail_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, cols[1]);
}

fn render_convert_tab(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(area);

    let f = &app.convert_form;
    let edition_label = match f.target_edition {
        None => "Auto (Flip Source)",
        Some(Edition::Java) => "Java Edition",
        Some(Edition::Bedrock) => "Bedrock Edition",
    };
    let version_label = f.selected_version().unwrap_or("26.2");
    let rot_label = match f.rotation_mode {
        RotationSnapMode::Preserve => "Preserve (Exact / 26.x {x,y,z})",
        RotationSnapMode::Snap22_5 => "Snap (Legacy 22.5°)",
        RotationSnapMode::Clamp45 => "Clamp ([-45..45])",
    };
    let geo_label = match f.bedrock_geo_version {
        None => "Auto (by target version)",
        Some(v) => v.as_str(),
    };
    let manifest_label = match f.bedrock_manifest_version {
        None => "Auto (v2)",
        Some(3) => "v3 (SemVer)",
        Some(2) => "v2",
        Some(1) => "v1",
        Some(_) => "Auto",
    };
    let items_label = match f.emit_java_items {
        None => "Auto (1.21.4+ / 26.x)",
        Some(true) => "Force Enabled",
        Some(false) => "Force Disabled",
    };
    let strict_label = if f.strict {
        "ON (fail on warnings/lossy)"
    } else {
        "OFF"
    };

    let form_rows: [(&str, String); 9] = [
        ("Input Pack [Enter]", f.input_path.clone()),
        ("Output Path [Enter]", f.output_path.clone()),
        ("Target Edition [←/→]", edition_label.to_string()),
        ("Target Version [←/→]", version_label.to_string()),
        ("Rotation Mode [←/→]", rot_label.to_string()),
        ("Bedrock Geo Ver [←/→]", geo_label.to_string()),
        ("Bedrock Manifest [←/→]", manifest_label.to_string()),
        ("Java items/*.json [←/→]", items_label.to_string()),
        ("Strict Mode [Space]", strict_label.to_string()),
    ];

    let items: Vec<ListItem<'_>> = form_rows
        .iter()
        .enumerate()
        .map(|(idx, (label, val))| {
            let focused = idx == f.selected_field;
            let style = if focused {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{label:<24}: "),
                    Style::default().fg(if focused { Color::Yellow } else { Color::Cyan }),
                ),
                Span::raw(val.clone()),
            ]))
            .style(style)
        })
        .collect();

    let form_list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Conversion Settings — Press 'x'/F5 to Run | 'e' Export JSON "),
    );
    frame.render_widget(form_list, cols[0]);

    let report_lines: Vec<Line<'static>> = match &app.last_conversion {
        None => vec![
            Line::from("No conversion executed in this session yet."),
            Line::from(""),
            Line::from("1. Select a pack in [1:Packs] and press 'c'"),
            Line::from("2. Adjust target edition / version (up to 26.3)"),
            Line::from("3. Press 'x' or F5 to run conversion"),
        ],
        Some(Err(err)) => vec![
            Line::from(Span::styled(
                "Conversion Failed:",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(err.clone()),
        ],
        Some(Ok(res)) => {
            let mut lines = vec![
                Line::from(Span::styled(
                    format!(
                        "SUCCESS: {} -> {} ({} ms)",
                        res.source, res.target, res.elapsed_ms
                    ),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(format!("Output: {}", res.output_path.display())),
                Line::from(""),
                Line::from(format!(
                    "  Converted assets:   {}",
                    res.report.converted_count
                )),
                Line::from(format!(
                    "  Pass-through:       {}",
                    res.report.passthrough_count
                )),
                Line::from(format!("  Lossy conversions:  {}", res.report.lossy_count)),
                Line::from(format!(
                    "  Skipped assets:     {}",
                    res.report.skipped_count
                )),
                Line::from(format!(
                    "  Warnings:           {}",
                    res.report.warnings.len()
                )),
            ];
            if !res.report.lossy_conversions.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Lossy Conversions:",
                    Style::default().fg(Color::Yellow),
                )));
                for item in &res.report.lossy_conversions {
                    lines.push(Line::from(format!(" • [{}] {}", item.asset, item.detail)));
                }
            }
            if !res.report.skipped_assets.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Skipped Assets:",
                    Style::default().fg(Color::Red),
                )));
                for item in &res.report.skipped_assets {
                    lines.push(Line::from(format!(" • [{}] {}", item.asset, item.reason)));
                }
            }
            if !res.report.warnings.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Warnings:",
                    Style::default().fg(Color::Cyan),
                )));
                for w in &res.report.warnings {
                    lines.push(Line::from(format!(
                        " • [{:?}] {}: {}",
                        w.severity, w.asset, w.message
                    )));
                }
            }
            lines
        }
    };

    let report_box = Paragraph::new(report_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Conversion Report (PgUp/PgDn to scroll) "),
        )
        .scroll((app.report_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(report_box, cols[1]);
}

fn render_batch_tab(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let rows_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6)])
        .split(area);

    let (ratio, label) = match &app.batch_progress {
        Some((done, total, msg)) if *total > 0 => (
            (*done as f64 / *total as f64).clamp(0.0, 1.0),
            format!("{done}/{total} — {msg}"),
        ),
        _ => (
            0.0,
            String::from(
                "Press 't' to Audit Round-Trip | 'b' to Batch Convert into ./converted/ | 'e' Export JSON",
            ),
        ),
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Batch & Round-Trip Benchmark Progress "),
        )
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(ratio)
        .label(label);
    frame.render_widget(gauge, rows_layout[0]);

    let header = Row::new(vec![
        Cell::from("Status"),
        Cell::from("Pack"),
        Cell::from("Direction"),
        Cell::from("Conv"),
        Cell::from("Lossy"),
        Cell::from("Time"),
        Cell::from("Details"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let table_rows: Vec<Row<'_>> = app
        .audit_outcomes
        .iter()
        .map(|o| {
            let status_cell = if o.passed {
                Cell::from("PASS").style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Cell::from("FAIL")
                    .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            };
            let (conv, lossy) = match &o.forward_report {
                Some(r) => (r.converted_count.to_string(), r.lossy_count.to_string()),
                None => (String::from("-"), String::from("-")),
            };
            Row::new(vec![
                status_cell,
                Cell::from(o.pack_name.clone()),
                Cell::from(format!("{} -> {}", o.source_edition, o.target_edition)),
                Cell::from(conv),
                Cell::from(lossy),
                Cell::from(format!("{}ms", o.elapsed_ms)),
                Cell::from(o.detail.clone()),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(6),
            Constraint::Length(22),
            Constraint::Length(17),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Results ({}) ", app.audit_outcomes.len())),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White));

    let mut state = TableState::default();
    if !app.audit_outcomes.is_empty() {
        state.select(Some(
            app.selected_audit_idx.min(app.audit_outcomes.len() - 1),
        ));
    }
    frame.render_stateful_widget(table, rows_layout[1], &mut state);
}

fn render_compare_tab(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let lines: Vec<Line<'static>> = match &app.pack_diff {
        None => vec![
            Line::from("No pack comparison run yet."),
            Line::from(""),
            Line::from(
                "1. Go to [1:Packs] and mark 2 packs with Space (e.g. KITSUNE.zip and kitsune-java.zip)",
            ),
            Line::from("2. Press '=' (or press 'r' right here) to run a canonical IR Pack Diff!"),
        ],
        Some(Err(err)) => vec![
            Line::from(Span::styled(
                "Pack Diff Error:",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(err.clone()),
        ],
        Some(Ok(d)) => {
            let mut out = vec![
                Line::from(vec![
                    Span::styled(
                        "Pack A: ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!("{} ({})", d.name_a, d.edition_a)),
                    Span::raw("   vs   "),
                    Span::styled(
                        "Pack B: ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!("{} ({})", d.name_b, d.edition_b)),
                ]),
                Line::from(""),
            ];

            let cats = [
                ("Textures", &d.textures),
                ("3D Models", &d.models),
                ("Attachables", &d.attachables),
                ("Sound Files", &d.sound_files),
                ("Locales", &d.locales),
            ];
            for (title, cat) in cats {
                out.push(Line::from(Span::styled(
                    format!(
                        "=== {title}: {} shared | -{} only in A | +{} only in B ===",
                        cat.shared_count,
                        cat.only_in_a.len(),
                        cat.only_in_b.len()
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                for id in cat.only_in_a.iter().take(15) {
                    out.push(Line::from(Span::styled(
                        format!("  - [A only] {id}"),
                        Style::default().fg(Color::Red),
                    )));
                }
                if cat.only_in_a.len() > 15 {
                    out.push(Line::from(format!(
                        "  ... (+{} more in A)",
                        cat.only_in_a.len() - 15
                    )));
                }
                for id in cat.only_in_b.iter().take(15) {
                    out.push(Line::from(Span::styled(
                        format!("  + [B only] {id}"),
                        Style::default().fg(Color::Green),
                    )));
                }
                if cat.only_in_b.len() > 15 {
                    out.push(Line::from(format!(
                        "  ... (+{} more in B)",
                        cat.only_in_b.len() - 15
                    )));
                }
                out.push(Line::from(""));
            }
            out
        }
    };

    let diff_view = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Canonical IR Pack Compare (↑/↓:Scroll | r:Run Diff) "),
        )
        .scroll((app.diff_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(diff_view, area);
}

fn render_versions_tab(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let header = Row::new(vec![
        Cell::from("Version"),
        Cell::from("Java fmt"),
        Cell::from("Bedrock Engine"),
        Cell::from("Geo Ver"),
        Cell::from("Features"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row<'_>> = app
        .mappings
        .pack_format_versions()
        .iter()
        .rev()
        .map(|entry| {
            let b = entry.bedrock_min_engine_version;
            let geo = BedrockGeoVersion::for_engine_version(b).as_str();
            let feat = describe_java_features(entry.java_pack_format);
            Row::new(vec![
                Cell::from(entry.minecraft_version.clone()),
                Cell::from(entry.java_pack_format.to_string()),
                Cell::from(format!("{}.{}.{}", b[0], b[1], b[2])),
                Cell::from(geo),
                Cell::from(feat),
            ])
        })
        .collect();

    let count = rows.len();
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(15),
            Constraint::Length(8),
            Constraint::Min(30),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Supported Minecraft Version Matrix (Newest First) "),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White));

    let mut state = TableState::default();
    if count > 0 {
        state.select(Some(app.version_scroll.min(count - 1)));
    }
    frame.render_stateful_widget(table, split[0], &mut state);

    let log_lines: Vec<Line<'_>> = app
        .logs
        .iter()
        .rev()
        .take(50)
        .map(|l| Line::from(l.clone()))
        .collect();
    let log_box = Paragraph::new(log_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Session Activity Log ({}) ", app.logs.len())),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(log_box, split[1]);
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let hints = match app.active_tab {
        ActiveTab::Packs => {
            "Enter:Explore | c:Convert | t:Audit | =:Compare | s:Sort | u:Unpack | p:Repack | Del:Delete | ?:Help | q:Quit"
        }
        ActiveTab::Explorer => {
            "↑/↓:Asset | a/d/w/s:Orbit 3D | h:HeadRef | f:Category | /:Search | c:Convert | ?:Help | q:Quit"
        }
        ActiveTab::Convert => {
            "↑/↓:Field | ←/→:Option | Enter:Edit | x/F5:Convert | e:Export JSON | ?:Help | q:Quit"
        }
        ActiveTab::BatchAudit => {
            "t:Audit Round-Trip | b:Batch Convert | e:Export JSON | ↑/↓:Result | ?:Help | q:Quit"
        }
        ActiveTab::Compare => "r:Compare Marked Packs | ↑/↓:Scroll Diff | ?:Help | q:Quit",
        ActiveTab::VersionsLogs => "↑/↓:Scroll Versions | 1-6/Tab:Switch Tab | ?:Help | q:Quit",
    };

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", app.status_banner),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(hints, Style::default().fg(Color::Gray)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, area);
}

fn render_help_overlay(frame: &mut Frame<'_>) {
    let area = centered_rect(76, 78, frame.area());
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(
            "rp-tui Keyboard Shortcuts (Linux Resource Pack Manager)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Global:"),
        Line::from("    1..6 / Tab / Shift+Tab   Switch tabs (mouse click & wheel supported)"),
        Line::from("    ? / F1                   Toggle help overlay"),
        Line::from("    q / Ctrl+C               Quit rp-tui"),
        Line::from(""),
        Line::from("  [1:Packs] Manager:"),
        Line::from("    Space / a / s            Mark / mark all / cycle sort order"),
        Line::from("    Enter / i / c / t        Explore / send to Convert / Round-Trip Audit"),
        Line::from("    = / u / p / Del          Compare 2 packs / Unpack / Repack / Delete"),
        Line::from("    d / r / /                Change directory / rescan / filter"),
        Line::from(""),
        Line::from("  [2:Explorer] Asset Viewer & 3D Braille Wireframe:"),
        Line::from("    a/d or [/] , w/s , h     Orbit 3D model yaw/pitch & toggle head box"),
        Line::from("    f / →  ,  / / Esc        Cycle category filter / search"),
        Line::from("    PgUp/PgDn or J/K         Scroll diagnostics"),
        Line::from(""),
        Line::from("  [3:Convert], [4:Batch], [5:Compare]:"),
        Line::from("    x / F5 / e               Run Convert / Export JSON report"),
        Line::from("    b / t / r                Batch Convert / Audit All / Refresh Diff"),
        Line::from(""),
        Line::from(Span::styled(
            "Press Esc or ? to close.",
            Style::default().fg(Color::Yellow),
        )),
    ];
    let help = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Help "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(help, area);
}

fn render_input_overlay(frame: &mut Frame<'_>, mode: &InputMode) {
    if let InputMode::ConfirmDelete { path, is_dir } = mode {
        let area = centered_rect(68, 22, frame.area());
        frame.render_widget(Clear, area);
        let kind = if *is_dir { "directory" } else { "file" };
        let prompt = Paragraph::new(vec![
            Line::from(Span::styled(
                format!("Permanently delete this {kind}?"),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(path.display().to_string()),
            Line::from(""),
            Line::from(Span::styled(
                "Press 'y' to confirm, or any other key to cancel.",
                Style::default().fg(Color::Yellow),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" Confirm Delete "),
        )
        .wrap(Wrap { trim: false });
        frame.render_widget(prompt, area);
        return;
    }

    let (title, buf) = match mode {
        InputMode::EditScanDir(s) => (" Change Scan Directory (Enter to apply, Esc to cancel) ", s),
        InputMode::FilterPacks(s) => (" Filter Resource Packs (Enter to apply, Esc to cancel) ", s),
        InputMode::FilterAssets(s) => (
            " Search Explorer Assets (Enter to apply, Esc to cancel) ",
            s,
        ),
        InputMode::EditConvertInput(s) => {
            (" Edit Input Pack Path (Enter to apply, Esc to cancel) ", s)
        }
        InputMode::EditConvertOutput(s) => {
            (" Edit Output Pack Path (Enter to apply, Esc to cancel) ", s)
        }
        InputMode::Normal | InputMode::ConfirmDelete { .. } => return,
    };
    let area = centered_rect(70, 18, frame.area());
    frame.render_widget(Clear, area);
    let prompt = Paragraph::new(Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Yellow)),
        Span::raw(buf.clone()),
        Span::styled("█", Style::default().fg(Color::Cyan)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(title),
    );
    frame.render_widget(prompt, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}

fn describe_java_features(pack_format: u32) -> &'static str {
    if pack_format <= 3 {
        "Pre-Flattening (blocks/, items/, .lang)"
    } else if pack_format < 18 {
        "Flattened (block/, item/, .json)"
    } else if pack_format < 46 {
        "supported_formats"
    } else if pack_format < 63 {
        "items/*.json (1.21.4+)"
    } else if pack_format < 69 {
        "items/*.json + arb angle in [-45,45]"
    } else if pack_format < 75 {
        "min/max_format + items/*.json + [-45,45]"
    } else if pack_format < 97 {
        "min/max_format + multi-axis Euler {x,y,z}"
    } else {
        "97.1 + multi-axis Euler + shade_direction_override"
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{ImageFormat, Rgba, RgbaImage};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tempfile::tempdir;

    use rp_core::{MappingTable, TextureAsset, TextureCategory, TexturePayload};
    use rp_io::{IoLimits, PackContainer};

    use super::*;
    use crate::preview::render_texture_halfblock;
    use crate::scanner::{
        audit_pack_roundtrip, diff_packs, load_pack_ir, repack_dir_to_archive,
        scan_directory_for_packs, unpack_pack_to_dir,
    };

    fn make_png(w: u32, h: u32, color: Rgba<u8>) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, color);
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .expect("png");
        out
    }

    #[test]
    fn renders_all_six_tabs_3d_canvas_diff_and_unpack_repack() {
        let dir = tempdir().expect("tempdir");
        let mut container = PackContainer::new();
        container
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"description":"TUI Test Pack","min_format":[97,0],"max_format":[97,1]}}"#
                    .to_vec(),
            )
            .expect("mcmeta");
        container
            .insert("pack.png", make_png(16, 16, Rgba([40, 200, 120, 255])))
            .expect("icon");
        container
            .insert(
                "assets/minecraft/textures/block/stone.png",
                make_png(16, 16, Rgba([120, 180, 220, 255])),
            )
            .expect("tex");
        container
            .insert(
                "assets/minecraft/models/block/cube_3d.json",
                br##"{"textures":{"all":"block/stone"},"elements":[{"from":[4,4,4],"to":[12,12,12],"rotation":{"x":15,"y":45,"z":0,"origin":[8,8,8]},"faces":{"north":{"uv":[0,0,16,16],"texture":"#all"}}}]}"##
                    .to_vec(),
            )
            .expect("model");
        let zip_path = dir.path().join("sample_java.zip");
        std::fs::write(&zip_path, container.write_to_zip_bytes().expect("zip")).expect("save");

        // Test unpack + repack round-trip
        let unpacked = unpack_pack_to_dir(&zip_path, IoLimits::default()).expect("unpack");
        assert!(unpacked.join("pack.mcmeta").is_file());
        let repacked = repack_dir_to_archive(&unpacked, IoLimits::default()).expect("repack");
        assert!(repacked.is_file());

        let mappings = MappingTable::default_embedded().expect("mappings");
        let packs =
            scan_directory_for_packs(dir.path(), &mappings, IoLimits::default()).expect("scan");
        assert!(packs.len() >= 2);

        let diff = diff_packs(&zip_path, &repacked, &mappings, IoLimits::default()).expect("diff");
        assert_eq!(diff.textures.shared_count, 1);
        assert_eq!(diff.models.shared_count, 1);

        let audit = audit_pack_roundtrip(
            &zip_path,
            Some("26.3"),
            &rp_cli::VersionSpecificOptions::default(),
            &mappings,
            IoLimits::default(),
        );
        assert!(audit.passed, "audit failed: {}", audit.detail);

        let tex = TextureAsset {
            id: String::from("block/stone"),
            category: TextureCategory::Block,
            atlas_key: None,
            width: Some(16),
            height: Some(16),
            resolution_tier: Some(rp_core::ResolutionTier::R16),
            payload: TexturePayload::from_raw(make_png(16, 16, Rgba([255, 80, 40, 255]))),
            animation: None,
        };
        assert!(!render_texture_halfblock(&tex, 32, 16).is_empty());

        let mut app = App::new(dir.path().to_path_buf(), None, mappings.clone());
        app.packs = packs;
        app.refresh_pack_icon_cache();
        let (ed, ir, rep) = load_pack_ir(&zip_path, &mappings, IoLimits::default()).expect("ir");
        app.loaded_pack = Some(crate::app::LoadedPack {
            path: zip_path,
            edition: ed,
            ir,
            report: rep,
        });
        app.explorer_category = crate::app::AssetCategoryFilter::Models;
        app.rebuild_explorer_items();
        app.audit_outcomes.push(audit);
        app.pack_diff = Some(Ok(diff));

        let backend = TestBackend::new(120, 38);
        let mut terminal = Terminal::new(backend).expect("terminal");

        for tab in ActiveTab::ALL {
            app.active_tab = tab;
            terminal
                .draw(|f| render(f, &app))
                .expect("draw tab without panic");
        }
        app.show_help = true;
        terminal
            .draw(|f| render(f, &app))
            .expect("draw help overlay");
    }
}
