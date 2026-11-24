export type Edition = "java" | "bedrock";
export type PackType =
  | "java_resource_pack"
  | "bedrock_resource_pack"
  | "bedrock_addon";
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
  pack_type: PackType;
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
  include_pstdr_workspace?: boolean;
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
  terminal_emulator: string | null;
}

export interface PackMetadataDto {
  edition: Edition;
  pack_type: PackType;
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
  bp_header_uuid?: string | null;
  bp_module_uuid?: string | null;
  bp_script_module_uuid?: string | null;
  bp_script_entry?: string | null;
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
  pack_type: PackType;
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
  include_dev_sources?: boolean;
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

export interface LspContributionDto {
  server_id: string;
  display_name: string;
  command: string;
  args: string[];
  language_ids: string[];
  builtin: boolean;
}

export interface ExtensionJsonSchemaDto {
  uri: string;
  file_match: string[];
  schema: Record<string, unknown>;
  source_extension_id: string;
}

export interface ExtensionSnippetDto {
  prefix: string;
  body: string[];
  description: string;
  language: string;
  source_extension_id: string;
}

export interface InstalledExtensionDto {
  id: string;
  display_name: string;
  publisher: string;
  version: string;
  description: string;
  enabled: boolean;
  builtin: boolean;
  install_path: string | null;
  has_js_entrypoint: boolean;
  runtime_note: string | null;
  schema_count: number;
  snippet_count: number;
  grammar_languages: string[];
  lsp_contributions: LspContributionDto[];
  raw_manifest_json: string;
}

export interface ExtensionsSnapshotDto {
  extensions_dir: string;
  extensions: InstalledExtensionDto[];
  active_schemas: ExtensionJsonSchemaDto[];
  active_snippets: ExtensionSnippetDto[];
}

export interface LspDiagnosticDto {
  server_id: string;
  rel_path: string;
  line: number;
  column: number;
  end_line: number;
  end_column: number;
  severity: WarningSeverity;
  code?: string | null;
  message: string;
  source: string;
}

export interface LspDiagnosticsEvent {
  server_id: string;
  rel_path: string;
  diagnostics: LspDiagnosticDto[];
}

export interface LspCompletionItemDto {
  label: string;
  detail: string;
  documentation: string;
  insert_text: string;
  kind: string;
}

export interface LspHoverDto {
  contents: string;
  source: string;
}

export interface LspTextEditDto {
  line: number;
  column: number;
  end_line: number;
  end_column: number;
  new_text: string;
}

export interface LspCodeActionDto {
  title: string;
  kind: string;
  diagnostic_code?: string | null;
  is_preferred: boolean;
  edits: LspTextEditDto[];
}

export interface PstdrBuildResultDto {
  success: boolean;
  status: AuditStatus;
  summary: string;
  file_count: number;
  error_count: number;
  warning_count: number;
  diagnostics: LspDiagnosticDto[];
  problems: ConversionWarning[];
  snapshot: ProjectSnapshot | null;
  elapsed_ms: number;
}

export interface LspServerStatusDto {
  server_id: string;
  display_name: string;
  command: string;
  args: string[];
  language_ids: string[];
  builtin: boolean;
  binary_found: boolean;
  running: boolean;
}

export type ActivityView =
  | "explorer"
  | "search"
  | "converter"
  | "metadata"
  | "extensions"
  | "settings";

export type BottomTab = "problems" | "output" | "terminal";

export type EditorTabKind =
  | "code"
  | "texture"
  | "converter"
  | "metadata"
  | "extensions"
  | "blockbench";

export interface EditorTab {
  id: string;
  kind: EditorTabKind;
  title: string;
  relPath?: string;
  dirty?: boolean;
  revealLine?: number;
  blockbenchUrl?: string;
  selectedExtensionId?: string;
}

export interface OutputLogLine {
  id: number;
  timestamp: string;
  level: "info" | "warn" | "error" | "success";
  message: string;
}

export interface StudioSettings {
  preferredFileManager: string; // "auto" | binary name
  autoSanitizeImports: boolean;
  convertImportedImagesToPng: boolean;
  defaultTargetResolution: number | null;
  defaultUvAtlasGuard: UvAtlasGuardMode;
  defaultCompositeGrassTga: boolean;
  defaultConvertPbr: boolean;
  externalImageEditor: string;
  blockbenchUiScale: number;
}
