use crate::IoError;

/// Validates and normalizes an archive or relative pack path using forward slashes (`/`).
///
/// Rejects null bytes, leading slashes, Windows drive prefixes, empty paths,
/// and any `..` path traversal components (`ZipSlipDetected`).
pub fn sanitize_archive_path(raw: &str) -> Result<String, IoError> {
    if raw.contains('\0') {
        return Err(IoError::InvalidArchive(String::from(
            "path contains null byte",
        )));
    }

    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(IoError::InvalidArchive(format!(
            "absolute path not allowed in archive: '{raw}'"
        )));
    }

    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(IoError::InvalidArchive(format!(
            "drive-prefixed path not allowed in archive: '{raw}'"
        )));
    }

    let mut clean_parts: Vec<&str> = Vec::new();
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(IoError::ZipSlipDetected {
                raw_path: raw.to_string(),
            });
        }
        if segment.contains(':') {
            return Err(IoError::InvalidArchive(format!(
                "invalid ':' character in path segment '{segment}' of '{raw}'"
            )));
        }
        clean_parts.push(segment);
    }

    if clean_parts.is_empty() {
        return Err(IoError::InvalidArchive(format!(
            "empty normalized archive path from '{raw}'"
        )));
    }

    Ok(clean_parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_valid_paths() {
        assert_eq!(
            sanitize_archive_path(r"assets\minecraft\textures\block\stone.png")
                .expect("should normalize"),
            "assets/minecraft/textures/block/stone.png"
        );
        assert_eq!(
            sanitize_archive_path("./assets//minecraft/./pack.mcmeta").expect("should normalize"),
            "assets/minecraft/pack.mcmeta"
        );
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert!(matches!(
            sanitize_archive_path("../evil.png"),
            Err(IoError::ZipSlipDetected { .. })
        ));
        assert!(matches!(
            sanitize_archive_path("assets/../../evil.png"),
            Err(IoError::ZipSlipDetected { .. })
        ));
        assert!(matches!(
            sanitize_archive_path(r"..\evil.png"),
            Err(IoError::ZipSlipDetected { .. })
        ));
        assert!(matches!(
            sanitize_archive_path("/etc/passwd"),
            Err(IoError::InvalidArchive(_))
        ));
        assert!(matches!(
            sanitize_archive_path(r"C:\evil.png"),
            Err(IoError::InvalidArchive(_))
        ));
        assert!(matches!(
            sanitize_archive_path("foo\0bar.png"),
            Err(IoError::InvalidArchive(_))
        ));
    }
}
