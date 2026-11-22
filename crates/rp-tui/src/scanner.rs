//! Re-exports of shared pack scanning, auditing, diffing, and unpack/repack operations from `rp-cli`.

pub use rp_cli::manager::{
    AuditOutcome, CategoryDiff, DiscoveredPack, PackDiff, PackSortKey, ProjectedQuad3D,
    ProjectedScene3D, audit_pack_roundtrip, diff_packs, export_audit_json, export_report_json,
    format_bytes, load_pack_ir, project_model_3d, repack_dir_to_archive, scan_directory_for_packs,
    sort_discovered_packs, suggest_output_path, unpack_pack_to_dir,
};
