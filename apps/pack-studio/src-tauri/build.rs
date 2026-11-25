use std::fs;
use std::path::Path;
use std::process::Command;

fn resolve_short_commit_sha() -> String {
    if let Ok(sha) = std::env::var("PACKSTUDIO_COMMIT_SHA") {
        let trimmed = sha.trim();
        if !trimmed.is_empty() {
            return trimmed.chars().take(12).collect();
        }
    }
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        let trimmed = sha.trim();
        if !trimmed.is_empty() {
            return trimmed.chars().take(7).collect();
        }
    }
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        && output.status.success()
    {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sha.is_empty() {
            return sha;
        }
    }
    String::from("dev")
}

fn main() {
    println!("cargo:rerun-if-changed=../dist/index.html");
    println!("cargo:rerun-if-env-changed=PACKSTUDIO_COMMIT_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-changed=../../../.git/HEAD");

    let commit_sha = resolve_short_commit_sha();
    let pkg_version = env!("CARGO_PKG_VERSION");
    println!("cargo:rustc-env=PACKSTUDIO_COMMIT_SHA={commit_sha}");
    println!("cargo:rustc-env=PACKSTUDIO_FULL_VERSION={pkg_version}+{commit_sha}");

    let dist_dir = Path::new("../dist");
    let dist_index = dist_dir.join("index.html");
    if !dist_index.exists() {
        let _ = fs::create_dir_all(dist_dir);
        let _ = fs::write(
            &dist_index,
            "<!doctype html><html><body><p>PackStudio frontend placeholder (run npm run build)</p></body></html>\n",
        );
    }

    tauri_build::build();
}
