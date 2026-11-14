#[path = "history_owner_credentials.rs"]
mod owner_credentials;

use serde_json::Value;

pub(super) fn redact_value(value: &mut Value, key: Option<&str>) {
    if key.is_some_and(sensitive_key) {
        *value = Value::String("[redacted]".to_string());
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                redact_value(value, Some(key));
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_value(value, None);
            }
        }
        Value::String(text) => *text = redact_text(text),
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    credential_key(key) || key_matches(key, &["path", "file", "directory", "cwd"])
}

fn key_matches(key: &str, needles: &[&str]) -> bool {
    let key = key.to_ascii_lowercase();
    let normalized: String = key.chars().filter(char::is_ascii_alphanumeric).collect();
    needles
        .iter()
        .any(|needle| normalized.contains(needle) || key.contains(needle))
}

pub(super) fn appears_path_or_url(value: &str) -> bool {
    let value = path_or_url_candidate(value);
    path_like_fragment(value) || word_contains_path_or_url(value)
}

fn path_or_url_candidate(value: &str) -> &str {
    let value = value.trim();
    let value = value.trim_matches(|character: char| {
        matches!(
            character,
            '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"' | ',' | ';' | ':'
        )
    });
    value
        .rsplit_once('=')
        .or_else(|| {
            value
                .rsplit_once(':')
                .filter(|(prefix, _)| !prefix.contains('/'))
        })
        .map(|(_, value)| value.trim())
        .unwrap_or(value)
}

fn word_contains_path_or_url(value: &str) -> bool {
    value
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || "/\\._-~:".contains(character))
        })
        .filter(|fragment| !fragment.is_empty())
        .any(|fragment| {
            path_like_fragment(fragment)
                || fragment
                    .find('/')
                    .is_some_and(|slash| path_like_fragment(&fragment[slash..]))
        })
}

fn path_like_fragment(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("file:")
        || value.contains("://")
        || value.contains("\\")
        || looks_like_relative_path(value)
}

fn looks_like_relative_path(value: &str) -> bool {
    let value = value.trim_matches(|character: char| {
        matches!(
            character,
            '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"' | ',' | ';' | ':'
        )
    });
    let value = strip_source_location(value);
    let segments: Vec<_> = value.split('/').collect();
    segments.len() > 1
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "._-~".contains(character)
                })
        })
}

fn strip_source_location(mut value: &str) -> &str {
    for _ in 0..2 {
        let Some((prefix, suffix)) = value.rsplit_once(':') else {
            break;
        };
        if !prefix.contains('/')
            || suffix.is_empty()
            || !suffix.bytes().all(|byte| byte.is_ascii_digit())
        {
            break;
        }
        value = prefix;
    }
    value
}

pub(super) fn redact_text(value: &str) -> String {
    let masked = if path_like_fragment(path_or_url_candidate(value)) {
        "[redacted]".to_string()
    } else {
        let mut output = String::with_capacity(value.len());
        for segment in value.split_inclusive(char::is_whitespace) {
            let whitespace = segment.find(char::is_whitespace).unwrap_or(segment.len());
            let (word, spacing) = segment.split_at(whitespace);
            output.push_str(if appears_path_or_url(word) {
                "[redacted]"
            } else {
                word
            });
            output.push_str(spacing);
        }
        output
    };
    owner_text(&masked)
}

pub(super) fn owner_value(value: &mut Value, key: Option<&str>) {
    if key.is_some_and(credential_key) {
        *value = Value::String("[redacted]".to_string());
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                owner_value(value, Some(key));
            }
        }
        Value::Array(values) => {
            for value in values {
                owner_value(value, None);
            }
        }
        Value::String(text) => *text = owner_text(text),
        _ => {}
    }
}

fn credential_key(key: &str) -> bool {
    key_matches(
        key,
        &[
            "token",
            "secret",
            "password",
            "authorization",
            "cookie",
            "credential",
            "privatekey",
            "apikey",
            "accesskey",
            "clientsecret",
        ],
    )
}

// Keep URL spelling and surrounding command text; only credential spans change.
pub(super) fn owner_text(value: &str) -> String {
    let mut rest = value;
    let mut output = String::with_capacity(value.len());
    while let Some(colon) = rest.find(':') {
        let scheme = rest[..colon]
            .rsplit(|character: char| {
                !(character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
            })
            .next()
            .unwrap_or_default();
        let outer_quote = rest[..colon - scheme.len()]
            .chars()
            .next_back()
            .filter(|character| matches!(character, '\'' | '"'));
        let is_scheme = scheme
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
        output.push_str(&rest[..colon + 1]);
        rest = &rest[colon + 1..];
        if !is_scheme {
            continue;
        }
        let end = rest
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '<' | '>' | '`' | '{' | '}')
                    || outer_quote == Some(character)
            })
            .unwrap_or(rest.len());
        let url = &rest[..end];
        if let Some(authority) = url.strip_prefix("//") {
            output.push_str("//");
            output.push_str(&owner_url_tail(authority));
        } else {
            let sanitized = owner_url_query(url);
            // A label or an opaque scheme can precede an authority URL.
            if let Some((prefix, authority)) = sanitized.split_once("://") {
                output.push_str(prefix);
                output.push_str("://");
                output.push_str(&owner_url_tail(authority));
            } else {
                output.push_str(&sanitized);
            }
        }
        rest = &rest[end..];
    }
    output.push_str(rest);
    owner_credentials::redact(&owner_relative_queries(&output))
}

fn owner_relative_queries(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for segment in value.split_inclusive(char::is_whitespace) {
        let whitespace = segment.find(char::is_whitespace).unwrap_or(segment.len());
        let (word, spacing) = segment.split_at(whitespace);
        let path = word.trim_start_matches(['\'', '"', '(', '[', '{']);
        if path.starts_with('/') || path.starts_with("./") || path.starts_with("../") {
            let prefix = &word[..word.len() - path.len()];
            output.push_str(prefix);
            let closing = prefix
                .chars()
                .next_back()
                .and_then(|character| match character {
                    '\'' | '"' => Some(character),
                    '(' => Some(')'),
                    '[' => Some(']'),
                    '{' => Some('}'),
                    _ => None,
                });
            let end = if closing.is_some_and(|character| path.ends_with(character)) {
                path.len() - 1
            } else {
                path.len()
            };
            output.push_str(&owner_url_query(&path[..end]));
            output.push_str(&path[end..]);
        } else {
            output.push_str(word);
        }
        output.push_str(spacing);
    }
    output
}

fn owner_url_tail(value: &str) -> String {
    let authority_end = value.find(['/', '?', '#']).unwrap_or(value.len());
    let (authority, suffix) = value.split_at(authority_end);
    let mut output = match authority.rsplit_once('@') {
        Some((userinfo, host)) => match userinfo.split_once(':') {
            Some((user, _)) => format!("{user}:[redacted]@{host}"),
            None => format!("[redacted]@{host}"),
        },
        None => authority.to_string(),
    };
    output.push_str(&owner_url_query(suffix));
    output
}

fn owner_url_query(value: &str) -> String {
    let mut output = String::new();
    let Some((path, query)) = value.split_once('?') else {
        output.push_str(value);
        return output;
    };
    // A question mark in a fragment does not start a URL query.
    if path.contains('#') {
        output.push_str(value);
        return output;
    }
    output.push_str(path);
    output.push('?');
    let (query, fragment) = query
        .split_once('#')
        .map_or((query, None), |(query, fragment)| (query, Some(fragment)));
    for (index, pair) in query.split('&').enumerate() {
        if index > 0 {
            output.push('&');
        }
        // Some history text percent-encodes the assignment separator too.
        // Inspect it without decoding/reserializing the surrounding URL.
        let assignment = pair.char_indices().find_map(|(index, character)| {
            let width = if character == '=' {
                1
            } else if pair[index..]
                .get(..3)
                .is_some_and(|text| text.eq_ignore_ascii_case("%3d"))
            {
                3
            } else {
                return None;
            };
            Some((&pair[..index], &pair[index..index + width]))
        });
        if let Some((key, separator)) = assignment.filter(|(key, _)| credential_query_key(key)) {
            output.push_str(key);
            output.push_str(separator);
            output.push_str("[redacted]");
        } else {
            output.push_str(pair);
        }
    }
    if let Some(fragment) = fragment {
        output.push('#');
        output.push_str(fragment);
    }
    output
}

fn credential_query_key(key: &str) -> bool {
    // Decode names through the existing URL parser, without reserializing the URL.
    let Ok(url) = reqwest::Url::parse(&format!("https://redaction.invalid/?{key}=")) else {
        return credential_key(key);
    };
    let Some((decoded, _)) = url.query_pairs().next() else {
        return false;
    };
    let normalized: String = decoded
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    credential_key(&decoded)
        || ["key", "auth", "signature"]
            .iter()
            .any(|name| normalized.contains(name))
}
