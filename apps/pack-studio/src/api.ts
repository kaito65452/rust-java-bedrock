import { invoke } from "@tauri-apps/api/core";
import type {
  AuditResultDto,
  BinaryAssetData,
  BlockbenchLaunchInfo,
  ConversionResultDto,
  ConverterOptions,
  DetectedFileManagers,
  ExtensionsSnapshotDto,
  ImportResult,
  LspCodeActionDto,
  LspCompletionItemDto,
  LspDiagnosticDto,
  LspHoverDto,
  LspServerStatusDto,
  NewProjectConfig,
  PackageExportConfig,
  PackageExportSummary,
  PackMetadataDto,
  PreflightReportDto,
  ProjectSnapshot,
  PstdrBuildResultDto,
  RenameResult,
  TextureTransformResult,
} from "./types";

export const api = {
  openProject: (path: string) =>
    invoke<ProjectSnapshot>("open_project", { path }),

  createProject: (config: NewProjectConfig) =>
    invoke<ProjectSnapshot>("create_project", { config }),

  fsCreateFile: (relPath: string, initialContent?: string) =>
    invoke<ProjectSnapshot>("fs_create_file", {
      relPath,
      initialContent: initialContent ?? null,
    }),

  fsCreateDir: (relPath: string) =>
    invoke<ProjectSnapshot>("fs_create_dir", { relPath }),

  fsRename: (oldRelPath: string, newName: string, isDir: boolean) =>
    invoke<RenameResult>("fs_rename", { oldRelPath, newName, isDir }),

  fsDelete: (relPath: string) =>
    invoke<ProjectSnapshot>("fs_delete", { relPath }),

  fsImportAssets: (
    targetDir: string,
    diskPaths: string[],
    autoSanitize: boolean,
    convertImagesToPng: boolean,
    overwrite: boolean,
  ) =>
    invoke<ImportResult>("fs_import_assets", {
      targetDir,
      diskPaths,
      autoSanitize,
      convertImagesToPng,
      overwrite,
    }),

  fsReadText: (relPath: string) =>
    invoke<string>("fs_read_text", { relPath }),

  fsWriteText: (relPath: string, content: string) =>
    invoke<ProjectSnapshot>("fs_write_text", { relPath, content }),

  fsReadBinaryBase64: (relPath: string) =>
    invoke<BinaryAssetData>("fs_read_binary_base64", { relPath }),

  fsWriteBinaryBase64: (relPath: string, base64Data: string) =>
    invoke<ProjectSnapshot>("fs_write_binary_base64", { relPath, base64Data }),

  detectFileManagers: () =>
    invoke<DetectedFileManagers>("detect_file_managers"),

  revealInFileManager: (path: string, managerOverride?: string | null) =>
    invoke<void>("reveal_in_file_manager", {
      path,
      managerOverride: managerOverride ?? null,
    }),

  getPackMetadata: () => invoke<PackMetadataDto>("get_pack_metadata"),

  savePackMetadata: (metadata: PackMetadataDto) =>
    invoke<ProjectSnapshot>("save_pack_metadata", { metadata }),

  runPreflight: (
    input: string | null,
    direction: "java_to_bedrock" | "bedrock_to_java",
    options: ConverterOptions,
  ) => invoke<PreflightReportDto>("run_preflight", { input, direction, options }),

  executeConvert: (
    input: string | null,
    output: string,
    direction: "java_to_bedrock" | "bedrock_to_java",
    options: ConverterOptions,
  ) =>
    invoke<ConversionResultDto>("execute_convert", {
      input,
      output,
      direction,
      options,
    }),

  runAudit: (projectPath?: string | null) =>
    invoke<AuditResultDto>("run_audit", { projectPath: projectPath ?? null }),

  packagePack: (
    projectPath: string | null,
    exportConfig: PackageExportConfig,
  ) =>
    invoke<PackageExportSummary>("package_pack", {
      projectPath,
      exportConfig,
    }),

  createBlankTexture: (path: string, width: number, height: number) =>
    invoke<ProjectSnapshot>("create_blank_texture", { path, width, height }),

  downscaleTexture: (path: string, targetSize: number) =>
    invoke<TextureTransformResult>("downscale_texture", { path, targetSize }),

  compositeGrassSide: (
    sidePath: string,
    overlayPath: string | null,
    outTga: string,
  ) =>
    invoke<TextureTransformResult>("composite_grass_side", {
      sidePath,
      overlayPath,
      outTga,
    }),

  ptySpawn: (
    cwd: string | null,
    cols: number,
    rows: number,
    cmdOpt?: string | null,
  ) =>
    invoke<number>("pty_spawn", {
      cwd,
      cols,
      rows,
      cmdOpt: cmdOpt ?? null,
    }),

  ptyWrite: (sessionId: number, data: string) =>
    invoke<void>("pty_write", { sessionId, data }),

  ptyResize: (sessionId: number, cols: number, rows: number) =>
    invoke<void>("pty_resize", { sessionId, cols, rows }),

  ptyKill: (sessionId: number) => invoke<void>("pty_kill", { sessionId }),

  blockbenchStart: () => invoke<BlockbenchLaunchInfo>("blockbench_start"),

  blockbenchOpenAsset: (relPath: string) =>
    invoke<BlockbenchLaunchInfo>("blockbench_open_asset", { relPath }),

  blockbenchNewBedrockGeo: (relPath: string, identifier: string) =>
    invoke<BlockbenchLaunchInfo>("blockbench_new_bedrock_geo", {
      relPath,
      identifier,
    }),

  extList: () => invoke<ExtensionsSnapshotDto>("ext_list"),

  extInstall: (sourcePath: string) =>
    invoke<ExtensionsSnapshotDto>("ext_install", { sourcePath }),

  extSetEnabled: (extId: string, enabled: boolean) =>
    invoke<ExtensionsSnapshotDto>("ext_set_enabled", { extId, enabled }),

  extUninstall: (extId: string) =>
    invoke<ExtensionsSnapshotDto>("ext_uninstall", { extId }),

  lspListServers: () => invoke<LspServerStatusDto[]>("lsp_list_servers"),

  lspRegisterCustomServer: (
    serverId: string,
    displayName: string,
    command: string,
    args: string[],
    languageIds: string[],
  ) =>
    invoke<void>("lsp_register_custom_server", {
      serverId,
      displayName,
      command,
      args,
      languageIds,
    }),

  lspUnregisterCustomServer: (serverId: string) =>
    invoke<void>("lsp_unregister_custom_server", { serverId }),

  lspStartServer: (
    serverId: string,
    command: string,
    args: string[],
    languageIds: string[],
  ) =>
    invoke<void>("lsp_start_server", {
      serverId,
      command,
      args,
      languageIds,
    }),

  lspStopServer: (serverId: string) =>
    invoke<void>("lsp_stop_server", { serverId }),

  lspSyncDocument: (
    relPath: string,
    languageId: string,
    content: string,
    version: number,
    eventKind: "open" | "change" | "save",
  ) =>
    invoke<LspDiagnosticDto[]>("lsp_sync_document", {
      relPath,
      languageId,
      content,
      version,
      eventKind,
    }),

  lspCompletion: (
    relPath: string,
    languageId: string,
    line: number,
    character: number,
    prefix: string,
  ) =>
    invoke<LspCompletionItemDto[]>("lsp_completion", {
      relPath,
      languageId,
      line,
      character,
      prefix,
    }),

  lspHover: (
    relPath: string,
    languageId: string,
    line: number,
    character: number,
    word: string,
  ) =>
    invoke<LspHoverDto | null>("lsp_hover", {
      relPath,
      languageId,
      line,
      character,
      word,
    }),

  lspCodeActions: (
    relPath: string,
    languageId: string,
    startLine: number,
    startColumn: number,
    endLine: number,
    endColumn: number,
  ) =>
    invoke<LspCodeActionDto[]>("lsp_code_actions", {
      relPath,
      languageId,
      startLine,
      startColumn,
      endLine,
      endColumn,
    }),

  pstdrCheckWorkspace: () =>
    invoke<PstdrBuildResultDto>("pstdr_check_workspace"),

  pstdrConvertWorkspace: () =>
    invoke<PstdrBuildResultDto>("pstdr_convert_workspace"),
};
