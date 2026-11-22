export type Edition = "java" | "bedrock";
export type AuditStatus = "passed" | "partial" | "failed";
export type WarningSeverity = "info" | "warning" | "error";
export type PipelineStage = "scan" | "resolve" | "transform" | "emit" | "audit";

export type ProjectTemplateKind =
  | "java_resource_pack"
  | "bedrock_resource_pack"
  | "bedrock_addon_workspace";

export type UvAtlasGuardMode = "safe_fallback2d" | "skip" | "warn";
export type RotationSnapMode = "Preserve" | "Snap22_5" | "Clamp45";

export interface FileTreeNodeDto {
  name: string;
  rel_path: string;
  is_dir: boolean;
  children: FileTreeNodeDto[];
}

export interface ProjectSnapshot {
  pack_name: string;
  source_path: string | null;
  is_directory: boolean;
  companion_bp_dir: string | null;
  edition: Edition;
  edition_label: string;
  file_count: number;
  texture_count: number;
  dominant_resolution: string | null;
  all_files: string[];
  tree: FileTreeNodeDto[];
}

export interface NewProjectConfig {
  kind: ProjectTemplateKind;
  pack_name: string;
  description: string;
  authors: string[];
  license: string;
  url: string;
  version: [number, number, number];
  java_pack_format: number;
  java_supported_formats: [number, number] | null;
  java_namespace: string;
  bedrock_min_engine_version: [number, number, number];
  icon_path: string | null;
  target_parent_dir: string;
}

export interface BinaryAssetData {
  rel_path: string;
  base64_png: string;
  raw_base64: string;
  width: number | null;
  height: number | null;
  format: string;
}

export interface RenameResult {
  old_rel_path: string;
  new_rel_path: string;
  snapshot: ProjectSnapshot;
}

export interface ImportResult {
  imported_paths: string[];
  skipped_paths: string[];
  snapshot: ProjectSnapshot;
}

export interface DetectedFileManagers {
  primary: string;
  available: string[];
  yazi_available: boolean;
  yazi_path: string | null;
  terminal_emulator: string | null;
  yazi_install_hint: string;
}

export interface PackMetadataDto {
  edition: Edition;
  name: string;
  description: string;
  authors: string[];
  license: string;
  url: string;
  version: [number, number, number];
  java_pack_format: number;
  java_supported_formats: [number, number] | null;
  bedrock_format_version: number;
  bedrock_min_engine_version: [number, number, number];
  bedrock_header_uuid: string;
  bedrock_module_uuid: string;
  bedrock_capabilities: string[];
  icon_base64_png: string | null;
}

export interface ConversionWarning {
  source_path: string;
  stage: PipelineStage;
  severity: WarningSeverity;
  reason: string;
  actionable_fix: string;
  asset: string;
  message: string;
}

export interface ConverterOptions {
  target_version: string;
  composite_grass_tga: boolean;
  uv_atlas_guard_mode: UvAtlasGuardMode;
  convert_pbr: boolean;
  auto_downscale: number | null;
  strip_junk_and_raw_java: boolean;
  rotation_mode: RotationSnapMode;
  bedrock_manifest_version: number;
}

export interface ConvertProgressEvent {
  fraction: number;
  message: string;
}

export interface PreflightRowDto {
  source_path: string;
  category: string;
  status: string;
  target_hint: string;
  detail: string;
}

export interface PreflightReportDto {
  source_edition: Edition;
  target_edition: Edition;
  total_assets: number;
  ready_count: number;
  composite_count: number;
  lossy_count: number;
  skipped_count: number;
  warning_count: number;
  rows: PreflightRowDto[];
  diagnostics: ConversionWarning[];
  semantic_preview_status: AuditStatus;
  semantic_preview_summary: string;
  elapsed_ms: number;
}

export interface ConversionResultDto {
  source_edition: Edition;
  target_edition: Edition;
  output_written_to: string | null;
  converted_count: number;
  passthrough_count: number;
  skipped_count: number;
  lossy_count: number;
  warnings: ConversionWarning[];
  gate_status: AuditStatus;
  gate_summary: string;
  elapsed_ms: number;
}

export interface AuditResultDto {
  edition: Edition;
  status: AuditStatus;
  summary: string;
  problems: ConversionWarning[];
  elapsed_ms: number;
}

export interface PackageExportConfig {
  format: "zip" | "mcpack" | "mcaddon";
  filename_template: string;
  output_dir: string;
  strip_junk_files: boolean;
  compression: "fast" | "balanced" | "maximum";
  run_semantic_audit: boolean;
  allow_export_with_errors: boolean;
}

export interface PackageExportSummary {
  output_path: string;
  size_bytes: number;
  asset_count: number;
  stripped_junk_count: number;
  audit_status: AuditStatus | null;
  audit_summary: string;
}

export interface TextureTransformResult {
  path: string;
  width: number;
  height: number;
  base64_png: string;
  snapshot: ProjectSnapshot;
}

export interface BlockbenchLaunchInfo {
  url: string;
  port: number;
  sha256: string;
  extracted_now: boolean;
}

export interface BlockbenchSavedEvent {
  saved_files: string[];
}

export interface PtyDataEvent {
  session_id: number;
  data: string;
}

export interface PtyExitEvent {
  session_id: number;
}

export type ActivityView =
  | "explorer"
  | "search"
  | "converter"
  | "metadata"
  | "settings";

export type BottomTab = "problems" | "output" | "terminal";

export type EditorTabKind =
  | "code"
  | "texture"
  | "converter"
  | "metadata"
  | "blockbench";

export interface EditorTab {
  id: string;
  kind: EditorTabKind;
  title: string;
  relPath?: string;
  dirty?: boolean;
  revealLine?: number;
  blockbenchUrl?: string;
}

export interface OutputLogLine {
  id: number;
  timestamp: string;
  level: "info" | "warn" | "error" | "success";
  message: string;
}

export interface StudioSettings {
  preferredFileManager: string; // "auto" | binary name
  useYaziPicker: boolean;
  autoSanitizeImports: boolean;
  convertImportedImagesToPng: boolean;
  defaultTargetResolution: number | null;
  defaultUvAtlasGuard: UvAtlasGuardMode;
  defaultCompositeGrassTga: boolean;
  defaultConvertPbr: boolean;
  externalImageEditor: string;
}
