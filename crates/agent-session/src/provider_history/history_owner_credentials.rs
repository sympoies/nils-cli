use super::credential_key;

pub(super) fn redact(value: &str) -> String {
    redact_assignments(&redact_userinfo(value))
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
        let left = value[..position].trim_end_matches(['\'', '"']);
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
        let outer_quote =
            previous.filter(|character| matches!(character, '\'' | '"') && left.len() == position);
        let end = start + value_end(&value[start..], outer_quote);
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
        copied = end;
        scan = end;
    }
    output.push_str(&value[copied..]);
    output
}

fn value_end(value: &str, outer_quote: Option<char>) -> usize {
    let mut quote = None;
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
        if outer_quote == Some(character) {
            return index;
        }
        if let Some(current) = quote {
            if character == current {
                quote = None;
                let next = index + character.len_utf8();
                if value.starts_with(current)
                    && value[next..].chars().next().is_none_or(|next| {
                        next.is_whitespace()
                            || matches!(next, ';' | '|' | '&' | ',' | '}' | ']' | ')')
                    })
                {
                    return next;
                }
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if outer_quote.is_none()
            && (character.is_whitespace() || matches!(character, ';' | '|' | '&'))
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
