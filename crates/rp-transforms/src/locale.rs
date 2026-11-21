use std::collections::{BTreeMap, HashMap};

use rp_core::{CoreError, Edition, LocaleTable, MappingTable};

/// Parses a Java Edition `assets/minecraft/lang/*.json` file into `LocaleTable`.
pub fn parse_java_locale_json(json_str: &str) -> Result<LocaleTable, CoreError> {
    let trimmed = json_str.trim_start_matches('\u{feff}');
    let map: BTreeMap<String, String> = serde_json::from_str(trimmed)?;
    let mut entries = HashMap::with_capacity(map.len());
    for (k, v) in map {
        entries.insert(k, v);
    }
    Ok(LocaleTable {
        entries,
        comments: Vec::new(),
    })
}

/// Emits a Java Edition `assets/minecraft/lang/*.json` string with deterministic sorted keys.
pub fn emit_java_locale_json(table: &LocaleTable) -> Result<String, CoreError> {
    let sorted: BTreeMap<&String, &String> = table.entries.iter().collect();
    Ok(serde_json::to_string_pretty(&sorted)?)
}

/// Parses a Bedrock Edition `texts/*.lang` file (`key=value`) into `LocaleTable`.
///
/// Handles UTF-8 BOM, blank lines, `#`/`##` comment lines, inline `\t#` comments,
/// `=` inside values, and `\n` / `\t` / `\\` escape sequences.
pub fn parse_bedrock_lang(lang_str: &str) -> Result<LocaleTable, CoreError> {
    let text = lang_str.strip_prefix('\u{feff}').unwrap_or(lang_str);
    let mut entries = HashMap::new();
    let mut comments = Vec::new();

    for (line_idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            comments.push(trimmed.to_string());
            continue;
        }

        let Some((raw_key, raw_val_with_comment)) = line.split_once('=') else {
            return Err(CoreError::InvalidFormat(format!(
                "invalid .lang line {}: missing '=' separator",
                line_idx + 1
            )));
        };

        let key = raw_key.trim();
        if key.is_empty() {
            return Err(CoreError::InvalidFormat(format!(
                "invalid .lang line {}: empty key",
                line_idx + 1
            )));
        }

        let raw_val = match raw_val_with_comment.find("\t#") {
            Some(idx) => raw_val_with_comment[..idx].trim_end_matches([' ', '\t']),
            None => raw_val_with_comment,
        };

        let unescaped = unescape_lang_value(raw_val);
        entries.insert(key.to_string(), unescaped);
    }

    Ok(LocaleTable { entries, comments })
}

/// Emits a Bedrock Edition `texts/*.lang` string with deterministic sorted keys and escaped values.
#[must_use]
pub fn emit_bedrock_lang(table: &LocaleTable) -> String {
    let mut out = String::new();

    for comment in &table.comments {
        let trimmed = comment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            out.push_str(trimmed);
        } else {
            out.push_str("## ");
            out.push_str(trimmed);
        }
        out.push('\n');
    }

    let sorted: BTreeMap<&String, &String> = table.entries.iter().collect();
    for (k, v) in sorted {
        out.push_str(k);
        out.push('=');
        out.push_str(&escape_lang_value(v));
        out.push('\n');
    }

    out
}

/// Transforms localization keys in `table` for the target Minecraft edition.
#[must_use]
pub fn transform_locale_keys(
    table: &LocaleTable,
    mappings: &MappingTable,
    target: Edition,
) -> LocaleTable {
    let mut mapped_entries = HashMap::with_capacity(table.entries.len());
    for (key, value) in &table.entries {
        let mapped_key = match target {
            Edition::Bedrock => mappings.java_locale_key_to_bedrock(key),
            Edition::Java => mappings.bedrock_locale_key_to_java(key),
        };
        mapped_entries.insert(mapped_key, value.clone());
    }

    LocaleTable {
        entries: mapped_entries,
        comments: table.comments.clone(),
    }
}

/// Normalizes a locale code to Java lowercase convention (`en_US` -> `en_us`).
#[must_use]
pub fn normalize_locale_to_java(code: &str) -> String {
    code.trim().replace('-', "_").to_ascii_lowercase()
}

/// Normalizes a locale code to Bedrock mixed-case convention (`en_us` -> `en_US`).
#[must_use]
pub fn normalize_locale_to_bedrock(code: &str) -> String {
    let clean = code.trim().replace('-', "_");
    let parts: Vec<&str> = clean.split('_').collect();
    if parts.len() == 2 {
        format!(
            "{}_{}",
            parts[0].to_ascii_lowercase(),
            parts[1].to_ascii_uppercase()
        )
    } else {
        clean.to_ascii_lowercase()
    }
}

fn unescape_lang_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek().copied() {
                Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('t') => {
                    chars.next();
                    out.push('\t');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                Some('#') => {
                    chars.next();
                    out.push('#');
                }
                Some('=') => {
                    chars.next();
                    out.push('=');
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn escape_lang_value(val: &str) -> String {
    let mut out = String::with_capacity(val.len());
    for ch in val.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bedrock_lang_edge_cases() {
        let raw = "\u{feff}## Header comment\n\n# Another comment\nkey.plain=Hello World\nkey.equals=a=b=c\nkey.escaped=Line1\\nLine2\\tTabbed\\\\Slash\t# inline comment\nkey.utf8=日本語テスト — 💎\n";
        let table = parse_bedrock_lang(raw).expect("parse .lang");

        assert_eq!(
            table.comments,
            vec!["## Header comment", "# Another comment"]
        );
        assert_eq!(
            table.entries.get("key.plain").map(String::as_str),
            Some("Hello World")
        );
        assert_eq!(
            table.entries.get("key.equals").map(String::as_str),
            Some("a=b=c")
        );
        assert_eq!(
            table.entries.get("key.escaped").map(String::as_str),
            Some("Line1\nLine2\tTabbed\\Slash")
        );
        assert_eq!(
            table.entries.get("key.utf8").map(String::as_str),
            Some("日本語テスト — 💎")
        );

        let emitted = emit_bedrock_lang(&table);
        let reparsed = parse_bedrock_lang(&emitted).expect("reparse");
        assert_eq!(reparsed, table);
    }

    #[test]
    fn rejects_invalid_lang_and_json() {
        assert!(parse_bedrock_lang("missing_equals_separator").is_err());
        assert!(parse_bedrock_lang("=empty_key").is_err());
        assert!(parse_java_locale_json("{not valid json").is_err());
        assert!(parse_java_locale_json(r#"{"k": 123}"#).is_err());
    }

    #[test]
    fn locale_code_normalization() {
        assert_eq!(normalize_locale_to_java("en_US"), "en_us");
        assert_eq!(normalize_locale_to_bedrock("en_us"), "en_US");
        assert_eq!(normalize_locale_to_bedrock("fr_fr"), "fr_FR");
    }
}
