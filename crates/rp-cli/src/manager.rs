use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use rp_bedrock::parse_bedrock_pack;
use rp_core::{
    ConversionReport, Edition, FaceDirection, MappingTable, ModelAsset, ModelElement,
    ResourcePackIR, RotationAxis,
};
use rp_io::{IoLimits, PackContainer};
use rp_java::parse_java_pack;
use rp_transforms::model::euler_zyx_to_mat3;

use crate::{
    InspectSummary, VersionSpecificOptions, convert_container_with_options, detect_edition,
    inspect_container,
};

/// Resource pack discovered inside a scanned directory.
#[derive(Debug, Clone)]
pub struct DiscoveredPack {
    /// Path to `.zip`, `.mcpack`, or unpacked resource pack folder.
    pub path: PathBuf,
    /// Filename or directory name.
    pub file_name: String,
    /// Size on disk in bytes.
    pub size_bytes: u64,
    /// Whether this pack is an unpacked directory.
    pub is_dir: bool,
    /// Optional raw `pack.png` / `pack_icon.png` bytes for thumbnail preview.
    pub icon_png: Option<Vec<u8>>,
    /// Parsed `InspectSummary`, or parse error message.
    pub summary: Result<InspectSummary, String>,
    /// Selection flag for batch operations / diff.
    pub selected: bool,
}

/// Sort criterion for discovered packs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackSortKey {
    Name,
    Edition,
    Size,
    Textures,
    Models,
}

impl PackSortKey {
    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Edition => "Edition",
            Self::Size => "Size",
            Self::Textures => "Textures",
            Self::Models => "3D Models",
        }
    }

    /// Next sort criterion cyclically.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Name => Self::Edition,
            Self::Edition => Self::Size,
            Self::Size => Self::Textures,
            Self::Textures => Self::Models,
            Self::Models => Self::Name,
        }
    }
}

/// Sorts a slice of `DiscoveredPack` in-place by `key`.
pub fn sort_discovered_packs(packs: &mut [DiscoveredPack], key: PackSortKey) {
    packs.sort_by(|a, b| match key {
        PackSortKey::Name => a
            .file_name
            .to_ascii_lowercase()
            .cmp(&b.file_name.to_ascii_lowercase()),
        PackSortKey::Edition => {
            let ea = a.summary.as_ref().map(|s| s.edition.to_string()).ok();
            let eb = b.summary.as_ref().map(|s| s.edition.to_string()).ok();
            ea.cmp(&eb).then_with(|| a.file_name.cmp(&b.file_name))
        }
        PackSortKey::Size => b.size_bytes.cmp(&a.size_bytes),
        PackSortKey::Textures => {
            let ta = a.summary.as_ref().map_or(0, |s| s.texture_count);
            let tb = b.summary.as_ref().map_or(0, |s| s.texture_count);
            tb.cmp(&ta).then_with(|| a.file_name.cmp(&b.file_name))
        }
        PackSortKey::Models => {
            let ma = a.summary.as_ref().map_or(0, |s| s.model_count);
            let mb = b.summary.as_ref().map_or(0, |s| s.model_count);
            mb.cmp(&ma).then_with(|| a.file_name.cmp(&b.file_name))
        }
    });
}

/// Result of a bidirectional round-trip & integrity audit or batch conversion item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditOutcome {
    /// Pack filename.
    pub pack_name: String,
    /// Source pack path.
    pub path: PathBuf,
    /// Source edition detected.
    pub source_edition: Edition,
    /// Target edition converted to.
    pub target_edition: Edition,
    /// Whether conversion + integrity + round-trip succeeded.
    pub passed: bool,
    /// Forward conversion report.
    pub forward_report: Option<ConversionReport>,
    /// Human-readable audit or output detail.
    pub detail: String,
    /// Elapsed wall-clock milliseconds.
    pub elapsed_ms: u128,
}

/// Per-category set difference between two resource packs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryDiff {
    /// IDs present only in Pack A.
    pub only_in_a: Vec<String>,
    /// IDs present only in Pack B.
    pub only_in_b: Vec<String>,
    /// Count of IDs present in both packs.
    pub shared_count: usize,
}

/// Canonical IR comparison between Pack A and Pack B.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackDiff {
    /// Label/filename of Pack A.
    pub name_a: String,
    /// Edition of Pack A.
    pub edition_a: Edition,
    /// Label/filename of Pack B.
    pub name_b: String,
    /// Edition of Pack B.
    pub edition_b: Edition,
    /// Texture ID diff.
    pub textures: CategoryDiff,
    /// 3D model ID diff.
    pub models: CategoryDiff,
    /// Attachable ID diff.
    pub attachables: CategoryDiff,
    /// Sound file ID diff.
    pub sound_files: CategoryDiff,
    /// Locale code diff.
    pub locales: CategoryDiff,
}

/// Scans `dir` for `.zip`, `.mcpack`, and unpacked resource pack subdirectories.
pub fn scan_directory_for_packs(
    dir: &Path,
    mappings: &MappingTable,
    limits: IoLimits,
) -> Result<Vec<DiscoveredPack>> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read directory '{}'", dir.display()))?;

    let mut candidates: Vec<(PathBuf, bool, u64)> = Vec::new();
    for entry_res in entries {
        let Ok(entry) = entry_res else {
            continue;
        };
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };

        if meta.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if matches!(ext.as_deref(), Some("zip" | "mcpack")) {
                candidates.push((path, false, meta.len()));
            }
        } else if meta.is_dir() && is_resource_pack_dir(&path) {
            let dir_size = compute_dir_size(&path);
            candidates.push((path, true, dir_size));
        }
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    let mut packs = Vec::with_capacity(candidates.len());
    for (path, is_dir, size_bytes) in candidates {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let (summary, icon_png) = match PackContainer::read_from_path(&path, limits) {
            Ok(container) => {
                let icon = container
                    .get("pack.png")
                    .or_else(|| container.get("pack_icon.png"))
                    .map(|b| b.to_vec());
                let sum = match inspect_container(&container, mappings) {
                    Ok(s) => Ok(s),
                    Err(err) => Err(format!("{err:#}")),
                };
                (sum, icon)
            }
            Err(err) => (Err(format!("{err}")), None),
        };

        if let Err(ref msg) = summary
            && !is_dir
            && msg.contains("Unable to detect resource pack edition")
        {
            continue;
        }

        packs.push(DiscoveredPack {
            path,
            file_name,
            size_bytes,
            is_dir,
            icon_png,
            summary,
            selected: false,
        });
    }

    Ok(packs)
}

/// Loads full canonical `ResourcePackIR` and parse diagnostics for a pack at `path`.
pub fn load_pack_ir(
    path: &Path,
    mappings: &MappingTable,
    limits: IoLimits,
) -> Result<(Edition, ResourcePackIR, ConversionReport)> {
    let container = PackContainer::read_from_path(path, limits)
        .with_context(|| format!("failed to open pack '{}'", path.display()))?;
    let edition = detect_edition(&container)?;
    let (ir, report) = match edition {
        Edition::Java => parse_java_pack(&container, mappings)?,
        Edition::Bedrock => parse_bedrock_pack(&container, mappings)?,
    };
    Ok((edition, ir, report))
}

/// Compares two resource packs at `path_a` and `path_b` in canonical IR space.
pub fn diff_packs(
    path_a: &Path,
    path_b: &Path,
    mappings: &MappingTable,
    limits: IoLimits,
) -> Result<PackDiff> {
    let (edition_a, ir_a, _) = load_pack_ir(path_a, mappings, limits)?;
    let (edition_b, ir_b, _) = load_pack_ir(path_b, mappings, limits)?;

    let name_a = path_a
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Pack A")
        .to_string();
    let name_b = path_b
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Pack B")
        .to_string();

    Ok(PackDiff {
        name_a,
        edition_a,
        name_b,
        edition_b,
        textures: diff_key_sets(ir_a.textures.keys(), ir_b.textures.keys()),
        models: diff_key_sets(ir_a.models.keys(), ir_b.models.keys()),
        attachables: diff_key_sets(ir_a.attachables.keys(), ir_b.attachables.keys()),
        sound_files: diff_key_sets(ir_a.sounds.files.keys(), ir_b.sounds.files.keys()),
        locales: diff_key_sets(ir_a.locales.keys(), ir_b.locales.keys()),
    })
}

fn diff_key_sets<'a>(
    a: impl Iterator<Item = &'a String>,
    b: impl Iterator<Item = &'a String>,
) -> CategoryDiff {
    let set_a: BTreeSet<&str> = a.map(String::as_str).collect();
    let set_b: BTreeSet<&str> = b.map(String::as_str).collect();
    let only_in_a = set_a.difference(&set_b).map(|s| (*s).to_string()).collect();
    let only_in_b = set_b.difference(&set_a).map(|s| (*s).to_string()).collect();
    let shared_count = set_a.intersection(&set_b).count();
    CategoryDiff {
        only_in_a,
        only_in_b,
        shared_count,
    }
}

/// Unpacks a `.zip` or `.mcpack` resource pack into a sibling directory (`<stem>_unpacked/`).
pub fn unpack_pack_to_dir(input: &Path, limits: IoLimits) -> Result<PathBuf> {
    if input.is_dir() {
        bail!("'{}' is already an unpacked directory", input.display());
    }
    let container = PackContainer::read_from_path(input, limits)
        .with_context(|| format!("failed to read archive '{}'", input.display()))?;
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("pack");
    let out_dir = parent.join(format!("{stem}_unpacked"));
    container
        .write_to_directory(&out_dir)
        .with_context(|| format!("failed to unpack into '{}'", out_dir.display()))?;
    Ok(out_dir)
}

/// Repacks an unpacked resource pack directory into a `.zip` (Java) or `.mcpack` (Bedrock) archive.
pub fn repack_dir_to_archive(dir: &Path, limits: IoLimits) -> Result<PathBuf> {
    if !dir.is_dir() {
        bail!("'{}' is not a directory", dir.display());
    }
    let container = PackContainer::read_from_path(dir, limits)
        .with_context(|| format!("failed to read directory '{}'", dir.display()))?;
    let edition = detect_edition(&container)?;
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    let stem = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repacked")
        .trim_end_matches("_unpacked");
    let out_path = match edition {
        Edition::Java => parent.join(format!("{stem}_repacked.zip")),
        Edition::Bedrock => parent.join(format!("{stem}_repacked.mcpack")),
    };
    let zip_bytes = container
        .write_to_zip_bytes()
        .context("failed to encode ZIP archive")?;
    fs::write(&out_path, zip_bytes)
        .with_context(|| format!("failed to write '{}'", out_path.display()))?;
    Ok(out_path)
}

/// Writes a `ConversionReport` JSON file to `path`.
pub fn export_report_json(report: &ConversionReport, path: &Path) -> Result<()> {
    let json = serde_json::to_vec_pretty(report)?;
    fs::write(path, json).with_context(|| format!("failed to write '{}'", path.display()))?;
    Ok(())
}

/// Writes a slice of `AuditOutcome` records as pretty JSON to `path`.
pub fn export_audit_json(outcomes: &[AuditOutcome], path: &Path) -> Result<()> {
    let json = serde_json::to_vec_pretty(outcomes)?;
    fs::write(path, json).with_context(|| format!("failed to write '{}'", path.display()))?;
    Ok(())
}

/// Runs a full bidirectional conversion (`Source -> Opposite -> Source`) and validates
/// emitted JSON syntax and Java `[-16, 32]` element bounds.
pub fn audit_pack_roundtrip(
    path: &Path,
    target_version: Option<&str>,
    version_options: &VersionSpecificOptions,
    mappings: &MappingTable,
    limits: IoLimits,
) -> AuditOutcome {
    let start = Instant::now();
    let pack_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let container = match PackContainer::read_from_path(path, limits) {
        Ok(c) => c,
        Err(err) => {
            return AuditOutcome {
                pack_name,
                path: path.to_path_buf(),
                source_edition: Edition::Java,
                target_edition: Edition::Bedrock,
                passed: false,
                forward_report: None,
                detail: format!("read error: {err}"),
                elapsed_ms: start.elapsed().as_millis(),
            };
        }
    };

    let source = match detect_edition(&container) {
        Ok(ed) => ed,
        Err(err) => {
            return AuditOutcome {
                pack_name,
                path: path.to_path_buf(),
                source_edition: Edition::Java,
                target_edition: Edition::Bedrock,
                passed: false,
                forward_report: None,
                detail: format!("edition error: {err}"),
                elapsed_ms: start.elapsed().as_millis(),
            };
        }
    };
    let target = match source {
        Edition::Java => Edition::Bedrock,
        Edition::Bedrock => Edition::Java,
    };

    let (converted, _, _, forward_report) = match convert_container_with_options(
        &container,
        Some(target),
        target_version,
        mappings,
        false,
        version_options,
    ) {
        Ok(res) => res,
        Err(err) => {
            return AuditOutcome {
                pack_name,
                path: path.to_path_buf(),
                source_edition: source,
                target_edition: target,
                passed: false,
                forward_report: None,
                detail: format!("forward convert error: {err:#}"),
                elapsed_ms: start.elapsed().as_millis(),
            };
        }
    };

    let mut issues: Vec<String> = Vec::new();
    for (rel, bytes) in converted.files() {
        if rel.ends_with(".json") || rel.ends_with(".mcmeta") {
            match serde_json::from_slice::<serde_json::Value>(bytes) {
                Ok(val) => {
                    if target == Edition::Java
                        && rel.starts_with("assets/minecraft/models/")
                        && let Some(elements) = val.get("elements").and_then(|e| e.as_array())
                    {
                        for (idx, el) in elements.iter().enumerate() {
                            for key in ["from", "to"] {
                                if let Some(arr) = el.get(key).and_then(|v| v.as_array()) {
                                    for c in arr.iter().filter_map(|v| v.as_f64()) {
                                        if !(-16.001..=32.001).contains(&c) {
                                            issues.push(format!(
                                                "{rel}[{idx}].{key} out of [-16,32]: {c:.2}"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(err) => issues.push(format!("invalid JSON '{rel}': {err}")),
            }
        }
    }

    let roundtrip_res = convert_container_with_options(
        &converted,
        Some(source),
        None,
        mappings,
        false,
        &VersionSpecificOptions::default(),
    );

    match roundtrip_res {
        Ok((rt_container, _, _, _)) if issues.is_empty() => AuditOutcome {
            pack_name,
            path: path.to_path_buf(),
            source_edition: source,
            target_edition: target,
            passed: true,
            forward_report: Some(forward_report),
            detail: format!(
                "OK: {} out files -> {} round-trip files, 0 JSON/bounds issues",
                converted.len(),
                rt_container.len()
            ),
            elapsed_ms: start.elapsed().as_millis(),
        },
        Ok(_) => AuditOutcome {
            pack_name,
            path: path.to_path_buf(),
            source_edition: source,
            target_edition: target,
            passed: false,
            forward_report: Some(forward_report),
            detail: format!("{} audit issue(s): {}", issues.len(), issues.join("; ")),
            elapsed_ms: start.elapsed().as_millis(),
        },
        Err(err) => AuditOutcome {
            pack_name,
            path: path.to_path_buf(),
            source_edition: source,
            target_edition: target,
            passed: false,
            forward_report: Some(forward_report),
            detail: format!("round-trip re-parse failed: {err:#}"),
            elapsed_ms: start.elapsed().as_millis(),
        },
    }
}

/// Projected 2D quad for painter's-algorithm 3D model rendering (shared by TUI Canvas & GUI Viewport).
#[derive(Debug, Clone)]
pub struct ProjectedQuad3D {
    /// 4 projected 2D vertices in normalized model space `[-1..1]` (Y-up).
    pub pts: [[f32; 2]; 4],
    /// Average camera-space Z depth for back-to-front sorting.
    pub depth: f32,
    /// Directional diffuse shading brightness `[0.35..1.0]`.
    pub shade: f32,
    /// Face direction (used for subtle color tinting).
    pub dir: FaceDirection,
}

/// Projected 3D scene ready for rendering in either TUI Braille Canvas or GUI Painter.
#[derive(Debug, Clone, Default)]
pub struct ProjectedScene3D {
    /// Depth-sorted front-facing quads (back-to-front).
    pub quads: Vec<ProjectedQuad3D>,
    /// Projected wireframe edges `([x0, y0], [x1, y1])` in normalized `[-1..1]` space.
    pub edges: Vec<([f32; 2], [f32; 2])>,
    /// Optional projected player-head `[4..12]^3` reference wireframe edges.
    pub head_ref_edges: Vec<([f32; 2], [f32; 2])>,
}

/// Projects all rotated cuboids of a `ModelAsset` into normalized `[-1..1]` 2D camera space
/// at `(yaw_deg, pitch_deg)` using exact `ZYX` element rotations.
#[must_use]
pub fn project_model_3d(
    model: &ModelAsset,
    yaw_deg: f32,
    pitch_deg: f32,
    include_head_ref: bool,
) -> ProjectedScene3D {
    if model.elements.is_empty() {
        return ProjectedScene3D::default();
    }

    // 1. Compute world-space corners for every rotated cuboid.
    let mut element_corners: Vec<[[f64; 3]; 8]> = Vec::with_capacity(model.elements.len());
    let mut min_c = [f64::INFINITY; 3];
    let mut max_c = [f64::NEG_INFINITY; 3];

    for el in &model.elements {
        let corners = rotated_cube_corners(el);
        for c in &corners {
            for i in 0..3 {
                min_c[i] = min_c[i].min(c[i]);
                max_c[i] = max_c[i].max(c[i]);
            }
        }
        element_corners.push(corners);
    }

    if include_head_ref {
        for i in 0..3 {
            min_c[i] = min_c[i].min(4.0);
            max_c[i] = max_c[i].max(12.0);
        }
    }

    let center = [
        0.5 * (min_c[0] + max_c[0]),
        0.5 * (min_c[1] + max_c[1]),
        0.5 * (min_c[2] + max_c[2]),
    ];
    let radius = ((max_c[0] - min_c[0])
        .max(max_c[1] - min_c[1])
        .max(max_c[2] - min_c[2])
        * 0.65)
        .max(1.0);

    // Camera rotation: yaw around Y, then pitch around X.
    let (sy, cy) = f64::from(yaw_deg).to_radians().sin_cos();
    let (sp, cp) = f64::from(pitch_deg).to_radians().sin_cos();

    let project_pt = |p: [f64; 3]| -> [f64; 3] {
        let x = (p[0] - center[0]) / radius;
        let y = (p[1] - center[1]) / radius;
        let z = (p[2] - center[2]) / radius;
        // Yaw around Y
        let x1 = cy * x + sy * z;
        let z1 = -sy * x + cy * z;
        // Pitch around X
        let y2 = cp * y - sp * z1;
        let z2 = sp * y + cp * z1;
        [x1, y2, z2]
    };

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

    // 6 faces indexed into 8 corners:
    // 0:(fx,fy,fz) 1:(tx,fy,fz) 2:(tx,ty,fz) 3:(fx,ty,fz)
    // 4:(fx,fy,tz) 5:(tx,fy,tz) 6:(tx,ty,tz) 7:(fx,ty,tz)
    const FACE_INDICES: [(FaceDirection, [usize; 4]); 6] = [
        (FaceDirection::North, [0, 3, 2, 1]),
        (FaceDirection::South, [4, 5, 6, 7]),
        (FaceDirection::East, [1, 2, 6, 5]),
        (FaceDirection::West, [0, 4, 7, 3]),
        (FaceDirection::Up, [3, 7, 6, 2]),
        (FaceDirection::Down, [0, 1, 5, 4]),
    ];

    let light_dir = [0.38_f64, 0.78, 0.49];

    let mut quads = Vec::new();
    let mut edges = Vec::new();

    for corners in &element_corners {
        let mut cam = [[0.0_f64; 3]; 8];
        for i in 0..8 {
            cam[i] = project_pt(corners[i]);
        }

        for &(a, b) in &CUBE_EDGES {
            edges.push((
                [cam[a][0] as f32, cam[a][1] as f32],
                [cam[b][0] as f32, cam[b][1] as f32],
            ));
        }

        for &(dir, idx) in &FACE_INDICES {
            let p0 = cam[idx[0]];
            let p1 = cam[idx[1]];
            let p2 = cam[idx[2]];
            let p3 = cam[idx[3]];

            let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
            let nx = e1[1] * e2[2] - e1[2] * e2[1];
            let ny = e1[2] * e2[0] - e1[0] * e2[2];
            let nz = e1[0] * e2[1] - e1[1] * e2[0];
            let nlen = (nx * nx + ny * ny + nz * nz).sqrt();
            if nlen < 1e-8 {
                continue;
            }
            // Back-face cull in camera space (camera looks toward -Z, front faces have nz > 0)
            let (nx, ny, nz) = (nx / nlen, ny / nlen, nz / nlen);
            if nz < -0.05 {
                continue;
            }
            let dot = (nx * light_dir[0] + ny * light_dir[1] + nz * light_dir[2]).abs();
            let shade = (0.38 + 0.62 * dot).clamp(0.35, 1.0) as f32;
            let depth = ((p0[2] + p1[2] + p2[2] + p3[2]) * 0.25) as f32;

            quads.push(ProjectedQuad3D {
                pts: [
                    [p0[0] as f32, p0[1] as f32],
                    [p1[0] as f32, p1[1] as f32],
                    [p2[0] as f32, p2[1] as f32],
                    [p3[0] as f32, p3[1] as f32],
                ],
                depth,
                shade,
                dir,
            });
        }
    }

    // Painter's algorithm: farthest depth first.
    quads.sort_by(|a, b| a.depth.total_cmp(&b.depth));

    let mut head_ref_edges = Vec::new();
    if include_head_ref {
        let h_corners = [
            [4.0, 4.0, 4.0],
            [12.0, 4.0, 4.0],
            [12.0, 12.0, 4.0],
            [4.0, 12.0, 4.0],
            [4.0, 4.0, 12.0],
            [12.0, 4.0, 12.0],
            [12.0, 12.0, 12.0],
            [4.0, 12.0, 12.0],
        ];
        let mut cam = [[0.0_f64; 3]; 8];
        for i in 0..8 {
            cam[i] = project_pt(h_corners[i]);
        }
        for &(a, b) in &CUBE_EDGES {
            head_ref_edges.push((
                [cam[a][0] as f32, cam[a][1] as f32],
                [cam[b][0] as f32, cam[b][1] as f32],
            ));
        }
    }

    ProjectedScene3D {
        quads,
        edges,
        head_ref_edges,
    }
}

fn rotated_cube_corners(el: &ModelElement) -> [[f64; 3]; 8] {
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

fn is_resource_pack_dir(dir: &Path) -> bool {
    dir.join("pack.mcmeta").is_file() || dir.join("manifest.json").is_file()
}

fn compute_dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let Ok(rd) = fs::read_dir(&cur) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if let Ok(m) = entry.metadata() {
                if m.is_file() {
                    total = total.saturating_add(m.len());
                } else if m.is_dir() {
                    stack.push(p);
                }
            }
        }
    }
    total
}

/// Formats a byte count in human-readable units (`B`, `KB`, `MB`).
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Suggests a clean output archive path next to `input`.
#[must_use]
pub fn suggest_output_path(input: &Path, target: Edition) -> String {
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("converted");
    let out = match target {
        Edition::Java => parent.join(format!("{stem}-java.zip")),
        Edition::Bedrock => parent.join(format!("{stem}-bedrock.mcpack")),
    };
    out.display().to_string()
}
