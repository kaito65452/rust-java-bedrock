use std::borrow::Cow;
use std::fmt::Write as _;

/// Decodes raw Minecraft text bytes (`.json`, `.mcmeta`, `.lang`) into UTF-8 text:
/// - Strips UTF-8 BOM (`EF BB BF`)
/// - Decodes UTF-16 LE (`FF FE`) and UTF-16 BE (`FE FF`) with BOM stripped
/// - Falls back to ISO-8859-1 byte-to-char mapping if invalid UTF-8 is encountered
///   (preserving raw `0xA7` -> `§` formatting codes and other Latin-1 bytes).
///
/// Returns `Cow::Borrowed` without allocating when the input is already valid UTF-8.
#[must_use]
pub fn decode_minecraft_text(bytes: &[u8]) -> Cow<'_, str> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return match std::str::from_utf8(rest) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => Cow::Owned(decode_iso_8859_1(rest)),
        };
    }

    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Cow::Owned(decode_utf16_bytes(rest, true));
    }

    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Cow::Owned(decode_utf16_bytes(rest, false));
    }

    match std::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(decode_iso_8859_1(bytes)),
    }
}

fn decode_iso_8859_1(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        out.push(char::from(b));
    }
    out
}

fn decode_utf16_bytes(bytes: &[u8], little_endian: bool) -> String {
    let units = bytes.chunks_exact(2).map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    let mut out = String::with_capacity(bytes.len() / 2);
    for decoded in char::decode_utf16(units) {
        out.push(decoded.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    out
}

/// Sanitizes decoded Minecraft JSON/JSONC text in a string-position-aware pass:
/// - Strips leading Unicode BOM (`U+FEFF`)
/// - Escapes unescaped control characters inside JSON string literals:
///   - `\r\n` and `\n` -> `\\n`
///   - `\t` -> `\\t`
///   - `\0` -> removed
///   - any other `0x01..=0x1F` -> `\\u00XX`
/// - Preserves valid escape sequences (including `\"` and `\\`) and all Unicode characters
/// - Strips JSONC `//` line comments, `/* ... */` block comments, and stray `\0` outside strings
/// - Removes trailing commas before `}` or `]`
///
/// Returns `Cow::Borrowed` without allocating when the input is already clean JSON.
#[must_use]
pub fn sanitize_minecraft_json(text: &str) -> Cow<'_, str> {
    let trimmed = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut changed = trimmed.len() != text.len();

    let mut out = String::with_capacity(trimmed.len());
    let mut chars = trimmed.chars().peekable();
    let mut in_string = false;
    let mut escape = false;
    let mut pending_comma_byte_idx: Option<usize> = None;

    while let Some(ch) = chars.next() {
        if in_string {
            if escape {
                escape = false;
                if (ch as u32) <= 0x1F {
                    // Previous `\` was followed by a raw control character; escape the backslash
                    // first so the JSON output remains valid, then sanitize the control char.
                    changed = true;
                    out.push('\\');
                    push_sanitized_string_control(&mut out, ch, &mut chars);
                } else {
                    out.push(ch);
                }
                continue;
            }

            match ch {
                '\\' => {
                    escape = true;
                    out.push('\\');
                }
                '"' => {
                    in_string = false;
                    out.push('"');
                }
                c if (c as u32) <= 0x1F => {
                    changed = true;
                    push_sanitized_string_control(&mut out, c, &mut chars);
                }
                c => out.push(c),
            }
            continue;
        }

        // Outside JSON string literals:
        if ch == '\0' {
            changed = true;
            continue;
        }

        // Line comment `// ...`
        if ch == '/' && chars.peek() == Some(&'/') {
            changed = true;
            let _ = chars.next();
            for next_ch in chars.by_ref() {
                if next_ch == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }

        // Block comment `/* ... */`
        if ch == '/' && chars.peek() == Some(&'*') {
            changed = true;
            let _ = chars.next();
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    let _ = chars.next();
                    break;
                }
            }
            continue;
        }

        if ch == '"' {
            pending_comma_byte_idx = None;
            in_string = true;
            out.push('"');
            continue;
        }

        if ch == ',' {
            pending_comma_byte_idx = Some(out.len());
            out.push(',');
            continue;
        }

        if (ch == '}' || ch == ']')
            && let Some(comma_pos) = pending_comma_byte_idx.take()
        {
            out.remove(comma_pos);
            changed = true;
            out.push(ch);
            continue;
        }

        if !ch.is_whitespace() {
            pending_comma_byte_idx = None;
        }

        out.push(ch);
    }

    if !changed {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(out)
    }
}

fn push_sanitized_string_control(
    out: &mut String,
    ch: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    match ch {
        '\0' => {}
        '\n' => out.push_str("\\n"),
        '\r' => {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
                out.push_str("\\n");
            } else {
                let _ = write!(out, "\\u{:04x}", '\r' as u32);
            }
        }
        '\t' => out.push_str("\\t"),
        other => {
            let _ = write!(out, "\\u{:04x}", other as u32);
        }
    }
}

/// Decodes raw Minecraft JSON bytes (`decode_minecraft_text`) and sanitizes control characters,
/// comments, and trailing commas (`sanitize_minecraft_json`).
#[must_use]
pub fn sanitize_minecraft_json_bytes(bytes: &[u8]) -> Cow<'_, str> {
    match decode_minecraft_text(bytes) {
        Cow::Borrowed(s) => sanitize_minecraft_json(s),
        Cow::Owned(s) => Cow::Owned(sanitize_minecraft_json(&s).into_owned()),
    }
}

/// Strips UTF-8/UTF-16 BOMs, decodes ISO-8859-1 fallback, escapes stray control characters
/// in JSON strings, and removes JSONC comments and trailing commas.
///
/// Returns `Cow::Borrowed` without allocating when the input already contains clean UTF-8 JSON.
#[must_use]
pub fn sanitize_jsonc_bytes(bytes: &[u8]) -> Cow<'_, [u8]> {
    let has_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF]);

    if !has_bom
        && let Ok(s) = std::str::from_utf8(bytes)
        && let Cow::Borrowed(_) = sanitize_minecraft_json(s)
    {
        return Cow::Borrowed(bytes);
    }

    Cow::Owned(
        sanitize_minecraft_json_bytes(bytes)
            .into_owned()
            .into_bytes(),
    )
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
    fn decodes_utf8_utf16_boms_and_iso_8859_1_section_sign() {
        // UTF-8 BOM
        let utf8_bom = b"\xEF\xBB\xBFhello \xC2\xA7aWorld";
        assert_eq!(decode_minecraft_text(utf8_bom), "hello §aWorld");

        // UTF-16 LE BOM
        let sample = "{\"description\":\"§b测试\"}";
        let mut utf16_le = vec![0xFF, 0xFE];
        for u in sample.encode_utf16() {
            utf16_le.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_minecraft_text(&utf16_le), sample);

        // UTF-16 BE BOM
        let mut utf16_be = vec![0xFE, 0xFF];
        for u in sample.encode_utf16() {
            utf16_be.extend_from_slice(&u.to_be_bytes());
        }
        assert_eq!(decode_minecraft_text(&utf16_be), sample);

        // Invalid UTF-8 raw 0xA7 (§) ISO-8859-1 fallback
        let latin1 = b"item.sword.name=\xA7bDiamond \xA76Sword";
        assert_eq!(
            decode_minecraft_text(latin1),
            "item.sword.name=§bDiamond §6Sword"
        );
    }

    #[test]
    fn escapes_multiline_newlines_tabs_nuls_and_control_chars_in_json_strings() {
        let raw = b"{\n  \"pack\": {\n    \"pack_format\": 15,\n    \"description\": \"Line 1\r\nLine 2\tTab\x00Hidden\x07Bell\\\"\x0BVert\nLine 3\",\n  },\n}";
        let clean = sanitize_minecraft_json_bytes(raw);
        let val: serde_json::Value =
            serde_json::from_str(&clean).expect("sanitized multiline control-char json");
        assert_eq!(
            val["pack"]["description"],
            "Line 1\nLine 2\tTabHidden\u{0007}Bell\"\u{000B}Vert\nLine 3"
        );
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
