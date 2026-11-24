pub mod blockbench;
pub mod converter;
pub mod extensions;
pub mod lsp;
pub mod pty;
pub mod state;
pub mod texture;
pub mod workspace;

use state::StudioState;

/// Launches the Pack-Studio Tauri v2 application runtime.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .manage(StudioState::default())
        .invoke_handler(tauri::generate_handler![
            workspace::open_project,
            workspace::create_project,
            workspace::fs_create_file,
            workspace::fs_create_dir,
            workspace::fs_rename,
            workspace::fs_delete,
            workspace::fs_import_assets,
            workspace::fs_read_text,
            workspace::fs_write_text,
            workspace::fs_read_binary_base64,
            workspace::fs_write_binary_base64,
            workspace::detect_file_managers,
            workspace::pick_with_yazi,
            workspace::reveal_in_file_manager,
            workspace::get_pack_metadata,
            workspace::save_pack_metadata,
            converter::run_preflight,
            converter::execute_convert,
            converter::run_audit,
            converter::package_pack,
            texture::create_blank_texture,
            texture::downscale_texture,
            texture::composite_grass_side,
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_kill,
            blockbench::blockbench_start,
            blockbench::blockbench_open_asset,
            blockbench::blockbench_new_bedrock_geo,
            extensions::ext_list,
            extensions::ext_install,
            extensions::ext_set_enabled,
            extensions::ext_uninstall,
            lsp::lsp_list_servers,
            lsp::lsp_register_custom_server,
            lsp::lsp_unregister_custom_server,
            lsp::lsp_start_server,
            lsp::lsp_stop_server,
            lsp::lsp_sync_document,
            lsp::lsp_completion,
            lsp::lsp_hover,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pack-Studio Tauri application");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Read;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use image::{Rgba, RgbaImage};
    use rp_core::{AuditStatus, Edition};
    use rp_io::PackContainer;
    use tempfile::TempDir;
    use zip::ZipArchive;

    use crate::blockbench::{
        BridgeSavePayload, BridgeTexturePayload, apply_bridge_save_payload,
        validate_bridge_relative_path,
    };
    use crate::converter::{
        ConverterOptions, PackageExportConfig, UvAtlasGuardMode, execute_convert_core,
        package_pack_core, run_audit_core, run_preflight_core,
    };
    use crate::texture::{
        TextureFormatKind, composite_grass_side_tga, create_blank_png_bytes, encode_texture_bytes,
        resize_nearest_neighbor,
    };
    use crate::workspace::{
        NewProjectConfig, ProjectTemplateKind, generate_minecraft_resource_id,
        parse_yazi_chooser_output, sanitize_minecraft_filename, scaffold_project_on_disk,
    };

    fn sample_png_bytes(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_pixel(width, height, Rgba(rgba));
        encode_texture_bytes(&img, TextureFormatKind::Png).expect("encode png")
    }

    fn minimal_java_pack_container() -> PackContainer {
        let mut c = PackContainer::new();
        c.insert(
            "pack.mcmeta",
            br#"{"pack":{"pack_format":34,"description":"Test Java Pack"}}"#.to_vec(),
        )
        .unwrap();
        c.insert(
            "assets/minecraft/textures/block/stone.png",
            sample_png_bytes(16, 16, [120, 120, 120, 255]),
        )
        .unwrap();
        c.insert(
            "assets/minecraft/textures/block/grass_block_side.png",
            sample_png_bytes(16, 16, [110, 80, 50, 255]),
        )
        .unwrap();
        c.insert(
            "assets/minecraft/textures/block/grass_block_side_overlay.png",
            sample_png_bytes(16, 16, [80, 180, 60, 200]),
        )
        .unwrap();
        c
    }

    #[test]
    fn scaffolds_three_project_templates_and_enforces_bedrock_engine_triplet() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path().display().to_string();

        let java_dir = scaffold_project_on_disk(&NewProjectConfig {
            kind: ProjectTemplateKind::JavaResourcePack,
            pack_name: String::from("Solar Java"),
            description: String::from("§bCrisp Java Textures"),
            authors: vec![String::from("Artist")],
            license: String::from("MIT"),
            url: String::new(),
            version: [1, 0, 0],
            java_pack_format: 46,
            java_supported_formats: Some([34, 46]),
            java_namespace: String::from("SolarMod"),
            bedrock_min_engine_version: [1, 21, 50],
            icon_path: None,
            target_parent_dir: parent.clone(),
        })
        .unwrap();
        assert!(java_dir.join("pack.mcmeta").is_file());
        assert!(java_dir.join("assets/solarmod/textures/block").is_dir());

        let bedrock_dir = scaffold_project_on_disk(&NewProjectConfig {
            kind: ProjectTemplateKind::BedrockResourcePack,
            pack_name: String::from("Solar Bedrock"),
            description: String::from("Bedrock Pack"),
            authors: vec![String::from("Artist")],
            license: String::from("CC0"),
            url: String::new(),
            version: [1, 2, 0],
            java_pack_format: 34,
            java_supported_formats: None,
            java_namespace: String::from("minecraft"),
            bedrock_min_engine_version: [26, 0, 0],
            icon_path: None,
            target_parent_dir: parent.clone(),
        })
        .unwrap();
        let manifest_txt = fs::read_to_string(bedrock_dir.join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_txt).unwrap();
        assert_eq!(manifest["header"]["min_engine_version"][0], 1);

        let addon_rp = scaffold_project_on_disk(&NewProjectConfig {
            kind: ProjectTemplateKind::BedrockAddonWorkspace,
            pack_name: String::from("Solar Addon"),
            description: String::from("RP+BP"),
            authors: vec![],
            license: String::new(),
            url: String::new(),
            version: [0, 1, 0],
            java_pack_format: 34,
            java_supported_formats: None,
            java_namespace: String::from("minecraft"),
            bedrock_min_engine_version: [1, 21, 0],
            icon_path: None,
            target_parent_dir: parent,
        })
        .unwrap();
        let rp_manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(addon_rp.join("manifest.json")).unwrap(),
        )
        .unwrap();
        let bp_manifest_path = addon_rp.parent().unwrap().join("BP/manifest.json");
        assert!(bp_manifest_path.is_file());
        let bp_manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(bp_manifest_path).unwrap()).unwrap();
        assert_eq!(
            bp_manifest["dependencies"][0]["uuid"],
            rp_manifest["header"]["uuid"]
        );
    }

    #[test]
    fn sanitizes_filenames_and_generates_resource_ids_and_parses_yazi() {
        assert_eq!(
            sanitize_minecraft_filename("My Cool Block!.PNG"),
            "my_cool_block.png"
        );
        assert_eq!(
            sanitize_minecraft_filename("Robot Model (v2).GEO.JSON"),
            "robot_model_v2.geo.json"
        );
        assert_eq!(
            generate_minecraft_resource_id(
                "assets/custom_ns/textures/block/oak_planks.png",
                Edition::Java
            ),
            "custom_ns:block/oak_planks"
        );
        assert_eq!(
            generate_minecraft_resource_id("textures/blocks/stone.png", Edition::Bedrock),
            "textures/blocks/stone"
        );

        let paths = parse_yazi_chooser_output("\n/tmp/a.png\r\n/tmp/b.json\n");
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].to_string_lossy(), "/tmp/a.png");
        assert_eq!(paths[1].to_string_lossy(), "/tmp/b.json");
    }

    #[test]
    fn texture_downscale_and_grass_composite_work() {
        let blank = create_blank_png_bytes(64, 64).unwrap();
        let img = image::load_from_memory(&blank).unwrap().into_rgba8();
        let downscaled = resize_nearest_neighbor(&img, 16, 16).unwrap();
        assert_eq!(downscaled.dimensions(), (16, 16));

        let side = RgbaImage::from_pixel(16, 16, Rgba([100, 70, 40, 255]));
        let overlay = RgbaImage::from_pixel(16, 16, Rgba([0, 255, 0, 255]));
        let (_comp_img, tga_bytes) = composite_grass_side_tga(
            &side,
            Some(&overlay),
            glam::Vec3::new(0.474, 0.753, 0.353),
            false,
        )
        .unwrap();
        let decoded = image::load_from_memory_with_format(&tga_bytes, image::ImageFormat::Tga)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.dimensions(), (16, 16));
    }

    #[test]
    fn preflight_convert_and_package_strip_junk_without_false_gate_failure() {
        let mut java = minimal_java_pack_container();
        java.insert(".DS_Store", b"junk".to_vec()).unwrap();
        java.insert("desktop.ini", b"junk".to_vec()).unwrap();

        let opts = ConverterOptions {
            target_version: String::from("1.21.0"),
            composite_grass_tga: true,
            uv_atlas_guard_mode: UvAtlasGuardMode::SafeFallback2d,
            ..ConverterOptions::default()
        };

        let preflight = run_preflight_core(&java, "java_to_bedrock", &opts).unwrap();
        assert_eq!(preflight.source_edition, Edition::Java);
        assert_eq!(preflight.target_edition, Edition::Bedrock);
        assert!(preflight.ready_count >= 1);

        let tmp = TempDir::new().unwrap();
        let bedrock_dir = tmp.path().join("out_bedrock");
        let (converted, res) = execute_convert_core(
            &java,
            &bedrock_dir.display().to_string(),
            "java_to_bedrock",
            &opts,
            |_, _| {},
        )
        .unwrap();
        assert_ne!(res.gate_status, AuditStatus::Failed);
        assert!(converted.get("manifest.json").is_some());

        let mut converted_with_junk = converted;
        converted_with_junk
            .insert(".DS_Store", b"finder-junk".to_vec())
            .unwrap();
        converted_with_junk
            .insert("desktop.ini", b"windows-junk".to_vec())
            .unwrap();

        let audit_res = run_audit_core(&java).unwrap();
        assert_ne!(audit_res.status, AuditStatus::Failed);

        let summary = package_pack_core(
            &converted_with_junk,
            None,
            &PackageExportConfig {
                format: String::from("mcpack"),
                filename_template: String::from("{pack_name}_{version}"),
                output_dir: tmp.path().display().to_string(),
                strip_junk_files: true,
                compression: String::from("balanced"),
                run_semantic_audit: true,
                allow_export_with_errors: false,
            },
            Edition::Bedrock,
            "Sample Pack",
            [1, 0, 0],
        )
        .unwrap();
        assert_eq!(summary.stripped_junk_count, 2);

        let zip_file = fs::File::open(&summary.output_path).unwrap();
        let mut archive = ZipArchive::new(zip_file).unwrap();
        for idx in 0..archive.len() {
            let mut entry = archive.by_index(idx).unwrap();
            let name = entry.name().to_string();
            assert!(!name.contains(".DS_Store"));
            assert!(!name.contains("desktop.ini"));
            let mut buf = Vec::new();
            let _ = entry.read_to_end(&mut buf).unwrap();
        }
    }

    #[test]
    fn blockbench_bridge_guard_blocks_path_traversal_and_writes_safe_assets() {
        assert!(validate_bridge_relative_path("../secret.txt").is_err());
        assert!(validate_bridge_relative_path("/etc/passwd").is_err());
        assert!(validate_bridge_relative_path("C:\\evil.png").is_err());
        assert_eq!(
            validate_bridge_relative_path("models/entity/chair.geo.json").unwrap(),
            "models/entity/chair.geo.json"
        );

        let tmp = TempDir::new().unwrap();
        let png = sample_png_bytes(8, 8, [255, 0, 0, 255]);
        let saved = apply_bridge_save_payload(
            Some(tmp.path()),
            &BridgeSavePayload {
                model_path: Some(String::from("models/entity/chair.geo.json")),
                model_content: Some(String::from("{\"format_version\":\"1.12.0\"}")),
                textures: vec![BridgeTexturePayload {
                    id: None,
                    path: String::from("textures/entity/chair.png"),
                    name: Some(String::from("chair")),
                    data_url: None,
                    png_base64: Some(BASE64_STANDARD.encode(&png)),
                }],
            },
        )
        .unwrap();
        assert_eq!(saved.len(), 2);
        assert!(tmp.path().join("models/entity/chair.geo.json").is_file());
        assert!(tmp.path().join("textures/entity/chair.png").is_file());
    }

    #[test]
    fn lsp_framing_and_builtin_minecraft_diagnostics_work() {
        use std::io::BufReader;

        let payload = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let framed = crate::lsp::frame_lsp_message(&payload);
        let mut reader = BufReader::new(&framed[..]);
        let decoded = crate::lsp::read_lsp_message(&mut reader).unwrap().unwrap();
        assert_eq!(decoded["id"], 1);
        assert_eq!(decoded["method"], "initialize");

        let bad_manifest = r#"{"format_version":2,"header":{"name":"X","min_engine_version":[26,0,0]},"modules":[]}"#;
        let diags = crate::lsp::builtin_minecraft_diagnostics("manifest.json", bad_manifest, &[]);
        assert!(diags.len() >= 2);

        let atlas = r#"{"texture_data":{"missing_tex":{"textures":"textures/blocks/missing"}}}"#;
        let atlas_diags = crate::lsp::builtin_minecraft_diagnostics(
            "textures/terrain_texture.json",
            atlas,
            &[String::from("textures/blocks/stone.png")],
        );
        assert_eq!(atlas_diags.len(), 1);
        assert!(atlas_diags[0].message.contains("missing"));
    }

    #[test]
    fn vsix_extension_installs_declarative_schemas_and_snippets_safely() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let tmp = TempDir::new().unwrap();
        let vsix_path = tmp.path().join("sample-mc.vsix");
        let ext_root = tmp.path().join("extensions");

        let file = fs::File::create(&vsix_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zip.start_file("extension/package.json", opts).unwrap();
        zip.write_all(
            br#"{
              "name": "mc-helper",
              "publisher": "community",
              "displayName": "Minecraft Helper Pack",
              "version": "1.2.0",
              "main": "./dist/extension.js",
              "contributes": {
                "jsonValidation": [{ "fileMatch": ["*.mc.json"], "url": "./schemas/mc.json" }],
                "snippets": [{ "language": "json", "path": "./snippets/mc.json" }]
              }
            }"#,
        )
        .unwrap();
        zip.start_file("extension/schemas/mc.json", opts).unwrap();
        zip.write_all(br#"{"type":"object"}"#).unwrap();
        zip.start_file("extension/snippets/mc.json", opts).unwrap();
        zip.write_all(
            br#"{"Sample Item":{"prefix":"mc-item","body":["{\"format_version\":\"1.20.0\"}"],"description":"Item"}}"#,
        )
        .unwrap();
        zip.finish().unwrap();

        let installed = crate::extensions::install_extension_core(&vsix_path, &ext_root).unwrap();
        assert_eq!(installed.id, "community.mc-helper");
        assert!(installed.has_js_entrypoint);
        assert!(installed.runtime_note.is_some());
        assert_eq!(installed.schema_count, 1);
        assert_eq!(installed.snippet_count, 1);

        let snap = crate::extensions::scan_extensions_snapshot(&ext_root);
        assert!(snap.extensions.iter().any(|e| e.id == "community.mc-helper"));
        assert!(snap.active_snippets.iter().any(|s| s.prefix == "mc-item"));
    }
}
