use super::credential_key;

pub(super) fn redact(value: &str) -> String {
    let value =
        preserve_marked_assignments(value, |value| redact_assignments(&redact_userinfo(value)));
    preserve_marked_assignments(&value, |value| {
        nils_common::redact::redact_text(value)
            .value
            .replace("[REDACTED]", "[redacted]")
    })
}

// Reprocessing already-masked assignments must retain their markers, quotes,
// and Authorization schemes. Use the shared matcher's assignment boundaries.
fn preserve_marked_assignments(value: &str, mask: impl Fn(&str) -> String) -> String {
    const MARKER: &str = "[redacted]";
    let mut output = String::with_capacity(value.len());
    let mut copied = 0;
    let mut quote_scan = 0;
    let mut quote = None;
    let mut escaped = false;
    for captures in nils_common::redact::assignment_secret_regex().captures_iter(value) {
        let assignment = captures.get(0).expect("assignment match");
        let raw = captures.name("value").expect("assignment value");
        let end = if value[raw.start()..].starts_with(MARKER) {
            raw.start() + MARKER.len()
        } else if raw.as_str().trim_matches(['\'', '"']) == MARKER {
            assignment.end()
        } else if captures["key"].eq_ignore_ascii_case("authorization")
            && ["bearer", "basic"]
                .iter()
                .any(|scheme| raw.as_str().eq_ignore_ascii_case(scheme))
        {
            let tail = &value[raw.end()..];
            let trimmed = tail.trim_start();
            if !trimmed.starts_with(MARKER) {
                continue;
            }
            raw.end() + tail.len() - trimmed.len() + MARKER.len()
        } else {
            continue;
        };
        // A quote only terminates an unquoted marker when it closes an outer
        // wrapper. Otherwise it can start another fragment of the secret.
        let closing_quote = value[end..].chars().next().is_some_and(|character| {
            if !matches!(character, '\'' | '"') || raw.as_str().starts_with(['\'', '"']) {
                return false;
            }
            for character in value[quote_scan..raw.start()].chars() {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if quote == Some(character) {
                    quote = None;
                } else if quote.is_none() && matches!(character, '\'' | '"') {
                    quote = Some(character);
                }
            }
            quote_scan = raw.start();
            quote == Some(character)
                && value[end + character.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(|next| next.is_whitespace() || ",;&}])|".contains(next))
        });
        // A marker prefix followed by credential text is still a secret value.
        if value[end..].chars().next().is_some_and(|character| {
            !character.is_whitespace()
                && !",;&}]".contains(character)
                && !closing_quote
                && !(character == '#'
                    && super::credential_query_key(&captures["key"])
                    && value[..raw.start()]
                        .rsplit(char::is_whitespace)
                        .next()
                        .is_some_and(|word| {
                            word.split_once('?').is_some_and(|(path, _)| {
                                word.contains(':')
                                    || super::path_like_fragment(
                                        path.trim_start_matches(['\'', '"', '(', '[', '{']),
                                    )
                            })
                        }))
        }) {
            continue;
        }
        output.push_str(&mask(&value[copied..assignment.start()]));
        output.push_str(&value[assignment.start()..end]);
        copied = end;
    }
    output.push_str(&mask(&value[copied..]));
    output
}

fn credential_name(value: &str) -> bool {
    credential_key(value)
        || matches!(
            value.trim_start_matches('-').to_ascii_lowercase().as_str(),
            "key" | "auth" | "signature"
        )
}

fn redact_assignments(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut copied = 0;
    let mut scan = 0;
    while let Some((offset, separator)) = value[scan..]
        .char_indices()
        .find(|(_, character)| matches!(character, '=' | ':') || character.is_whitespace())
    {
        let position = scan + offset;
        scan = position + separator.len_utf8();
        if separator.is_whitespace()
            && value[..position]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            continue;
        }
        let key_end = value[..position].trim_end();
        let left = key_end.trim_end_matches(['\'', '"']);
        let key = left
            .rsplit(|character: char| {
                !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
            })
            .next()
            .unwrap_or_default();
        if key.is_empty()
            || !credential_name(key)
            || (separator.is_whitespace() && !key.starts_with("--"))
        {
            continue;
        }
        let previous = value[..left.len() - key.len()].chars().next_back();
        if previous.is_some_and(|character| {
            !character.is_whitespace()
                && !matches!(character, '\'' | '"' | '(' | '{' | '[' | ';' | '|' | '&')
        }) {
            continue;
        }
        let mut start = scan;
        while let Some(character) = value[start..]
            .chars()
            .next()
            .filter(|character| character.is_whitespace())
        {
            start += character.len_utf8();
        }
        if separator.is_whitespace() && value[start..].starts_with(['=', ':']) {
            continue;
        }
        let outer_quote = previous
            .filter(|character| matches!(character, '\'' | '"') && left.len() == key_end.len());
        // Authorization values include a scheme and credentials separated by spaces.
        let authorization = key.to_ascii_lowercase().contains("authorization");
        let end = start + value_end(&value[start..], outer_quote, authorization);
        if start == end {
            continue;
        }
        output.push_str(&value[copied..start]);
        let raw = &value[start..end];
        let wrapper = raw.chars().next().filter(|character| {
            outer_quote.is_none()
                && matches!(character, '\'' | '"')
                && raw.len() > 1
                && raw.ends_with(*character)
        });
        if let Some(quote) = wrapper {
            output.push(quote);
        }
        output.push_str("[redacted]");
        if let Some(quote) = wrapper {
            output.push(quote);
        }
        // Concatenation can consume the original outer closing quote. Keep
        // one wrapper at the new value boundary, rather than its secret suffix.
        if let Some(quote) = outer_quote
            && raw.contains(quote)
            && !value[end..].starts_with(quote)
        {
            output.push(quote);
        }
        copied = end;
        scan = end;
    }
    output.push_str(&value[copied..]);
    output
}

fn value_end(value: &str, outer_quote: Option<char>, authorization: bool) -> usize {
    let mut quote = outer_quote;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        let next = index + character.len_utf8();
        let at_boundary = || {
            value[next..].chars().next().is_none_or(|next| {
                next.is_whitespace() || matches!(next, ';' | '|' | '&' | ',' | '}' | ']' | ')')
            })
        };
        if outer_quote == Some(character) && at_boundary() {
            return index;
        }
        if let Some(current) = quote {
            if character == current {
                quote = None;
                if outer_quote.is_none() && value.starts_with(current) && at_boundary() {
                    return next;
                }
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if (character.is_whitespace()
            && (!authorization || matches!(character, '\r' | '\n')))
            || matches!(character, ';' | '|' | '&')
        {
            return index;
        }
    }
    value.len()
}

fn redact_userinfo(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for segment in value.split_inclusive(char::is_whitespace) {
        let whitespace = segment.find(char::is_whitespace).unwrap_or(segment.len());
        let (word, spacing) = segment.split_at(whitespace);
        let candidate = word.trim_start_matches(['\'', '"', '(', '[', '{']);
        let userinfo = candidate.rsplit_once('@').and_then(|(userinfo, host)| {
            let (user, secret) = userinfo.split_once(':')?;
            (!candidate.contains("://")
                && !secret.is_empty()
                && !user.is_empty()
                && user.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "._-%+".contains(character)
                })
                && !userinfo.contains('\\')
                && !host.is_empty())
            .then_some((user, host))
        });
        if let Some((user, host)) = userinfo {
            output.push_str(&word[..word.len() - candidate.len()]);
            output.push_str(user);
            output.push_str(":[redacted]@");
            output.push_str(host);
        } else {
            output.push_str(word);
        }
        output.push_str(spacing);
    }
    output
}
