use std::borrow::Cow;

/// Strips UTF-8 BOM, JSONC comments (`//`, `/* */`), and trailing commas outside string literals.
///
/// Returns `Cow::Borrowed` without allocating when the input already contains clean JSON.
#[must_use]
pub fn sanitize_jsonc_bytes(bytes: &[u8]) -> Cow<'_, [u8]> {
    let trimmed = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let has_bom = trimmed.len() != bytes.len();

    let mut out = Vec::with_capacity(trimmed.len());
    let mut changed = has_bom;

    let mut i = 0usize;
    let mut in_string = false;
    let mut escape = false;
    let mut pending_comma_idx: Option<usize> = None;

    while i < trimmed.len() {
        let b = trimmed[i];

        if in_string {
            out.push(b);
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        // Line comment `// ...`
        if b == b'/' && trimmed.get(i + 1) == Some(&b'/') {
            changed = true;
            i += 2;
            while i < trimmed.len() && trimmed[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment `/* ... */`
        if b == b'/' && trimmed.get(i + 1) == Some(&b'*') {
            changed = true;
            i += 2;
            while i + 1 < trimmed.len() && !(trimmed[i] == b'*' && trimmed[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(trimmed.len());
            continue;
        }

        if b == b'"' {
            pending_comma_idx = None;
            in_string = true;
            out.push(b);
            i += 1;
            continue;
        }

        if b == b',' {
            pending_comma_idx = Some(out.len());
            out.push(b);
            i += 1;
            continue;
        }

        if (b == b'}' || b == b']')
            && let Some(comma_pos) = pending_comma_idx.take()
        {
            out.remove(comma_pos);
            changed = true;
            out.push(b);
            i += 1;
            continue;
        }

        if !b.is_ascii_whitespace() {
            pending_comma_idx = None;
        }

        out.push(b);
        i += 1;
    }

    if !changed {
        Cow::Borrowed(bytes)
    } else {
        Cow::Owned(out)
    }
}

/// Normalizes a relative asset path into a valid Java Edition `ResourceLocation` path
/// (`[a-z0-9/._-]`), lowercasing uppercase ASCII and replacing invalid characters with `_`.
#[must_use]
pub fn sanitize_java_resource_path(id: &str) -> String {
    let clean = id.trim().replace('\\', "/");
    let mut out = String::with_capacity(clean.len());
    for ch in clean.chars() {
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_lowercase() || lc.is_ascii_digit() || matches!(lc, '/' | '.' | '_' | '-') {
            out.push(lc);
        } else {
            out.push('_');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bom_comments_and_trailing_commas_while_preserving_strings() {
        let input = b"\xEF\xBB\xBF{\n  // line comment\n  \"url\": \"https://example.com/*not_comment*/\",\n  /* block \n comment */ \"arr\": [1, 2, /* c */ ],\n}\n";
        let sanitized = sanitize_jsonc_bytes(input);
        let parsed: serde_json::Value =
            serde_json::from_slice(&sanitized).expect("valid sanitized json");
        assert_eq!(parsed["url"], "https://example.com/*not_comment*/");
        assert_eq!(parsed["arr"], serde_json::json!([1, 2]));
    }

    #[test]
    fn clean_json_is_borrowed_zero_copy() {
        let input = br#"{"pack":{"pack_format":15,"description":"Clean"}}"#;
        let out = sanitize_jsonc_bytes(input);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn sanitizes_java_resource_location_paths() {
        assert_eq!(
            sanitize_java_resource_path("models/armor/Kitsune Mask (v2)"),
            "models/armor/kitsune_mask__v2_"
        );
        assert_eq!(
            sanitize_java_resource_path("block/stone_01-alt.png"),
            "block/stone_01-alt.png"
        );
    }
}
