use std::fs;
use std::path::Path;

use toml_edit::{Document, Item, Value};

pub(crate) const PREFERENCE: &str = "hide_rate_limit_model_nudge";
pub(crate) const KEY: &str = "notice.hide_rate_limit_model_nudge";
pub(crate) const OVERRIDE: &str = "notice.hide_rate_limit_model_nudge=true";
pub(crate) const CONFIG: &[u8] = b"[notice]\nhide_rate_limit_model_nudge = true\n";

pub(super) fn set(value: &str) -> i32 {
    if value != "true" {
        eprintln!("codex-cli config: {KEY} requires true");
        return 64;
    }
    let path = crate::runtime::original_codex_home().join("config.toml");
    match persist(&path) {
        Ok(()) => {
            println!("saved {KEY} to {}", path.display());
            0
        }
        Err(message) => {
            eprintln!("codex-cli config: {message}");
            1
        }
    }
}

fn persist(path: &Path) -> Result<(), String> {
    let mut content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    if enable_notice(&mut content)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?
    {
        super::write_config(path, content.as_bytes())?;
    }
    Ok(())
}

fn enable_notice(content: &mut String) -> Result<bool, String> {
    // Edit the original text using parser spans. Re-rendering the whole document
    // can group interleaved dotted keys and normalize line endings.
    let document = Document::parse(content.as_str()).map_err(|error| error.message().to_owned())?;
    let notice = document.get("notice");
    if let Some(item) = notice {
        let table = item
            .as_table_like()
            .ok_or_else(|| "notice must be a table".to_string())?;
        if let Some(setting) = table.get(PREFERENCE) {
            match setting.as_bool() {
                Some(true) => return Ok(false),
                Some(false) => {
                    let span = setting.span().expect("parsed boolean span");
                    content.replace_range(span, "true");
                    return Ok(true);
                }
                None => return Err(format!("{KEY} must be a boolean")),
            }
        }
    }

    match notice {
        Some(Item::Table(table)) if table.is_dotted() || !table.is_implicit() => {
            let end = table
                .get_values()
                .into_iter()
                .filter_map(|(_, value)| value.span().map(|span| span.end))
                .max()
                .unwrap_or_else(|| table.span().expect("parsed notice table span").end);
            let key = if table.is_dotted() { KEY } else { PREFERENCE };
            insert_after_line(content, end, &format!("{key} = true"));
        }
        Some(Item::Value(Value::InlineTable(table))) => {
            let start = table.span().expect("parsed inline table span").start;
            let separator = if table.is_empty() { " " } else { "," };
            content.insert_str(start + 1, &format!(" {PREFERENCE} = true{separator}"));
        }
        _ => {
            let newline = line_ending(content);
            let section = format!("[notice]{newline}{PREFERENCE} = true");
            if content.is_empty() {
                content.push_str(&format!("{section}{newline}"));
            } else if content.ends_with('\n') {
                content.push_str(&format!("{newline}{section}{newline}"));
            } else {
                content.push_str(&format!("{newline}{newline}{section}"));
            }
        }
    }
    Ok(true)
}

fn insert_after_line(content: &mut String, after: usize, assignment: &str) {
    if let Some(offset) = content[after..].find('\n') {
        let start = after + offset + 1;
        let newline = if content[..start].ends_with("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        content.insert_str(start, &format!("{assignment}{newline}"));
    } else {
        content.push_str(&format!("{}{assignment}", line_ending(content)));
    }
}

fn line_ending(content: &str) -> &'static str {
    match content.split_once('\n') {
        Some((first, _)) if first.ends_with('\r') => "\r\n",
        _ => "\n",
    }
}
