use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// GitHub REST API endpoint for validating the authenticated user's profile.
const GITHUB_API_USER_URL: &str = "https://api.github.com/user";

/// Public GitHub profile metadata cached locally (token is NEVER exposed to the frontend).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubUserProfile {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub html_url: String,
}

/// Private on-disk structure stored with `0600` permissions under `<config_dir>/pack-studio/github-auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredGithubAuth {
    token: String,
    profile: GithubUserProfile,
}

/// Safe DTO returned to the frontend by `github_auth_status`, `github_connect_pat`, and `github_disconnect`.
/// Intentionally omits the raw personal access token.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GithubAuthStatusDto {
    pub connected: bool,
    pub token_stored: bool,
    pub login: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub html_url: Option<String>,
    pub storage_path: String,
}

#[derive(Debug, Deserialize)]
struct GithubApiUserResponse {
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

/// Returns `<config_dir>/pack-studio/github-auth.json` (or `.config/pack-studio/github-auth.json`).
#[must_use]
pub fn default_github_auth_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("pack-studio")
        .join("github-auth.json")
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

/// Parses GitHub `/user` JSON payload into `GithubUserProfile`.
pub fn parse_github_user_json(raw_bytes: &[u8]) -> Result<GithubUserProfile> {
    let parsed: GithubApiUserResponse =
        serde_json::from_slice(raw_bytes).context("failed parsing GitHub /user JSON response")?;
    let login = parsed.login.trim().to_string();
    if login.is_empty() {
        bail!("GitHub /user response missing required 'login' field");
    }
    let name = parsed
        .name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let avatar_url = parsed
        .avatar_url
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("https://"));
    let html_url = parsed
        .html_url
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("https://"))
        .unwrap_or_else(|| format!("https://github.com/{login}"));

    Ok(GithubUserProfile {
        login,
        name,
        avatar_url,
        html_url,
    })
}

/// Saves the GitHub token and profile to `path` with `0700` dir / `0600` file permissions on Unix.
pub fn save_github_auth_to_path(
    path: &Path,
    token: &str,
    profile: &GithubUserProfile,
) -> Result<()> {
    let clean_token = token.trim();
    if clean_token.is_empty() {
        bail!("GitHub token must not be empty");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating config directory '{}'", parent.display()))?;
        set_unix_mode(parent, 0o700)?;
    }
    let stored = StoredGithubAuth {
        token: clean_token.to_string(),
        profile: profile.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&stored)?;
    let mut file = File::create(path)
        .with_context(|| format!("failed creating GitHub auth file '{}'", path.display()))?;
    file.write_all(&bytes)?;
    file.sync_all().ok();
    set_unix_mode(path, 0o600)?;
    Ok(())
}

/// Loads safe `GithubAuthStatusDto` (never exposing the raw token) from `path`.
#[must_use]
pub fn load_github_auth_status_from_path(path: &Path) -> GithubAuthStatusDto {
    let storage_path = path.display().to_string();
    let Ok(text) = fs::read_to_string(path) else {
        return GithubAuthStatusDto {
            connected: false,
            token_stored: false,
            login: None,
            name: None,
            avatar_url: None,
            html_url: None,
            storage_path,
        };
    };
    let Ok(stored) = serde_json::from_str::<StoredGithubAuth>(&text) else {
        return GithubAuthStatusDto {
            connected: false,
            token_stored: false,
            login: None,
            name: None,
            avatar_url: None,
            html_url: None,
            storage_path,
        };
    };
    let has_token = !stored.token.trim().is_empty();
    GithubAuthStatusDto {
        connected: has_token && !stored.profile.login.trim().is_empty(),
        token_stored: has_token,
        login: Some(stored.profile.login),
        name: stored.profile.name,
        avatar_url: stored.profile.avatar_url,
        html_url: Some(stored.profile.html_url),
        storage_path,
    }
}

/// Loads the stored GitHub token for internal authenticated API requests (e.g. Studio update check).
/// Never exposed to the frontend.
#[must_use]
pub fn load_stored_github_token_internal() -> Option<String> {
    let text = fs::read_to_string(default_github_auth_path()).ok()?;
    let stored = serde_json::from_str::<StoredGithubAuth>(&text).ok()?;
    let clean = stored.token.trim().to_string();
    (!clean.is_empty()).then_some(clean)
}

/// Removes the stored GitHub token and profile from `path`.
pub fn disconnect_github_auth_at_path(path: &Path) -> Result<GithubAuthStatusDto> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed removing GitHub auth file '{}'", path.display()))?;
    }
    Ok(load_github_auth_status_from_path(path))
}

/// Tauri Command: Return current GitHub connection status & cached profile (never returns raw token).
#[tauri::command]
pub fn github_auth_status() -> GithubAuthStatusDto {
    load_github_auth_status_from_path(&default_github_auth_path())
}

/// Tauri Command: Validate a GitHub Personal Access Token via `https://api.github.com/user`,
/// persist it with `0600` permissions, and return the public profile (`login`, `name`, `avatar_url`, `html_url`).
#[tauri::command]
pub async fn github_connect_pat(token: String) -> Result<GithubAuthStatusDto, String> {
    let clean_token = token.trim().to_string();
    if clean_token.is_empty() {
        return Err(String::from("Please paste a valid GitHub Personal Access Token."));
    }

    let client = reqwest::Client::builder()
        .user_agent("PackStudio")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed creating HTTPS client: {e}"))?;

    let resp = client
        .get(GITHUB_API_USER_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .bearer_auth(&clean_token)
        .send()
        .await
        .map_err(|e| format!("Network request to GitHub API failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 {
            return Err(String::from(
                "GitHub rejected this token (HTTP 401 Unauthorized). Check that the PAT is valid and has not expired.",
            ));
        }
        return Err(format!(
            "GitHub API returned HTTP {} while verifying token.",
            status.as_u16()
        ));
    }

    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed reading GitHub profile response: {e}"))?;
    let profile = parse_github_user_json(&body).map_err(|e| format!("{e:#}"))?;
    let auth_path = default_github_auth_path();
    save_github_auth_to_path(&auth_path, &clean_token, &profile)
        .map_err(|e| format!("{e:#}"))?;

    Ok(load_github_auth_status_from_path(&auth_path))
}

/// Tauri Command: Clear stored GitHub token and cached profile.
#[tauri::command]
pub fn github_disconnect() -> Result<GithubAuthStatusDto, String> {
    disconnect_github_auth_at_path(&default_github_auth_path()).map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parses_github_user_profile_and_persists_without_exposing_raw_token() {
        let sample = br#"{
            "login": "kaito65452",
            "name": "Saryu",
            "avatar_url": "https://avatars.githubusercontent.com/u/12345?v=4",
            "html_url": "https://github.com/kaito65452"
        }"#;
        let profile = parse_github_user_json(sample).expect("parse profile");
        assert_eq!(profile.login, "kaito65452");
        assert_eq!(profile.name.as_deref(), Some("Saryu"));
        assert_eq!(
            profile.avatar_url.as_deref(),
            Some("https://avatars.githubusercontent.com/u/12345?v=4")
        );

        let tmp = TempDir::new().unwrap();
        let auth_path = tmp.path().join("pack-studio/github-auth.json");
        let secret_token = "ghp_secret_unit_test_token_1234567890";
        save_github_auth_to_path(&auth_path, secret_token, &profile).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&auth_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "github-auth.json must have 0600 permissions");
        }

        let status = load_github_auth_status_from_path(&auth_path);
        assert!(status.connected);
        assert!(status.token_stored);
        assert_eq!(status.login.as_deref(), Some("kaito65452"));
        let serialized = serde_json::to_string(&status).unwrap();
        assert!(
            !serialized.contains(secret_token),
            "raw token must never appear in serialized GithubAuthStatusDto"
        );

        let after_disconnect = disconnect_github_auth_at_path(&auth_path).unwrap();
        assert!(!after_disconnect.connected);
        assert!(!after_disconnect.token_stored);
        assert!(!auth_path.exists());
    }

    #[test]
    fn rejects_non_https_avatar_and_missing_login() {
        let bad_avatar = br#"{"login":"octocat","avatar_url":"http://evil.example/a.png"}"#;
        let profile = parse_github_user_json(bad_avatar).unwrap();
        assert_eq!(profile.avatar_url, None);
        assert_eq!(profile.html_url, "https://github.com/octocat");

        let empty_login = br#"{"login":"   "}"#;
        assert!(parse_github_user_json(empty_login).is_err());
    }
}
