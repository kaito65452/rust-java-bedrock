use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, VerifyingKey};
use flate2::read::GzDecoder;
use pstdr_lang::spec::BedrockSpec;
use reqwest::Url;
use reqwest::redirect::Policy as RedirectPolicy;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Archive, EntryType};
use tauri::AppHandle;

use crate::lsp::reload_pstdr_runtime;
use crate::state::StudioState;

/// Fixed GitHub Releases endpoint for the official PSTDR release manifest.
pub const PSTDR_MANIFEST_URL: &str =
    "https://github.com/kaito65452/pstdr-dsl/releases/latest/download/pstdr-release.json";

/// Fixed GitHub Releases endpoint for the detached Ed25519 signature of `pstdr-release.json`.
pub const PSTDR_SIGNATURE_URL: &str =
    "https://github.com/kaito65452/pstdr-dsl/releases/latest/download/pstdr-release.json.sig";

/// Expected identifier of the official PSTDR release signing key.
pub const PSTDR_SIGNING_KEY_ID: &str = "saryu-pstdr-root-2026";

/// Hardcoded 32-byte Ed25519 public key (64 lowercase hex characters) used to verify
/// `pstdr-release.json.sig`. NEVER fetched from the internet.
///
/// Key Fingerprint (`SHA-256`):
/// `0b:fb:5a:7a:03:86:6d:c8:31:f6:f1:a8:95:88:36:a6:25:8f:8b:95:e9:75:d0:d4:82:73:38:d0:4a:f6:a4:92`
pub const PSTDR_SIGNING_PUBKEY_HEX: &str =
    "4970e4b61c17aec73af409e091a4e11589603b4db7d23e2535016aa3741c4e09";

/// Built-in PackStudio desktop application SemVer base version (synchronized with `tauri.conf.json`).
pub const PACKSTUDIO_APP_VERSION: &str = "0.3.1-beta";

/// Build-time short Git commit SHA injected by `build.rs`.
pub const PACKSTUDIO_COMMIT_SHA: &str = match option_env!("PACKSTUDIO_COMMIT_SHA") {
    Some(v) if !v.is_empty() => v,
    _ => "dev",
};

/// Full SemVer 2.0 display version (`0.3.1-beta+<commit>`) injected by `build.rs`.
pub const PACKSTUDIO_DISPLAY_VERSION: &str = match option_env!("PACKSTUDIO_FULL_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => "0.3.1-beta+dev",
};

/// Fixed GitHub API endpoint for checking the latest published PackStudio release.
pub const PACKSTUDIO_RELEASES_API_URL: &str =
    "https://api.github.com/repos/kaito65452/rust-java-bedrock/releases/latest";

/// Built-in compiled `pstdr-lang` version shipped inside PackStudio.
pub const BUILTIN_PSTDR_VERSION: &str = "0.1.0-pre-beta.1";

/// Allowlisted hosts for all PSTDR update manifest and artifact downloads (including redirects).
pub const ALLOWED_DOWNLOAD_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// Required path prefix when downloading directly from `github.com`.
pub const ALLOWED_GITHUB_PATH_PREFIX: &str = "/kaito65452/pstdr-dsl/";

/// Safety cap (128 MiB) on extracted `.tar.gz` bytes to prevent decompression bombs.
const MAX_EXTRACTED_BYTES: u64 = 128 * 1024 * 1024;

/// Safety cap (128 MiB) on single downloaded artifact size.
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;

/// Single artifact entry inside `pstdr-release.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PstdrReleaseArtifact {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// Signed `pstdr-release.json` manifest schema (schema_version = 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PstdrReleaseManifest {
    pub schema_version: u32,
    pub version: String,
    pub released_at: String,
    pub spec_version: String,
    pub key_id: String,
    #[serde(default)]
    pub changelog: Vec<String>,
    pub artifacts: BTreeMap<String, PstdrReleaseArtifact>,
}

/// Persisted metadata for an applied synced runtime inside `pstdr-runtime/active.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivePstdrRuntimeMeta {
    pub version: String,
    pub spec_version: String,
    pub applied_at_unix_secs: u64,
    pub manifest_sha256: String,
    pub key_id: String,
    pub pubkey_fingerprint: String,
    pub has_spec_bundle: bool,
    pub has_binaries: bool,
}

/// Persisted metadata inside a quarantine directory before manual user approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuarantineMeta {
    version: String,
    selected_artifacts: Vec<QuarantinedArtifactEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuarantinedArtifactEntry {
    name: String,
    filename: String,
    url: String,
    expected_sha256: String,
    computed_sha256: String,
    size_bytes: u64,
}

/// Status DTO returned by `pstdr_sync_status`, `pstdr_sync_approve_and_apply`, and `pstdr_sync_revert_builtin`.
#[derive(Debug, Clone, Serialize)]
pub struct PstdrSyncStatusDto {
    pub active_source: String,
    pub active_version: String,
    pub builtin_version: String,
    pub active_spec_version: String,
    pub external_runtime_active: bool,
    pub synced_lsp_available: bool,
    pub pubkey_configured: bool,
    pub key_id: String,
    pub pubkey_fingerprint: String,
    pub runtime_dir: String,
}

/// Artifact offer item displayed in `PstdrUpdateModal`.
#[derive(Debug, Clone, Serialize)]
pub struct PstdrUpdateArtifactDto {
    pub name: String,
    pub url: String,
    pub expected_sha256: String,
    pub size: u64,
}

/// Structured update offer returned by `pstdr_sync_check`.
#[derive(Debug, Clone, Serialize)]
pub struct PstdrUpdateOfferDto {
    pub update_available: bool,
    pub version: String,
    pub current_version: String,
    pub spec_version: String,
    pub released_at: String,
    pub key_id: String,
    pub changelog: Vec<String>,
    pub signature_verified: bool,
    pub pubkey_fingerprint: String,
    pub manifest_sha256: String,
    pub artifacts: Vec<PstdrUpdateArtifactDto>,
}

/// Verified artifact fingerprint item returned after quarantine download.
#[derive(Debug, Clone, Serialize)]
pub struct VerifiedQuarantineArtifactDto {
    pub name: String,
    pub filename: String,
    pub expected_sha256: String,
    pub computed_sha256: String,
    pub size_bytes: u64,
    pub sha256_verified: bool,
}

/// Report returned by `pstdr_sync_download_quarantine`.
#[derive(Debug, Clone, Serialize)]
pub struct QuarantineVerificationReportDto {
    pub version: String,
    pub spec_version: String,
    pub quarantine_dir: String,
    pub signature_verified: bool,
    pub pubkey_fingerprint: String,
    pub manifest_sha256: String,
    pub all_hashes_verified: bool,
    pub artifacts: Vec<VerifiedQuarantineArtifactDto>,
}

// ─── Storage Directories & Permissions ───────────────────────────────────────

/// Returns `<data_local_dir>/pack-studio` (or `.cache/pack-studio`).
#[must_use]
pub fn pack_studio_data_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("pack-studio")
}

/// Returns `~/.local/share/pack-studio/pstdr-runtime`.
#[must_use]
pub fn default_runtime_dir() -> PathBuf {
    pack_studio_data_root().join("pstdr-runtime")
}

/// Returns `~/.local/share/pack-studio/pstdr-quarantine`.
#[must_use]
pub fn default_quarantine_root() -> PathBuf {
    pack_studio_data_root().join("pstdr-quarantine")
}

fn set_unix_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed setting permissions {mode:o} on '{}'", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn ensure_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("failed creating directory '{}'", dir.display()))?;
    set_unix_mode(dir, 0o700)?;
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    let mut file = File::create(path)
        .with_context(|| format!("failed creating file '{}'", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed writing file '{}'", path.display()))?;
    file.sync_all().ok();
    set_unix_mode(path, 0o600)?;
    Ok(())
}

fn sanitize_version_dirname(version: &str) -> Result<String> {
    let trimmed = version.trim().trim_start_matches('v');
    let parsed = Version::parse(trimmed)
        .with_context(|| format!("invalid SemVer version '{version}'"))?;
    let clean = parsed.to_string();
    if clean.is_empty()
        || clean.contains("..")
        || clean.contains('/')
        || clean.contains('\\')
    {
        bail!("unsafe version directory name '{version}'");
    }
    Ok(clean)
}

// ─── Pure-Rust Cryptography, Domain Pinning & Anti-Downgrade ─────────────────

/// Computes lowercase hex SHA-256 digest of `bytes`.
#[must_use]
pub fn compute_sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Formats a 32-byte Ed25519 public key fingerprint (`SHA-256(pubkey_bytes)`) as colon-separated hex.
#[must_use]
pub fn pubkey_fingerprint_from_bytes(pubkey_bytes: &[u8; 32]) -> String {
    let digest = Sha256::digest(pubkey_bytes);
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Decodes a 64-char hex Ed25519 public key into a `VerifyingKey` and raw 32-byte array.
pub fn decode_ed25519_pubkey(pubkey_hex: &str) -> Result<(VerifyingKey, [u8; 32])> {
    let trimmed = pubkey_hex.trim();
    if trimmed.is_empty() || trimmed.contains("PASTE_YOUR") || trimmed.starts_with('<') {
        bail!(
            "PSTDR signing public key is not configured. Paste your 64-char Ed25519 public key hex into PSTDR_SIGNING_PUBKEY_HEX."
        );
    }
    let decoded = hex::decode(trimmed)
        .context("invalid hex encoding in hardcoded Ed25519 public key")?;
    let key_bytes: [u8; 32] = decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("Ed25519 public key must be exactly 32 bytes (64 hex chars)"))?;
    let vk = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| anyhow::anyhow!("invalid Ed25519 public key bytes: {e}"))?;
    Ok((vk, key_bytes))
}

/// Verifies an Ed25519 detached signature (128-char hex) against exact raw manifest bytes.
/// Returns the colon-separated SHA-256 fingerprint of the public key on success.
pub fn verify_manifest_ed25519(
    manifest_raw: &[u8],
    sig_hex: &str,
    pubkey_hex: &str,
) -> Result<String> {
    let (verifying_key, key_bytes) = decode_ed25519_pubkey(pubkey_hex)?;
    let sig_clean = sig_hex.trim();
    let sig_bytes_vec = hex::decode(sig_clean)
        .map_err(|_| anyhow::anyhow!("Ed25519 signature verification failed — update rejected (malformed hex signature)"))?;
    let sig_bytes: [u8; 64] = sig_bytes_vec.try_into().map_err(|_| {
        anyhow::anyhow!("Ed25519 signature verification failed — update rejected (signature must be 64 bytes)")
    })?;
    let signature = Signature::from_bytes(&sig_bytes);
    verifying_key
        .verify_strict(manifest_raw, &signature)
        .map_err(|_| anyhow::anyhow!("Ed25519 signature verification failed — update rejected"))?;
    Ok(pubkey_fingerprint_from_bytes(&key_bytes))
}

/// Verifies that `bytes` match the expected 64-char hex SHA-256 digest.
pub fn verify_sha256_hex(bytes: &[u8], expected_hex: &str) -> Result<String> {
    let expected_norm = expected_hex.trim().to_ascii_lowercase();
    if expected_norm.len() != 64 || !expected_norm.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid SHA-256 digest in manifest: expected 64 hex characters");
    }
    let computed = compute_sha256_hex(bytes);
    if computed != expected_norm {
        bail!(
            "SHA-256 integrity mismatch — update rejected (expected {expected_norm}, computed {computed})"
        );
    }
    Ok(computed)
}

/// Strictly validates a download or redirect URL against HTTPS + pinned GitHub hosts/path.
pub fn validate_pinned_download_url(raw_url: &str) -> Result<Url> {
    let parsed = Url::parse(raw_url.trim())
        .with_context(|| format!("invalid download URL '{raw_url}'"))?;
    if parsed.scheme() != "https" {
        bail!(
            "Blocked non-HTTPS URL '{}': only https:// downloads are permitted",
            parsed
        );
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("Blocked URL containing embedded credentials: '{parsed}'");
    }
    if let Some(port) = parsed.port()
        && port != 443
    {
        bail!("Blocked URL using non-standard HTTPS port {port}: '{parsed}'");
    }
    let Some(host) = parsed.host_str() else {
        bail!("Blocked URL without host: '{parsed}'");
    };
    let host_lower = host.to_ascii_lowercase();
    if !ALLOWED_DOWNLOAD_HOSTS.contains(&host_lower.as_str()) {
        bail!(
            "Blocked untrusted download host '{host_lower}': allowed hosts are {}",
            ALLOWED_DOWNLOAD_HOSTS.join(", ")
        );
    }
    if host_lower == "github.com" {
        let path = parsed.path();
        if !path.starts_with(ALLOWED_GITHUB_PATH_PREFIX)
            || path.contains("..")
            || path.contains('\\')
        {
            bail!(
                "Blocked github.com URL outside pinned repository '{}': '{}'",
                ALLOWED_GITHUB_PATH_PREFIX,
                parsed
            );
        }
    }
    Ok(parsed)
}

/// Parses SemVer versions and enforces strict anti-downgrade (`remote > installed`).
pub fn check_anti_downgrade(installed_version: &str, remote_version: &str) -> Result<(Version, Version)> {
    let installed = Version::parse(installed_version.trim().trim_start_matches('v'))
        .with_context(|| format!("invalid installed SemVer version '{installed_version}'"))?;
    let remote = Version::parse(remote_version.trim().trim_start_matches('v'))
        .with_context(|| format!("invalid remote SemVer version '{remote_version}'"))?;
    if remote <= installed {
        bail!(
            "Anti-downgrade guard rejected release {remote}: installed version {installed} is already up-to-date or newer"
        );
    }
    Ok((installed, remote))
}

/// Parses and validates a signed manifest after signature verification.
pub fn parse_verified_manifest(manifest_raw: &[u8]) -> Result<PstdrReleaseManifest> {
    let manifest: PstdrReleaseManifest = serde_json::from_slice(manifest_raw)
        .context("failed parsing verified pstdr-release.json manifest")?;
    if manifest.schema_version != 1 {
        bail!(
            "Unsupported pstdr-release.json schema_version {} (expected 1)",
            manifest.schema_version
        );
    }
    let _ = sanitize_version_dirname(&manifest.version)?;
    if manifest.artifacts.is_empty() {
        bail!("pstdr-release.json contains no downloadable artifacts");
    }
    Ok(manifest)
}

fn platform_binary_artifact_candidates() -> &'static [&'static str] {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "aarch64") => &[
            "linux_aarch64",
            "pstdr_linux_aarch64",
            "aarch64_linux",
            "aarch64-unknown-linux-gnu",
        ],
        ("linux", "x86_64") => &[
            "linux_x86_64",
            "pstdr_linux_x86_64",
            "x86_64_linux",
            "x86_64-unknown-linux-gnu",
        ],
        ("macos", "aarch64") => &["macos_aarch64", "darwin_aarch64", "aarch64-apple-darwin"],
        ("macos", "x86_64") => &["macos_x86_64", "darwin_x86_64", "x86_64-apple-darwin"],
        ("windows", "x86_64") => &["windows_x86_64", "x86_64-pc-windows-msvc"],
        _ => &[],
    }
}

/// Selects matching artifacts (`spec_bundle` + current OS/arch runtime tarball if available)
/// and validates every selected artifact URL against domain pinning.
pub fn select_matching_artifacts(
    manifest: &PstdrReleaseManifest,
) -> Result<Vec<(String, PstdrReleaseArtifact)>> {
    let mut selected = Vec::new();
    if let Some(spec_art) = manifest.artifacts.get("spec_bundle") {
        validate_pinned_download_url(&spec_art.url)?;
        selected.push((String::from("spec_bundle"), spec_art.clone()));
    }
    for key in platform_binary_artifact_candidates() {
        if let Some(bin_art) = manifest.artifacts.get(*key) {
            validate_pinned_download_url(&bin_art.url)?;
            selected.push(((*key).to_string(), bin_art.clone()));
            break;
        }
    }
    if selected.is_empty() {
        bail!(
            "Release {} contains no compatible artifacts for {}/{}",
            manifest.version,
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
    Ok(selected)
}

// ─── Anti-Zip-Slip `.tar.gz` Extraction ──────────────────────────────────────

/// Validates a tar entry relative path strictly against Zip-Slip, absolute paths,
/// Windows drive prefixes, NUL bytes, and parent directory traversal.
pub fn validate_safe_tar_path(raw_path_bytes: &[u8]) -> Result<PathBuf> {
    if raw_path_bytes.is_empty() {
        bail!("Refusing empty path entry in archive");
    }
    if raw_path_bytes.contains(&0) {
        bail!("Refusing archive entry containing NUL byte");
    }
    let path_str = std::str::from_utf8(raw_path_bytes)
        .context("Refusing non-UTF-8 archive entry path")?;
    if path_str.starts_with('/') || path_str.starts_with('\\') {
        bail!("Refusing absolute archive entry path '{path_str}'");
    }
    if path_str.contains('\\') {
        bail!("Refusing archive entry containing backslash '{path_str}'");
    }
    let bytes = path_str.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        bail!("Refusing archive entry with Windows drive prefix '{path_str}'");
    }

    let mut clean = PathBuf::new();
    for comp in Path::new(path_str).components() {
        match comp {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                bail!("Refusing Zip-Slip parent traversal in archive entry '{path_str}'");
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("Refusing rooted archive entry '{path_str}'");
            }
        }
    }
    if clean.as_os_str().is_empty() {
        bail!("Refusing empty normalized archive entry '{path_str}'");
    }
    Ok(clean)
}

/// Safely extracts a `.tar.gz` archive into `dest_dir`, rejecting symlinks, hardlinks,
/// device nodes, traversal paths, absolute paths, and oversized archives.
pub fn extract_tar_gz_safe(archive_bytes: &[u8], dest_dir: &Path) -> Result<Vec<PathBuf>> {
    ensure_private_dir(dest_dir)?;
    let gz = GzDecoder::new(Cursor::new(archive_bytes));
    let mut archive = Archive::new(gz);
    archive.set_preserve_permissions(false);
    archive.set_preserve_mtime(false);
    archive.set_unpack_xattrs(false);

    let mut extracted_files = Vec::new();
    let mut total_bytes = 0u64;

    for entry_res in archive.entries().context("failed reading tar.gz entries")? {
        let mut entry = entry_res.context("corrupt tar.gz entry")?;
        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::Regular | EntryType::Directory => {}
            EntryType::Symlink => {
                bail!("Refusing symlink entry in archive (security policy)");
            }
            EntryType::Link => {
                bail!("Refusing hardlink entry in archive (security policy)");
            }
            other => {
                bail!("Refusing unsupported archive entry type '{other:?}' (security policy)");
            }
        }

        let raw_bytes = entry.path_bytes();
        // Allow top-level `.` directory entry cleanly
        if entry_type == EntryType::Directory
            && (raw_bytes.as_ref() == b"." || raw_bytes.as_ref() == b"./")
        {
            continue;
        }
        let rel = validate_safe_tar_path(raw_bytes.as_ref())?;
        let out_path = dest_dir.join(&rel);

        if entry_type == EntryType::Directory {
            ensure_private_dir(&out_path)?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            ensure_private_dir(parent)?;
        }
        let mut buf = Vec::new();
        let mut limited = (&mut entry).take(MAX_EXTRACTED_BYTES.saturating_sub(total_bytes) + 1);
        limited
            .read_to_end(&mut buf)
            .with_context(|| format!("failed reading archive entry '{}'", rel.display()))?;
        total_bytes = total_bytes.saturating_add(buf.len() as u64);
        if total_bytes > MAX_EXTRACTED_BYTES {
            bail!(
                "Archive exceeded maximum safe extraction size ({} MiB)",
                MAX_EXTRACTED_BYTES / (1024 * 1024)
            );
        }
        write_private_file(&out_path, &buf)?;
        extracted_files.push(rel);
    }

    Ok(extracted_files)
}

// ─── Active Runtime Discovery Helpers (used by `lsp.rs` & `converter.rs`) ────

fn read_active_runtime_meta(runtime_dir: &Path) -> Option<ActivePstdrRuntimeMeta> {
    let path = runtime_dir.join("active.json");
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn find_spec_json_in_dir(spec_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        spec_dir.join("data/spec/bedrock_api_latest.json"),
        spec_dir.join("spec/bedrock_api_latest.json"),
        spec_dir.join("bedrock_api_latest.json"),
    ];
    candidates.into_iter().find(|c| c.is_file())
}

/// Loads the active synced `BedrockSpec` if installed and valid; otherwise falls back to bundled.
#[must_use]
pub fn load_active_bedrock_spec() -> BedrockSpec {
    let runtime_dir = default_runtime_dir();
    if read_active_runtime_meta(&runtime_dir).is_some()
        && let Some(spec_path) = find_spec_json_in_dir(&runtime_dir.join("spec"))
        && let Ok(spec) = BedrockSpec::load(Some(&spec_path), None)
    {
        return spec;
    }
    BedrockSpec::bundled()
}

/// Returns the path to an active synced `pstdr-lsp` binary if installed.
#[must_use]
pub fn active_synced_lsp_binary() -> Option<PathBuf> {
    let runtime_dir = default_runtime_dir();
    let meta = read_active_runtime_meta(&runtime_dir)?;
    if !meta.has_binaries {
        return None;
    }
    let bin_name = if cfg!(windows) {
        "pstdr-lsp.exe"
    } else {
        "pstdr-lsp"
    };
    let candidate = runtime_dir.join("bin").join(bin_name);
    candidate.is_file().then_some(candidate)
}

/// Builds the current sync status DTO for `runtime_dir` and `pubkey_hex`.
#[must_use]
pub fn build_sync_status(runtime_dir: &Path, pubkey_hex: &str) -> PstdrSyncStatusDto {
    let (pubkey_configured, pubkey_fingerprint) = match decode_ed25519_pubkey(pubkey_hex) {
        Ok((_, bytes)) => (true, pubkey_fingerprint_from_bytes(&bytes)),
        Err(_) => (false, String::from("UNCONFIGURED")),
    };

    let active_spec = load_active_bedrock_spec();
    let active_spec_str = format!(
        "Bedrock {} / {}",
        active_spec.bedrock_version, active_spec.script_api_version
    );

    if let Some(meta) = read_active_runtime_meta(runtime_dir) {
        let lsp_bin = active_synced_lsp_binary().is_some();
        PstdrSyncStatusDto {
            active_source: String::from("synced"),
            active_version: meta.version,
            builtin_version: String::from(BUILTIN_PSTDR_VERSION),
            active_spec_version: meta.spec_version,
            external_runtime_active: true,
            synced_lsp_available: lsp_bin,
            pubkey_configured,
            key_id: String::from(PSTDR_SIGNING_KEY_ID),
            pubkey_fingerprint,
            runtime_dir: runtime_dir.display().to_string(),
        }
    } else {
        PstdrSyncStatusDto {
            active_source: String::from("builtin"),
            active_version: String::from(BUILTIN_PSTDR_VERSION),
            builtin_version: String::from(BUILTIN_PSTDR_VERSION),
            active_spec_version: active_spec_str,
            external_runtime_active: false,
            synced_lsp_available: false,
            pubkey_configured,
            key_id: String::from(PSTDR_SIGNING_KEY_ID),
            pubkey_fingerprint,
            runtime_dir: runtime_dir.display().to_string(),
        }
    }
}

fn current_installed_version(runtime_dir: &Path) -> String {
    read_active_runtime_meta(runtime_dir)
        .map(|m| m.version)
        .unwrap_or_else(|| String::from(BUILTIN_PSTDR_VERSION))
}

// ─── Pinned HTTPS HTTP Client ────────────────────────────────────────────────

fn build_pinned_http_client() -> Result<reqwest::Client> {
    let policy = RedirectPolicy::custom(|attempt| {
        if attempt.previous().len() >= 8 {
            return attempt.error("too many redirects while downloading PSTDR release");
        }
        match validate_pinned_download_url(attempt.url().as_str()) {
            Ok(_) => attempt.follow(),
            Err(err) => attempt.error(err.to_string()),
        }
    });

    reqwest::Client::builder()
        .user_agent("PackStudio-PSTDR-Sync/0.2")
        .redirect(policy)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .context("failed constructing rustls HTTPS client")
}

async fn download_pinned_bytes(client: &reqwest::Client, raw_url: &str) -> Result<Vec<u8>> {
    let url = validate_pinned_download_url(raw_url)?;
    let resp = client
        .get(url.clone())
        .send()
        .await
        .with_context(|| format!("network request failed for '{url}'"))?;

    // Re-check final URL after redirects (defense-in-depth).
    let _ = validate_pinned_download_url(resp.url().as_str())?;

    if !resp.status().is_success() {
        bail!(
            "HTTP {} while fetching '{}'",
            resp.status().as_u16(),
            url
        );
    }

    if let Some(len) = resp.content_length()
        && len > MAX_DOWNLOAD_BYTES
    {
        bail!(
            "Remote file '{}' exceeds maximum allowed size ({} MiB)",
            url,
            MAX_DOWNLOAD_BYTES / (1024 * 1024)
        );
    }

    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("failed reading response body from '{url}'"))?;
    if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
        bail!("Downloaded payload from '{url}' exceeded maximum allowed size");
    }
    Ok(bytes.to_vec())
}

// ─── Core Quarantine & Apply Logic (directly unit-testable) ──────────────────

/// Verifies a manifest + signature + anti-downgrade and writes verified artifacts into quarantine.
/// On any SHA-256 mismatch, deletes the quarantine directory immediately and returns an error.
pub fn write_and_verify_quarantine_core(
    quarantine_root: &Path,
    runtime_dir: &Path,
    manifest_raw: &[u8],
    sig_hex: &str,
    pubkey_hex: &str,
    artifact_payloads: &[(String, PstdrReleaseArtifact, Vec<u8>)],
) -> Result<QuarantineVerificationReportDto> {
    let pubkey_fingerprint = verify_manifest_ed25519(manifest_raw, sig_hex, pubkey_hex)?;
    let manifest = parse_verified_manifest(manifest_raw)?;
    let installed = current_installed_version(runtime_dir);
    let _ = check_anti_downgrade(&installed, &manifest.version)?;
    let clean_ver = sanitize_version_dirname(&manifest.version)?;
    let manifest_sha256 = compute_sha256_hex(manifest_raw);

    let q_dir = quarantine_root.join(&clean_ver);
    if q_dir.exists() {
        let _ = fs::remove_dir_all(&q_dir);
    }
    ensure_private_dir(&q_dir)?;

    let res = (|| -> Result<QuarantineVerificationReportDto> {
        write_private_file(&q_dir.join("pstdr-release.json"), manifest_raw)?;
        write_private_file(&q_dir.join("pstdr-release.json.sig"), sig_hex.trim().as_bytes())?;

        let arts_dir = q_dir.join("artifacts");
        ensure_private_dir(&arts_dir)?;

        let mut meta_entries = Vec::new();
        let mut report_items = Vec::new();

        for (name, art, bytes) in artifact_payloads {
            let _ = validate_pinned_download_url(&art.url)?;
            let computed = verify_sha256_hex(bytes, &art.sha256)?;
            if art.size > 0 && bytes.len() as u64 != art.size {
                bail!(
                    "Artifact '{name}' byte length mismatch (expected {}, got {}) — update rejected",
                    art.size,
                    bytes.len()
                );
            }
            let safe_name: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let filename = format!("{safe_name}.tar.gz");
            write_private_file(&arts_dir.join(&filename), bytes)?;

            meta_entries.push(QuarantinedArtifactEntry {
                name: name.clone(),
                filename: filename.clone(),
                url: art.url.clone(),
                expected_sha256: art.sha256.trim().to_ascii_lowercase(),
                computed_sha256: computed.clone(),
                size_bytes: bytes.len() as u64,
            });
            report_items.push(VerifiedQuarantineArtifactDto {
                name: name.clone(),
                filename,
                expected_sha256: art.sha256.trim().to_ascii_lowercase(),
                computed_sha256: computed,
                size_bytes: bytes.len() as u64,
                sha256_verified: true,
            });
        }

        let meta = QuarantineMeta {
            version: clean_ver.clone(),
            selected_artifacts: meta_entries,
        };
        let meta_bytes = serde_json::to_vec_pretty(&meta)?;
        write_private_file(&q_dir.join("meta.json"), &meta_bytes)?;

        Ok(QuarantineVerificationReportDto {
            version: clean_ver.clone(),
            spec_version: manifest.spec_version,
            quarantine_dir: q_dir.display().to_string(),
            signature_verified: true,
            pubkey_fingerprint,
            manifest_sha256,
            all_hashes_verified: true,
            artifacts: report_items,
        })
    })();

    if res.is_err() {
        let _ = fs::remove_dir_all(&q_dir);
    }
    res
}

/// Re-verifies Ed25519 signature + artifact SHA-256 hashes inside quarantine, extracts with
/// Anti-Zip-Slip protection into a staging directory, atomically swaps into `runtime_dir`,
/// writes `active.json`, and deletes quarantine.
pub fn approve_and_apply_quarantine_core(
    quarantine_root: &Path,
    runtime_dir: &Path,
    version: &str,
    pubkey_hex: &str,
) -> Result<ActivePstdrRuntimeMeta> {
    let clean_ver = sanitize_version_dirname(version)?;
    let q_dir = quarantine_root.join(&clean_ver);
    if !q_dir.is_dir() {
        bail!(
            "Quarantine directory for version '{clean_ver}' does not exist. Download & verify first."
        );
    }

    // Defense-in-depth: re-read and re-verify Ed25519 signature of quarantined manifest.
    let manifest_raw = fs::read(q_dir.join("pstdr-release.json"))
        .context("missing quarantined pstdr-release.json")?;
    let sig_hex = fs::read_to_string(q_dir.join("pstdr-release.json.sig"))
        .context("missing quarantined pstdr-release.json.sig")?;
    let pubkey_fingerprint = verify_manifest_ed25519(&manifest_raw, &sig_hex, pubkey_hex)?;
    let manifest = parse_verified_manifest(&manifest_raw)?;
    let installed = current_installed_version(runtime_dir);
    let _ = check_anti_downgrade(&installed, &manifest.version)?;
    let manifest_sha256 = compute_sha256_hex(&manifest_raw);

    let meta_text = fs::read_to_string(q_dir.join("meta.json"))
        .context("missing quarantined meta.json")?;
    let q_meta: QuarantineMeta =
        serde_json::from_str(&meta_text).context("corrupt quarantined meta.json")?;

    let parent_dir = runtime_dir
        .parent()
        .context("runtime_dir has no parent directory")?;
    ensure_private_dir(parent_dir)?;

    let staging_dir = parent_dir.join(format!(".pstdr-runtime-staging-{}", uuid::Uuid::new_v4()));
    if staging_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir);
    }
    ensure_private_dir(&staging_dir)?;

    let stage_res = (|| -> Result<ActivePstdrRuntimeMeta> {
        let mut has_spec_bundle = false;
        let mut has_binaries = false;

        for entry in &q_meta.selected_artifacts {
            let manifest_art = manifest.artifacts.get(&entry.name).ok_or_else(|| {
                anyhow::anyhow!("Quarantined artifact '{}' not found in signed manifest", entry.name)
            })?;
            let _ = validate_pinned_download_url(&manifest_art.url)?;

            let archive_path = q_dir.join("artifacts").join(&entry.filename);
            let archive_bytes = fs::read(&archive_path)
                .with_context(|| format!("missing quarantined archive '{}'", archive_path.display()))?;
            let _ = verify_sha256_hex(&archive_bytes, &manifest_art.sha256)?;

            if entry.name == "spec_bundle" {
                let spec_dest = staging_dir.join("spec");
                let _ = extract_tar_gz_safe(&archive_bytes, &spec_dest)?;
                has_spec_bundle = true;
            } else {
                let bin_tmp = staging_dir.join("bin_extract");
                let extracted = extract_tar_gz_safe(&archive_bytes, &bin_tmp)?;
                let bin_dest = staging_dir.join("bin");
                ensure_private_dir(&bin_dest)?;
                for rel in extracted {
                    let fname = rel
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    if matches!(fname, "pstdr-c" | "pstdr-lsp" | "pstdr-c.exe" | "pstdr-lsp.exe") {
                        let dst_bin = bin_dest.join(fname);
                        fs::copy(bin_tmp.join(&rel), &dst_bin)?;
                        set_unix_mode(&dst_bin, 0o700)?;
                        has_binaries = true;
                    }
                }
                let _ = fs::remove_dir_all(&bin_tmp);
            }
        }

        let applied_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());

        let active_meta = ActivePstdrRuntimeMeta {
            version: clean_ver.clone(),
            spec_version: manifest.spec_version.clone(),
            applied_at_unix_secs: applied_at,
            manifest_sha256,
            key_id: manifest.key_id.clone(),
            pubkey_fingerprint,
            has_spec_bundle,
            has_binaries,
        };
        write_private_file(
            &staging_dir.join("active.json"),
            &serde_json::to_vec_pretty(&active_meta)?,
        )?;

        // Atomic swap into `runtime_dir` (backup old -> rename staging -> remove backup).
        let backup_dir = parent_dir.join(format!(".pstdr-runtime-backup-{}", uuid::Uuid::new_v4()));
        let had_old = runtime_dir.exists();
        if had_old {
            fs::rename(runtime_dir, &backup_dir)
                .context("failed moving existing pstdr-runtime to backup")?;
        }
        if let Err(err) = fs::rename(&staging_dir, runtime_dir) {
            if had_old {
                let _ = fs::rename(&backup_dir, runtime_dir);
            }
            return Err(err).context("failed swapping staged pstdr-runtime into place");
        }
        if had_old {
            let _ = fs::remove_dir_all(&backup_dir);
        }

        Ok(active_meta)
    })();

    if stage_res.is_err() {
        let _ = fs::remove_dir_all(&staging_dir);
        return stage_res;
    }

    let _ = fs::remove_dir_all(&q_dir);
    stage_res
}

/// Deletes the quarantine folder for `version`.
pub fn reject_quarantine_core(quarantine_root: &Path, version: &str) -> Result<()> {
    let clean_ver = sanitize_version_dirname(version)?;
    let q_dir = quarantine_root.join(&clean_ver);
    if q_dir.exists() {
        fs::remove_dir_all(&q_dir)
            .with_context(|| format!("failed removing quarantine '{}'", q_dir.display()))?;
    }
    Ok(())
}

/// Removes active synced runtime directory, reverting cleanly to the built-in engine.
pub fn revert_to_builtin_core(runtime_dir: &Path) -> Result<()> {
    if runtime_dir.exists() {
        fs::remove_dir_all(runtime_dir)
            .with_context(|| format!("failed removing synced runtime '{}'", runtime_dir.display()))?;
    }
    Ok(())
}

// ─── Tauri Commands ──────────────────────────────────────────────────────────

/// Status DTO returned by `studio_check_update`.
#[derive(Debug, Clone, Serialize)]
pub struct StudioUpdateStatusDto {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
    pub published_at: Option<String>,
    pub release_notes: String,
    pub note: Option<String>,
}

/// Tauri Command: Check GitHub Releases for a newer PackStudio application release (read-only).
#[tauri::command]
pub async fn studio_check_update() -> Result<StudioUpdateStatusDto, String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("PackStudio/{PACKSTUDIO_APP_VERSION}"))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed creating HTTPS client: {e}"))?;

    let mut req = client
        .get(PACKSTUDIO_RELEASES_API_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = crate::github_auth::load_stored_github_token_internal() {
        req = req.bearer_auth(token);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Failed checking PackStudio updates: {e}"))?;

    if resp.status().as_u16() == 404 {
        return Ok(StudioUpdateStatusDto {
            current_version: String::from(PACKSTUDIO_DISPLAY_VERSION),
            latest_version: String::from(PACKSTUDIO_APP_VERSION),
            update_available: false,
            release_url: String::from("https://github.com/kaito65452/rust-java-bedrock/releases"),
            published_at: None,
            release_notes: String::new(),
            note: Some(String::from(
                "No published PackStudio GitHub Release found yet. You are running the latest development build.",
            )),
        });
    }

    if !resp.status().is_success() {
        return Err(format!(
            "GitHub Releases API returned HTTP {} while checking PackStudio updates.",
            resp.status().as_u16()
        ));
    }

    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed reading PackStudio release response: {e}"))?;
    let val: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| format!("Failed parsing PackStudio release JSON: {e}"))?;
    let raw_tag = val
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(PACKSTUDIO_APP_VERSION);
    let latest_clean = raw_tag.trim().trim_start_matches('v').to_string();
    let update_available = check_anti_downgrade(PACKSTUDIO_APP_VERSION, &latest_clean).is_ok();
    let release_url = val
        .get("html_url")
        .and_then(serde_json::Value::as_str)
        .filter(|u| u.starts_with("https://github.com/kaito65452/rust-java-bedrock/"))
        .unwrap_or("https://github.com/kaito65452/rust-java-bedrock/releases")
        .to_string();
    let published_at = val
        .get("published_at")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let release_notes = val
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .chars()
        .take(2000)
        .collect();

    Ok(StudioUpdateStatusDto {
        current_version: String::from(PACKSTUDIO_DISPLAY_VERSION),
        latest_version: latest_clean,
        update_available,
        release_url,
        published_at,
        release_notes,
        note: None,
    })
}

/// Tauri Command: Return current active PSTDR version, spec version, and signing key fingerprint.
#[tauri::command]
pub fn pstdr_sync_status() -> PstdrSyncStatusDto {
    build_sync_status(&default_runtime_dir(), PSTDR_SIGNING_PUBKEY_HEX)
}

/// Tauri Command: Fetch `/releases/latest/download/pstdr-release.json` + `.sig`, verify Ed25519
/// signature with the hardcoded public key, enforce SemVer anti-downgrade, and return offer details.
/// NEVER downloads artifacts, writes runtime files, or executes anything.
#[tauri::command]
pub async fn pstdr_sync_check() -> Result<PstdrUpdateOfferDto, String> {
    let client = build_pinned_http_client().map_err(|e| format!("{e:#}"))?;
    let manifest_raw = download_pinned_bytes(&client, PSTDR_MANIFEST_URL)
        .await
        .map_err(|e| format!("Failed fetching PSTDR release manifest: {e:#}"))?;
    let sig_bytes = download_pinned_bytes(&client, PSTDR_SIGNATURE_URL)
        .await
        .map_err(|e| format!("Failed fetching PSTDR release signature: {e:#}"))?;
    let sig_hex = std::str::from_utf8(&sig_bytes)
        .map_err(|_| String::from("Ed25519 signature verification failed — update rejected (invalid UTF-8 signature)"))?;

    let pubkey_fingerprint =
        verify_manifest_ed25519(&manifest_raw, sig_hex, PSTDR_SIGNING_PUBKEY_HEX)
            .map_err(|e| format!("{e:#}"))?;
    let manifest = parse_verified_manifest(&manifest_raw).map_err(|e| format!("{e:#}"))?;
    let runtime_dir = default_runtime_dir();
    let current_ver = current_installed_version(&runtime_dir);
    let manifest_sha256 = compute_sha256_hex(&manifest_raw);

    let is_newer = check_anti_downgrade(&current_ver, &manifest.version).is_ok();
    let matched = select_matching_artifacts(&manifest).map_err(|e| format!("{e:#}"))?;
    let artifacts = matched
        .into_iter()
        .map(|(name, art)| PstdrUpdateArtifactDto {
            name,
            url: art.url,
            expected_sha256: art.sha256.trim().to_ascii_lowercase(),
            size: art.size,
        })
        .collect();

    Ok(PstdrUpdateOfferDto {
        update_available: is_newer,
        version: manifest.version,
        current_version: current_ver,
        spec_version: manifest.spec_version,
        released_at: manifest.released_at,
        key_id: manifest.key_id,
        changelog: manifest.changelog,
        signature_verified: true,
        pubkey_fingerprint,
        manifest_sha256,
        artifacts,
    })
}

/// Tauri Command: Download verified release artifacts into `0600` quarantine and verify SHA-256.
/// NEVER overwrites active files or executes any binary.
#[tauri::command]
pub async fn pstdr_sync_download_quarantine(
    version: String,
) -> Result<QuarantineVerificationReportDto, String> {
    let clean_expected = sanitize_version_dirname(&version).map_err(|e| format!("{e:#}"))?;
    let client = build_pinned_http_client().map_err(|e| format!("{e:#}"))?;
    let manifest_raw = download_pinned_bytes(&client, PSTDR_MANIFEST_URL)
        .await
        .map_err(|e| format!("Failed fetching PSTDR release manifest: {e:#}"))?;
    let sig_bytes = download_pinned_bytes(&client, PSTDR_SIGNATURE_URL)
        .await
        .map_err(|e| format!("Failed fetching PSTDR release signature: {e:#}"))?;
    let sig_hex = std::str::from_utf8(&sig_bytes)
        .map_err(|_| String::from("Ed25519 signature verification failed — update rejected"))?;

    let _ = verify_manifest_ed25519(&manifest_raw, sig_hex, PSTDR_SIGNING_PUBKEY_HEX)
        .map_err(|e| format!("{e:#}"))?;
    let manifest = parse_verified_manifest(&manifest_raw).map_err(|e| format!("{e:#}"))?;
    let remote_clean = sanitize_version_dirname(&manifest.version).map_err(|e| format!("{e:#}"))?;
    if remote_clean != clean_expected {
        return Err(format!(
            "Requested version '{clean_expected}' does not match latest signed release '{remote_clean}'"
        ));
    }

    let selected = select_matching_artifacts(&manifest).map_err(|e| format!("{e:#}"))?;
    let mut payloads = Vec::with_capacity(selected.len());
    for (name, art) in selected {
        let bytes = download_pinned_bytes(&client, &art.url)
            .await
            .map_err(|e| format!("Failed downloading artifact '{name}': {e:#}"))?;
        payloads.push((name, art, bytes));
    }

    write_and_verify_quarantine_core(
        &default_quarantine_root(),
        &default_runtime_dir(),
        &manifest_raw,
        sig_hex,
        PSTDR_SIGNING_PUBKEY_HEX,
        &payloads,
    )
    .map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Apply a manually approved quarantined update after defense-in-depth re-verification.
#[tauri::command]
pub fn pstdr_sync_approve_and_apply(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
    version: String,
) -> Result<PstdrSyncStatusDto, String> {
    let runtime_dir = default_runtime_dir();
    let _meta = approve_and_apply_quarantine_core(
        &default_quarantine_root(),
        &runtime_dir,
        &version,
        PSTDR_SIGNING_PUBKEY_HEX,
    )
    .map_err(|e| format!("{e:#}"))?;

    reload_pstdr_runtime(&app, &state);
    Ok(build_sync_status(&runtime_dir, PSTDR_SIGNING_PUBKEY_HEX))
}

/// Tauri Command: Reject a quarantined update and delete its quarantine directory cleanly.
#[tauri::command]
pub fn pstdr_sync_reject_quarantine(version: String) -> Result<(), String> {
    reject_quarantine_core(&default_quarantine_root(), &version).map_err(|e| format!("{e:#}"))
}

/// Tauri Command: Remove active synced PSTDR runtime and revert immediately to built-in engine.
#[tauri::command]
pub fn pstdr_sync_revert_builtin(
    app: AppHandle,
    state: tauri::State<'_, StudioState>,
) -> Result<PstdrSyncStatusDto, String> {
    let runtime_dir = default_runtime_dir();
    revert_to_builtin_core(&runtime_dir).map_err(|e| format!("{e:#}"))?;
    reload_pstdr_runtime(&app, &state);
    Ok(build_sync_status(&runtime_dir, PSTDR_SIGNING_PUBKEY_HEX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::{Builder, Header};
    use tempfile::TempDir;

    fn test_keypair() -> (SigningKey, String) {
        let secret: [u8; 32] = [0x42; 32];
        let sk = SigningKey::from_bytes(&secret);
        let pub_hex = hex::encode(sk.verifying_key().to_bytes());
        (sk, pub_hex)
    }

    fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut enc);
            for (path, data) in entries {
                let mut header = Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append_data(&mut header, *path, *data).unwrap();
            }
            builder.finish().unwrap();
        }
        enc.finish().unwrap()
    }

    fn build_raw_tar_gz_with_header(header: &mut Header, data: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut enc);
            builder.append(header, data).unwrap();
            builder.finish().unwrap();
        }
        enc.finish().unwrap()
    }

    fn sample_manifest_bytes(version: &str, spec_sha: &str, spec_size: u64) -> Vec<u8> {
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "version": version,
            "released_at": "2026-11-26T10:00:00Z",
            "spec_version": "Bedrock 26.60 / 2.11.0",
            "key_id": PSTDR_SIGNING_KEY_ID,
            "changelog": ["Added Wave 3A mcfunction checks"],
            "artifacts": {
                "spec_bundle": {
                    "url": format!("https://github.com/kaito65452/pstdr-dsl/releases/download/v{version}/spec-bundle.tar.gz"),
                    "sha256": spec_sha,
                    "size": spec_size
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn hardcoded_production_pubkey_decodes_and_matches_expected_fingerprint() {
        let (_vk, bytes) =
            decode_ed25519_pubkey(PSTDR_SIGNING_PUBKEY_HEX).expect("hardcoded pubkey must decode");
        assert_eq!(
            pubkey_fingerprint_from_bytes(&bytes),
            "0b:fb:5a:7a:03:86:6d:c8:31:f6:f1:a8:95:88:36:a6:25:8f:8b:95:e9:75:d0:d4:82:73:38:d0:4a:f6:a4:92"
        );
    }

    #[test]
    fn verifies_valid_ed25519_manifest_and_rejects_single_byte_tampering() {
        let (sk, pub_hex) = test_keypair();
        let manifest_raw = sample_manifest_bytes("0.2.0", &"a".repeat(64), 123);
        let sig_hex = hex::encode(sk.sign(&manifest_raw).to_bytes());

        let fp = verify_manifest_ed25519(&manifest_raw, &sig_hex, &pub_hex)
            .expect("valid signature must pass");
        assert_eq!(fp.split(':').count(), 32);

        // Flip a single byte in the manifest
        let mut tampered = manifest_raw.clone();
        tampered[10] ^= 0x01;
        let err = verify_manifest_ed25519(&tampered, &sig_hex, &pub_hex)
            .expect_err("1-byte tampered manifest must be rejected");
        assert!(err.to_string().contains("Ed25519 signature verification failed"));
    }

    #[test]
    fn enforces_semver_anti_downgrade_guard() {
        assert!(check_anti_downgrade("0.1.0-alpha.1", "0.2.0").is_ok());
        assert!(check_anti_downgrade("0.2.0", "0.2.0").is_err());
        assert!(check_anti_downgrade("0.3.0", "0.2.9").is_err());
        assert!(check_anti_downgrade("0.2.0", "0.2.0-beta.1").is_err());
    }

    #[test]
    fn enforces_strict_domain_pinning_allowlist() {
        assert!(
            validate_pinned_download_url(
                "https://github.com/kaito65452/pstdr-dsl/releases/latest/download/pstdr-release.json"
            )
            .is_ok()
        );
        assert!(
            validate_pinned_download_url(
                "https://objects.githubusercontent.com/github-production-release-asset-2e65be/spec-bundle.tar.gz"
            )
            .is_ok()
        );
        assert!(
            validate_pinned_download_url(
                "https://release-assets.githubusercontent.com/github-production-release-asset/spec.tar.gz"
            )
            .is_ok()
        );

        // Reject http://
        assert!(
            validate_pinned_download_url(
                "http://github.com/kaito65452/pstdr-dsl/releases/latest/download/pstdr-release.json"
            )
            .is_err()
        );
        // Reject untrusted host
        assert!(validate_pinned_download_url("https://evil.example.com/pstdr-release.json").is_err());
        // Reject another GitHub repo
        assert!(
            validate_pinned_download_url(
                "https://github.com/attacker/pstdr-dsl/releases/latest/download/pstdr-release.json"
            )
            .is_err()
        );
    }

    #[test]
    fn sha256_mismatch_rejects_and_cleans_quarantine_and_valid_roundtrip_applies() {
        let tmp = TempDir::new().unwrap();
        let quarantine_root = tmp.path().join("quarantine");
        let runtime_dir = tmp.path().join("runtime");
        let (sk, pub_hex) = test_keypair();

        let spec_tar = build_tar_gz(&[(
            "data/spec/bedrock_api_latest.json",
            pstdr_lang::spec::BUNDLED_SPEC_JSON.as_bytes(),
        )]);
        let good_sha = compute_sha256_hex(&spec_tar);
        let manifest_raw = sample_manifest_bytes("0.2.0", &good_sha, spec_tar.len() as u64);
        let sig_hex = hex::encode(sk.sign(&manifest_raw).to_bytes());
        let manifest: PstdrReleaseManifest = serde_json::from_slice(&manifest_raw).unwrap();
        let art = manifest.artifacts.get("spec_bundle").unwrap().clone();

        // 1. Corrupt payload -> must fail SHA-256 and delete quarantine folder
        let mut corrupt_tar = spec_tar.clone();
        corrupt_tar[0] ^= 0xff;
        let err = write_and_verify_quarantine_core(
            &quarantine_root,
            &runtime_dir,
            &manifest_raw,
            &sig_hex,
            &pub_hex,
            &[(String::from("spec_bundle"), art.clone(), corrupt_tar)],
        )
        .expect_err("SHA-256 mismatch must fail");
        assert!(err.to_string().contains("SHA-256 integrity mismatch"));
        assert!(!quarantine_root.join("0.2.0").exists());

        // 2. Valid payload -> quarantine succeeds, then approve & apply swaps atomically
        let q_report = write_and_verify_quarantine_core(
            &quarantine_root,
            &runtime_dir,
            &manifest_raw,
            &sig_hex,
            &pub_hex,
            &[(String::from("spec_bundle"), art, spec_tar)],
        )
        .expect("valid quarantine should succeed");
        assert!(q_report.all_hashes_verified);
        assert!(quarantine_root.join("0.2.0/pstdr-release.json").is_file());

        let applied = approve_and_apply_quarantine_core(
            &quarantine_root,
            &runtime_dir,
            "0.2.0",
            &pub_hex,
        )
        .expect("approve & apply should succeed");
        assert_eq!(applied.version, "0.2.0");
        assert!(applied.has_spec_bundle);
        assert!(runtime_dir.join("active.json").is_file());
        assert!(
            runtime_dir
                .join("spec/data/spec/bedrock_api_latest.json")
                .is_file()
        );
        assert!(!quarantine_root.join("0.2.0").exists());

        // 3. Revert removes active runtime cleanly
        revert_to_builtin_core(&runtime_dir).unwrap();
        assert!(!runtime_dir.exists());
    }

    #[test]
    fn anti_zip_slip_rejects_traversal_absolute_and_symlink_entries() {
        let tmp = TempDir::new().unwrap();

        // 1. Parent traversal `../evil`
        let mut h_trav = Header::new_gnu();
        h_trav.set_entry_type(EntryType::Regular);
        h_trav.set_size(4);
        h_trav.set_mode(0o644);
        h_trav.as_old_mut().name[..7].copy_from_slice(b"../evil");
        h_trav.set_cksum();
        let tar_trav = build_raw_tar_gz_with_header(&mut h_trav, b"boom");
        assert!(extract_tar_gz_safe(&tar_trav, &tmp.path().join("out1")).is_err());

        // 2. Absolute path `/etc/passwd`
        let mut h_abs = Header::new_gnu();
        h_abs.set_entry_type(EntryType::Regular);
        h_abs.set_size(4);
        h_abs.set_mode(0o644);
        h_abs.as_old_mut().name[..11].copy_from_slice(b"/etc/passwd");
        h_abs.set_cksum();
        let tar_abs = build_raw_tar_gz_with_header(&mut h_abs, b"root");
        assert!(extract_tar_gz_safe(&tar_abs, &tmp.path().join("out2")).is_err());

        // 3. Symlink entry
        let mut h_sym = Header::new_gnu();
        h_sym.set_entry_type(EntryType::Symlink);
        h_sym.set_size(0);
        h_sym.set_mode(0o777);
        h_sym.as_old_mut().name[..4].copy_from_slice(b"link");
        h_sym.as_old_mut().linkname[..11].copy_from_slice(b"/etc/passwd");
        h_sym.set_cksum();
        let tar_sym = build_raw_tar_gz_with_header(&mut h_sym, b"");
        assert!(extract_tar_gz_safe(&tar_sym, &tmp.path().join("out3")).is_err());
    }
}
