#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! PackStudio (`pack-studio`) — VS Code-inspired desktop Resource Pack Artist Studio
//! for Minecraft Java & Bedrock Edition, built directly on `rp-core`, `rp-io`,
//! `rp-transforms`, `rp-java`, and `rp-bedrock`.

pub mod app;
pub mod artist;
pub mod blockbench;
pub mod command_palette;
pub mod explorer;
pub mod packager;
pub mod pipeline;
pub mod state;
pub mod theme;
pub mod watcher;
pub mod wizard;
pub mod workspace;

pub use app::PackStudioApp;
pub use state::WorkbenchState;

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::time::Duration;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use glam::Vec3;
    use image::{Rgba, RgbaImage};
    use rp_bedrock::schema::BedrockManifest;
    use rp_core::{AuditStatus, Edition, WarningSeverity, parse_minecraft_json};
    use rp_io::PackContainer;
    use rp_java::schema::JavaPackMcmeta;
    use tempfile::TempDir;
    use zip::ZipArchive;

    use crate::artist::{
        BiomeTintPreset, TextureFormatKind, apply_biome_tint, build_alpha_mask_preview,
        composite_grass_side_tga, decode_texture_bytes, encode_texture_bytes,
        resize_nearest_neighbor,
    };
    use crate::blockbench::bundle::ensure_extracted_from_bytes;
    use crate::blockbench::server::{
        BlockbenchServer, BridgeSavePayload, BridgeTexturePayload, apply_bridge_save_payload,
        validate_bridge_relative_path,
    };
    use crate::command_palette::{StudioCommand, filter_commands, fuzzy_score};
    use crate::explorer::{
        generate_minecraft_resource_id, sanitize_minecraft_filename,
    };
    use crate::packager::{
        CompressionPreset, ExportPackageConfig, PackageFormat, export_pack_archive,
    };
    use crate::pipeline::{
        BackgroundEvent, ConversionConfig, ConversionDirection, spawn_convert, spawn_preflight,
        spawn_semantic_audit,
    };
    use crate::state::{ActivityTab, PackMetadataForm, WorkbenchState};
    use crate::wizard::{NewProjectWizardState, ProjectTemplateKind, scaffold_new_project};
    use crate::workspace::{LoadedPack, PackSource, create_new_bedrock_pack};

    fn make_solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba(rgba));
        encode_texture_bytes(&img, TextureFormatKind::Png).expect("encode png")
    }

    fn build_tar_zst_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            for (path, data) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, *path, *data)
                    .expect("append tar entry");
            }
            builder.finish().expect("finish tar");
        }
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).expect("zstd encoder");
        encoder.write_all(&tar_buf).expect("write tar to zstd");
        encoder.finish().expect("finish zstd")
    }

    #[test]
    fn workbench_state_tabs_activity_bar_and_command_palette_filter() {
        let mut wb = WorkbenchState::new();
        assert_eq!(wb.open_tab_count(), 1);
        assert_eq!(wb.active_activity, ActivityTab::Explorer);
        assert!(wb.sidebar_visible);
        assert!(wb.bottom_panel_visible);

        let pack = create_new_bedrock_pack("Artist Studio Pack").expect("starter bedrock pack");
        wb.set_loaded_pack(pack);

        wb.open_asset_tab("manifest.json");
        assert_eq!(wb.open_tab_count(), 2);
        wb.open_asset_tab("manifest.json");
        assert_eq!(wb.open_tab_count(), 2);

        wb.open_asset_tab("textures/blocks/stone.png");
        assert_eq!(wb.open_tab_count(), 3);

        wb.open_conversion_dashboard();
        assert_eq!(wb.open_tab_count(), 4);

        assert!(wb.close_tab_by_key("code:manifest.json"));
        assert_eq!(wb.open_tab_count(), 3);
        assert!(!wb.close_tab_by_key("code:manifest.json"));

        wb.select_activity(ActivityTab::Search);
        assert_eq!(wb.active_activity, ActivityTab::Search);
        assert!(wb.sidebar_visible);

        wb.select_activity(ActivityTab::Search);
        assert!(!wb.sidebar_visible);

        wb.select_activity(ActivityTab::Converter);
        assert_eq!(wb.active_activity, ActivityTab::Converter);
        assert!(wb.sidebar_visible);

        wb.toggle_sidebar();
        assert!(!wb.sidebar_visible);
        wb.toggle_bottom_panel();
        assert!(!wb.bottom_panel_visible);

        assert!(fuzzy_score("preflight", "Convert: Java -> Bedrock (Run Preflight)").is_some());
        assert!(fuzzy_score("xyz_no_match", "View: Toggle Sidebar").is_none());

        let bb_hits = filter_commands("open in blockbench");
        assert!(!bb_hits.is_empty());
        assert_eq!(bb_hits[0].command, StudioCommand::OpenInBlockbenchStudio);

        let audit_hits = filter_commands("audit sem");
        assert!(
            audit_hits
                .iter()
                .any(|c| c.command == StudioCommand::RunSemanticAudit)
        );

        let mut form = PackMetadataForm::default();
        form.bedrock_min_engine_version = [26, 60, 0];
        form.enforce_bedrock_engine_version_invariant();
        assert_eq!(form.bedrock_min_engine_version[0], 1);
        form.regenerate_uuids();
        assert_ne!(form.bedrock_header_uuid, form.bedrock_module_uuid);
    }

    #[test]
    fn artist_nearest_neighbor_resize_biome_tint_alpha_mask_and_grass_tga() {
        let mut src = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let px = match (x < 2, y < 2) {
                    (true, true) => Rgba([255, 0, 0, 255]),
                    (false, true) => Rgba([0, 255, 0, 255]),
                    (true, false) => Rgba([0, 0, 255, 255]),
                    (false, false) => Rgba([200, 200, 200, 128]),
                };
                src.put_pixel(x, y, px);
            }
        }

        let down = resize_nearest_neighbor(&src, 2, 2).expect("nearest downscale");
        assert_eq!(down.dimensions(), (2, 2));
        assert_eq!(*down.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
        assert_eq!(*down.get_pixel(1, 0), Rgba([0, 255, 0, 255]));
        assert_eq!(*down.get_pixel(0, 1), Rgba([0, 0, 255, 255]));
        assert_eq!(*down.get_pixel(1, 1), Rgba([200, 200, 200, 128]));

        let mut gray = RgbaImage::new(2, 1);
        gray.put_pixel(0, 0, Rgba([200, 200, 200, 255]));
        gray.put_pixel(1, 0, Rgba([100, 100, 100, 77]));
        let tint = BiomeTintPreset::Plains.rgb_multiplier();
        let tinted = apply_biome_tint(&gray, tint);
        let p0 = tinted.get_pixel(0, 0);
        let p1 = tinted.get_pixel(1, 0);
        assert_eq!(p0[3], 255);
        assert_eq!(p1[3], 77);
        assert!(p0[1] > p0[0] && p0[0] > p0[2]);
        assert!(p1[1] > p1[0] && p1[0] > p1[2]);

        let alpha_preview = build_alpha_mask_preview(&gray, 128);
        assert_eq!(*alpha_preview.get_pixel(0, 0), Rgba([240, 240, 240, 255]));
        assert_eq!(*alpha_preview.get_pixel(1, 0), Rgba([235, 64, 180, 255]));

        let base = RgbaImage::from_pixel(16, 16, Rgba([120, 85, 55, 255]));
        let mut overlay = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        for x in 0..16 {
            overlay.put_pixel(x, 0, Rgba([180, 180, 180, 255]));
        }
        let (_img, tga_bytes) =
            composite_grass_side_tga(&base, Some(&overlay), Vec3::new(0.5, 0.8, 0.3), true)
                .expect("composite tga");
        let (decoded, fmt) =
            decode_texture_bytes(&tga_bytes, "textures/blocks/grass_side.tga").expect("decode tga");
        assert_eq!(fmt, TextureFormatKind::Tga);
        assert_eq!(decoded.dimensions(), (16, 16));
        assert_eq!(decoded.get_pixel(0, 0)[3], 0);
        assert_eq!(decoded.get_pixel(0, 15)[3], 255);
    }

    #[test]
    fn background_pipeline_preflight_convert_audit_synthetic_fixture_and_anti_false_positive() {
        let mut java_pack = PackContainer::new();
        java_pack
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":34,"description":"Synthetic Studio Fixture"}}"#.to_vec(),
            )
            .expect("insert mcmeta");
        java_pack
            .insert(
                "assets/minecraft/textures/block/stone.png",
                make_solid_png(16, 16, [128, 128, 128, 255]),
            )
            .expect("insert stone");

        let temp = TempDir::new().expect("tempdir");
        let out_mcpack = temp.path().join("out_bedrock.mcpack");

        let cfg = ConversionConfig {
            direction: ConversionDirection::JavaToBedrock,
            target_version: String::from("1.21.0"),
            output_path: out_mcpack.display().to_string(),
            convert_pbr: false,
            uv_atlas_guard: true,
            ..ConversionConfig::default()
        };

        let (tx, rx) = mpsc::channel();
        spawn_preflight(java_pack.clone(), cfg.clone(), tx);
        let preflight = loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("preflight msg") {
                BackgroundEvent::PreflightFinished(res) => break res.expect("preflight ok"),
                BackgroundEvent::Log { .. } => continue,
                _ => panic!("unexpected event"),
            }
        };
        assert_eq!(preflight.source_edition, Edition::Java);
        assert_eq!(preflight.target_edition, Edition::Bedrock);
        assert_eq!(preflight.ready_count, 1);
        assert_eq!(preflight.semantic_preview_status, AuditStatus::Passed);

        let (tx2, rx2) = mpsc::channel();
        spawn_convert(java_pack, cfg, tx2);
        let converted = loop {
            match rx2.recv_timeout(Duration::from_secs(10)).expect("convert msg") {
                BackgroundEvent::ConversionFinished(res) => break res.expect("convert ok"),
                BackgroundEvent::Log { .. } => continue,
                _ => panic!("unexpected event"),
            }
        };
        assert_eq!(converted.gate_status, AuditStatus::Passed);
        assert!(out_mcpack.is_file());

        let (tx3, rx3) = mpsc::channel();
        spawn_semantic_audit(converted.output_container.clone(), true, tx3);
        let audit = loop {
            match rx3.recv_timeout(Duration::from_secs(10)).expect("audit msg") {
                BackgroundEvent::AuditFinished(res) => break res.expect("audit ok"),
                BackgroundEvent::Log { .. } => continue,
                _ => panic!("unexpected event"),
            }
        };
        assert_eq!(audit.edition, Edition::Bedrock);
        assert_eq!(audit.status, AuditStatus::Passed);

        // Anti-False-Positive Test A: Bedrock pack with min_engine_version = [26, 60, 0] MUST FAIL
        let mut bad_bedrock = PackContainer::new();
        bad_bedrock
            .insert(
                "manifest.json",
                br#"{
                  "format_version": 2,
                  "header": {
                    "name": "Bad Major",
                    "description": "Invalid major 26",
                    "uuid": "11111111-1111-4111-8111-111111111111",
                    "version": [1, 0, 0],
                    "min_engine_version": [26, 60, 0]
                  },
                  "modules": [{
                    "type": "resources",
                    "uuid": "22222222-2222-4222-8222-222222222222",
                    "version": [1, 0, 0]
                  }]
                }"#
                .to_vec(),
            )
            .expect("insert bad manifest");
        bad_bedrock
            .insert(
                "textures/blocks/stone.png",
                make_solid_png(16, 16, [120, 120, 120, 255]),
            )
            .expect("insert stone");

        let (tx4, rx4) = mpsc::channel();
        spawn_semantic_audit(bad_bedrock, true, tx4);
        let bad_audit = loop {
            match rx4.recv_timeout(Duration::from_secs(10)).expect("bad audit msg") {
                BackgroundEvent::AuditFinished(res) => break res.expect("audit result"),
                BackgroundEvent::Log { .. } => continue,
                _ => panic!("unexpected event"),
            }
        };
        assert_eq!(bad_audit.status, AuditStatus::Failed);
        assert!(
            bad_audit
                .diagnostics
                .iter()
                .any(|d| d.severity == WarningSeverity::Error)
        );

        // Anti-False-Positive Test B: Unmappable custom texture (0 mapped textures) MUST FAIL
        let mut unmappable_java = PackContainer::new();
        unmappable_java
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":34,"description":"Unmappable"}}"#.to_vec(),
            )
            .expect("insert mcmeta");
        unmappable_java
            .insert(
                "assets/minecraft/textures/custom_mod/never_mapped.png",
                make_solid_png(16, 16, [255, 0, 255, 255]),
            )
            .expect("insert unmappable");

        let blocked_out = temp.path().join("must_not_exist.mcpack");
        let blocked_cfg = ConversionConfig {
            direction: ConversionDirection::JavaToBedrock,
            target_version: String::from("1.21.0"),
            output_path: blocked_out.display().to_string(),
            ..ConversionConfig::default()
        };
        let (tx5, rx5) = mpsc::channel();
        spawn_convert(unmappable_java, blocked_cfg, tx5);
        let blocked_res = loop {
            match rx5.recv_timeout(Duration::from_secs(10)).expect("blocked msg") {
                BackgroundEvent::ConversionFinished(res) => break res.expect("res ok"),
                BackgroundEvent::Log { .. } => continue,
                _ => panic!("unexpected event"),
            }
        };
        assert_eq!(blocked_res.gate_status, AuditStatus::Failed);
        assert!(!blocked_out.exists());
    }

    #[test]
    fn blockbench_tar_zst_extraction_and_version_cache_skips_reextract() {
        let temp = TempDir::new().expect("tempdir");
        let cache_dir = temp.path().join("blockbench-web");

        let archive_v1 = build_tar_zst_bytes(&[
            ("index.html", b"<html><body>Blockbench v1</body></html>"),
            ("studio_bridge.js", b"console.log('bridge');"),
            ("LICENSE.MD", b"GNU GENERAL PUBLIC LICENSE Version 3"),
        ]);

        // 1st run extracts cleanly and writes .version marker
        let first = ensure_extracted_from_bytes(&archive_v1, &cache_dir).expect("extract v1");
        assert!(first.extracted);
        assert!(cache_dir.join("index.html").is_file());
        assert!(cache_dir.join("studio_bridge.js").is_file());
        assert!(cache_dir.join("LICENSE.MD").is_file());
        assert_eq!(
            fs::read_to_string(cache_dir.join(".version"))
                .expect("read version")
                .trim(),
            first.sha256
        );

        // Place a sentinel file to prove the 2nd run does NOT wipe or re-extract
        let sentinel = cache_dir.join(".sentinel_cache_hit");
        fs::write(&sentinel, b"untouched").expect("write sentinel");

        let second = ensure_extracted_from_bytes(&archive_v1, &cache_dir).expect("cache hit");
        assert!(!second.extracted, "matching .version hash must skip re-extraction");
        assert_eq!(second.sha256, first.sha256);
        assert!(sentinel.is_file(), "cache directory must not be rewritten on cache hit");

        // Changing archive contents changes SHA-256 and triggers clean re-extraction
        let archive_v2 = build_tar_zst_bytes(&[
            ("index.html", b"<html><body>Blockbench v2</body></html>"),
            ("LICENSE.MD", b"GNU GPL v3"),
        ]);
        let third = ensure_extracted_from_bytes(&archive_v2, &cache_dir).expect("re-extract v2");
        assert!(third.extracted);
        assert_ne!(third.sha256, first.sha256);
        assert!(!sentinel.exists(), "stale cache should be replaced cleanly");
        assert!(
            fs::read_to_string(cache_dir.join("index.html"))
                .expect("v2 html")
                .contains("Blockbench v2")
        );
    }

    #[test]
    fn blockbench_bridge_path_traversal_guard_and_roundtrip_save_json_png() {
        // 1. Path traversal guard MUST reject escaping paths
        for malicious in [
            "../outside.json",
            "models/../../outside.json",
            "/etc/passwd",
            r"C:\Windows\evil.json",
            "textures/a\0b.png",
        ] {
            assert!(
                validate_bridge_relative_path(malicious).is_err(),
                "expected traversal guard to reject '{malicious}'"
            );
        }

        let temp = TempDir::new().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("mkdir project");

        let bad_payload = BridgeSavePayload {
            model_path: Some(String::from("../outside.json")),
            model_content: Some(String::from("{}")),
            textures: Vec::new(),
        };
        assert!(apply_bridge_save_payload(Some(&project_root), &bad_payload).is_err());
        assert!(!temp.path().join("outside.json").exists());

        // 2. Live loopback HTTP round-trip Save of .geo.json + PNG texture
        let web_root = temp.path().join("web");
        fs::create_dir_all(&web_root).expect("mkdir web");
        fs::write(web_root.join("index.html"), b"<html>ok</html>").expect("index");

        let (save_tx, save_rx) = mpsc::channel();
        let server = BlockbenchServer::start(
            web_root,
            Some(project_root.clone()),
            save_tx,
        )
        .expect("start loopback server");

        let sample_png = make_solid_png(16, 16, [0x33, 0xCC, 0x66, 0xFF]);
        let sample_geo = r#"{"format_version":"1.21.0","minecraft:geometry":[{"description":{"identifier":"geometry.test_cube"}}]}"#;
        let payload = BridgeSavePayload {
            model_path: Some(String::from("models/entity/test_cube.geo.json")),
            model_content: Some(sample_geo.to_string()),
            textures: vec![BridgeTexturePayload {
                path: String::from("textures/entity/test_cube.png"),
                name: Some(String::from("test_cube.png")),
                data_url: None,
                png_base64: Some(BASE64_STANDARD.encode(&sample_png)),
            }],
        };
        let body = serde_json::to_vec(&payload).expect("serialize save payload");

        let mut stream =
            TcpStream::connect(("127.0.0.1", server.port())).expect("connect loopback");
        let req = format!(
            "POST /api/save?token={} HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n",
            server.token(),
            server.port(),
            body.len()
        );
        stream.write_all(req.as_bytes()).expect("write header");
        stream.write_all(&body).expect("write body");

        let mut resp_text = String::new();
        stream.read_to_string(&mut resp_text).expect("read resp");
        assert!(resp_text.starts_with("HTTP/1.1 200 OK"), "unexpected resp: {resp_text}");

        // Verify files written on disk inside project_root
        assert_eq!(
            fs::read_to_string(project_root.join("models/entity/test_cube.geo.json"))
                .expect("read geo"),
            sample_geo
        );
        assert_eq!(
            fs::read(project_root.join("textures/entity/test_cube.png")).expect("read png"),
            sample_png
        );

        // Verify UI thread received BridgeSaveEvent
        let ev = save_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("bridge save event");
        assert_eq!(ev.saved_files.len(), 2);
    }

    #[test]
    fn wizard_scaffolds_java_bedrock_rp_and_bedrock_addon_with_linked_uuids() {
        let temp = TempDir::new().expect("tempdir");

        // 1. Java Edition Resource Pack
        let java_cfg = NewProjectWizardState {
            kind: ProjectTemplateKind::JavaResourcePack,
            pack_name: String::from("Alpine HD Pack"),
            description: String::from("§bCrisp §fJava Pack"),
            authors: String::from("Alice, Bob"),
            license: String::from("MIT"),
            url: String::from("https://example.com/alpine"),
            version: [1, 2, 0],
            java_pack_format: 34,
            java_use_supported_formats: true,
            java_supported_min: 15,
            java_supported_max: 64,
            java_namespace: String::from("MyCustomNS"),
            target_parent_dir: temp.path().display().to_string(),
            ..NewProjectWizardState::default()
        };
        let java_res = scaffold_new_project(&java_cfg).expect("scaffold java");
        assert!(java_res.open_dir.join("pack.mcmeta").is_file());
        assert!(java_res.open_dir.join("pack.png").is_file());
        assert!(
            java_res
                .open_dir
                .join("assets/mycustomns/textures/block/stone.png")
                .is_file()
        );
        assert!(java_res.open_dir.join("assets/mycustomns/models/block").is_dir());
        assert!(java_res.open_dir.join("assets/mycustomns/blockstates").is_dir());
        let mcmeta_bytes = fs::read(java_res.open_dir.join("pack.mcmeta")).expect("read mcmeta");
        let mcmeta: JavaPackMcmeta = parse_minecraft_json(&mcmeta_bytes).expect("valid mcmeta");
        let (fmt, range, desc) = mcmeta.validate_and_extract().expect("validate mcmeta");
        assert_eq!(fmt, 34);
        assert_eq!(range, Some([15, 64]));
        assert_eq!(desc, "§bCrisp §fJava Pack");

        // 2. Bedrock Resource Pack (even if major=26 passed, must normalize to [1, x, x])
        let bedrock_cfg = NewProjectWizardState {
            kind: ProjectTemplateKind::BedrockResourcePack,
            pack_name: String::from("Bedrock Neon Pack"),
            authors: String::from("Artist One, Artist Two"),
            bedrock_min_engine_version: [26, 60, 0],
            target_parent_dir: temp.path().display().to_string(),
            ..NewProjectWizardState::default()
        };
        let br_res = scaffold_new_project(&bedrock_cfg).expect("scaffold bedrock rp");
        assert!(br_res.open_dir.join("manifest.json").is_file());
        assert!(br_res.open_dir.join("pack_icon.png").is_file());
        assert!(br_res.open_dir.join("textures/terrain_texture.json").is_file());
        assert!(br_res.open_dir.join("textures/item_texture.json").is_file());
        assert!(br_res.open_dir.join("models/entity").is_dir());

        let manifest_bytes = fs::read(br_res.open_dir.join("manifest.json")).expect("read manifest");
        let manifest: BedrockManifest =
            parse_minecraft_json(&manifest_bytes).expect("valid bedrock manifest");
        assert_eq!(manifest.header.min_engine_version[0], 1);
        let authors = manifest
            .metadata
            .as_ref()
            .expect("metadata present")
            .authors
            .clone();
        assert_eq!(authors, vec!["Artist One", "Artist Two"]);

        // 3. Bedrock Add-On Workspace (RP + BP linked via dependencies UUID)
        let addon_cfg = NewProjectWizardState {
            kind: ProjectTemplateKind::BedrockAddonWorkspace,
            pack_name: String::from("Dragon Addon"),
            authors: String::from("Studio Team"),
            bedrock_min_engine_version: [1, 21, 50],
            target_parent_dir: temp.path().display().to_string(),
            ..NewProjectWizardState::default()
        };
        let addon_res = scaffold_new_project(&addon_cfg).expect("scaffold addon");
        let rp_dir = addon_res.workspace_root.join("RP");
        let bp_dir = addon_res.companion_bp_dir.expect("companion BP dir");
        assert!(rp_dir.join("manifest.json").is_file());
        assert!(bp_dir.join("manifest.json").is_file());

        let rp_json: serde_json::Value =
            serde_json::from_slice(&fs::read(rp_dir.join("manifest.json")).expect("rp json"))
                .expect("parse rp");
        let bp_json: serde_json::Value =
            serde_json::from_slice(&fs::read(bp_dir.join("manifest.json")).expect("bp json"))
                .expect("parse bp");

        let rp_uuid = rp_json["header"]["uuid"].as_str().expect("rp uuid");
        let bp_uuid = bp_json["header"]["uuid"].as_str().expect("bp uuid");
        assert_ne!(rp_uuid, bp_uuid);
        assert_eq!(rp_json["dependencies"][0]["uuid"].as_str(), Some(bp_uuid));
        assert_eq!(bp_json["dependencies"][0]["uuid"].as_str(), Some(rp_uuid));
        assert_eq!(rp_json["header"]["min_engine_version"][0].as_u64(), Some(1));
        assert_eq!(bp_json["header"]["min_engine_version"][0].as_u64(), Some(1));
    }

    #[test]
    fn packager_exports_clean_zip_and_mcpack_stripping_os_junk_without_double_nesting() {
        let temp = TempDir::new().expect("tempdir");

        // Java .zip with OS/VCS junk injected into container
        let mut java_c = PackContainer::new();
        java_c
            .insert(
                "pack.mcmeta",
                br#"{"pack":{"pack_format":34,"description":"Clean Zip Test"}}"#.to_vec(),
            )
            .expect("mcmeta");
        java_c
            .insert(
                "assets/minecraft/textures/block/stone.png",
                make_solid_png(16, 16, [100, 100, 100, 255]),
            )
            .expect("stone");
        java_c.insert("desktop.ini", b"junk".to_vec()).expect("junk1");
        java_c.insert(".DS_Store", b"junk".to_vec()).expect("junk2");
        java_c.insert(".git/HEAD", b"ref: main".to_vec()).expect("junk3");
        java_c.insert("notes.tmp", b"temp".to_vec()).expect("junk4");

        let zip_cfg = ExportPackageConfig {
            format: PackageFormat::Zip,
            filename_template: String::from("{pack_name}-v{version}-{edition}"),
            output_dir: temp.path().display().to_string(),
            strip_junk_files: true,
            compression: CompressionPreset::Maximum,
            run_semantic_audit: true,
            allow_export_with_errors: false,
            ..ExportPackageConfig::default()
        };
        let zip_sum = export_pack_archive(
            &java_c,
            None,
            &zip_cfg,
            Edition::Java,
            "My Pack",
            [1, 0, 0],
            true,
        )
        .expect("export java zip");
        assert_eq!(zip_sum.stripped_junk_count, 4);
        assert!(zip_sum.output_path.ends_with("my_pack-v1.0.0-java.zip"));

        let mut zip_archive =
            ZipArchive::new(File::open(&zip_sum.output_path).expect("open zip")).expect("read zip");
        let names: Vec<String> = (0..zip_archive.len())
            .map(|i| zip_archive.by_index(i).expect("entry").name().to_string())
            .collect();
        assert!(names.contains(&String::from("pack.mcmeta")));
        assert!(names.contains(&String::from("assets/minecraft/textures/block/stone.png")));
        assert!(!names.iter().any(|n| n.contains("desktop.ini") || n.contains(".DS_Store") || n.contains(".git")));

        // Bedrock .mcpack clean root check
        let mut br_pack = create_new_bedrock_pack("Clean Bedrock").expect("bedrock starter");
        br_pack
            .container
            .insert("Thumbs.db", b"junk".to_vec())
            .expect("thumbs");
        let mcpack_cfg = ExportPackageConfig {
            format: PackageFormat::Mcpack,
            filename_template: String::from("{pack_name}-v{version}-{edition}"),
            output_dir: temp.path().display().to_string(),
            strip_junk_files: true,
            compression: CompressionPreset::Fast,
            run_semantic_audit: true,
            allow_export_with_errors: false,
            ..ExportPackageConfig::default()
        };
        let mc_sum = export_pack_archive(
            &br_pack.container,
            None,
            &mcpack_cfg,
            Edition::Bedrock,
            "Clean Bedrock",
            [2, 1, 0],
            true,
        )
        .expect("export bedrock mcpack");
        assert_eq!(mc_sum.stripped_junk_count, 1);
        assert!(mc_sum.output_path.ends_with("clean_bedrock-v2.1.0-bedrock.mcpack"));

        let mut mc_archive =
            ZipArchive::new(File::open(&mc_sum.output_path).expect("open mcpack")).expect("zip");
        let mc_names: Vec<String> = (0..mc_archive.len())
            .map(|i| mc_archive.by_index(i).expect("entry").name().to_string())
            .collect();
        assert!(mc_names.contains(&String::from("manifest.json")));
        assert!(!mc_names.iter().any(|n| n.ends_with("Thumbs.db")));
    }

    #[test]
    fn explorer_filename_sanitizer_and_minecraft_resource_id_generator() {
        assert_eq!(
            sanitize_minecraft_filename("My Cool Block!.PNG"),
            "my_cool_block.png"
        );
        assert_eq!(
            sanitize_minecraft_filename("My Sword 01.png"),
            "my_sword_01.png"
        );
        assert_eq!(
            sanitize_minecraft_filename("Dragon Wing.geo.json"),
            "dragon_wing.geo.json"
        );
        assert_eq!(
            sanitize_minecraft_filename("Water Flow.png.mcmeta"),
            "water_flow.png.mcmeta"
        );

        assert_eq!(
            generate_minecraft_resource_id(
                "assets/minecraft/textures/block/stone.png",
                Edition::Java
            ),
            "minecraft:block/stone"
        );
        assert_eq!(
            generate_minecraft_resource_id(
                "assets/my_mod/models/item/ruby_sword.json",
                Edition::Java
            ),
            "my_mod:item/ruby_sword"
        );
        assert_eq!(
            generate_minecraft_resource_id("textures/blocks/stone.png", Edition::Bedrock),
            "textures/blocks/stone"
        );
    }

    #[test]
    fn loaded_pack_from_synthetic_java_classifies_groups_and_runs_headless_ui_frame() {
        let mut container = PackContainer::new();
        container
            .insert(
                "pack.mcmeta",
                r#"{"pack":{"pack_format":34,"description":"§bUI Smoke"}}"#
                    .as_bytes()
                    .to_vec(),
            )
            .expect("insert");
        container
            .insert(
                "assets/minecraft/textures/block/stone.png",
                make_solid_png(32, 32, [90, 90, 90, 255]),
            )
            .expect("insert");
        let loaded = LoadedPack::from_container(PackSource::InMemory(String::from("Smoke")), container)
            .expect("load");
        assert_eq!(loaded.edition, Edition::Java);
        assert_eq!(loaded.texture_count, 1);

        let mut app = crate::PackStudioApp::new(None);
        app.state.set_loaded_pack(loaded);
        app.state.open_asset_tab("assets/minecraft/textures/block/stone.png");
        app.state.open_asset_tab("pack.mcmeta");
        app.state.open_conversion_dashboard();

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(1440.0, 900.0),
            )),
            ..egui::RawInput::default()
        };
        for activity in ActivityTab::ALL {
            app.state.focus_activity(activity);
            app.state.command_palette_open = activity == ActivityTab::Settings;
            let _ = ctx.run(raw_input.clone(), |ctx| {
                app.render_ui(ctx);
            });
        }
        app.state.command_palette_open = false;
        app.state.wizard.open = true;
        let _ = ctx.run(raw_input.clone(), |ctx| app.render_ui(ctx));
        app.state.wizard.open = false;
        app.state.export_config.open = true;
        let _ = ctx.run(raw_input.clone(), |ctx| app.render_ui(ctx));
        app.state.export_config.open = false;
        app.state.quick_open_visible = true;
        let _ = ctx.run(raw_input.clone(), |ctx| app.render_ui(ctx));
        app.state.quick_open_visible = false;

        for key in [
            "__welcome__",
            "tex:assets/minecraft/textures/block/stone.png",
            "code:pack.mcmeta",
            "__converter__",
        ] {
            assert!(app.state.focus_tab_by_key(key));
            let _ = ctx.run(raw_input.clone(), |ctx| {
                app.render_ui(ctx);
            });
        }
    }
}
