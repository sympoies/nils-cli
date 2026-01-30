use crate::util;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasDef {
    pub name: String,
    pub value: String,
    pub doc: Option<String>,
    pub source_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub source: String,
    pub doc: Option<String>,
    pub source_file: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefIndex {
    pub aliases: Vec<AliasDef>,
    pub functions: Vec<FunctionDef>,
}

pub fn build_index() -> Result<DefIndex> {
    let root = util::zsh_root()?;
    let files = list_first_party_files(&root)?;
    let mut index = DefIndex::default();

    for file in files {
        let content = match fs::read(&file) {
            Ok(v) => String::from_utf8_lossy(&v).to_string(),
            Err(_) => continue,
        };
        index_file(&file, &content, &mut index)?;
    }

    index.aliases.sort_by(|a, b| a.name.cmp(&b.name));
    index.functions.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(index)
}

fn list_first_party_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();

    let zshrc = root.join(".zshrc");
    if zshrc.is_file() {
        files.push(zshrc);
    }
    let zprofile = root.join(".zprofile");
    if zprofile.is_file() {
        files.push(zprofile);
    }

    for dir in ["scripts", "bootstrap", "tools"] {
        let d = root.join(dir);
        if !d.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&d).follow_links(true) {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("zsh") {
                continue;
            }
            if path.components().any(|c| c.as_os_str() == "plugins") {
                continue;
            }
            files.push(path.to_path_buf());
        }
    }

    files.sort();
    Ok(files)
}

fn index_file(path: &Path, content: &str, out: &mut DefIndex) -> Result<()> {
    let mut comment_buf: Vec<String> = Vec::new();
    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        if is_comment_line(line) {
            comment_buf.push(line.to_string());
            continue;
        }

        if line.trim().is_empty() {
            comment_buf.clear();
            continue;
        }

        if let Some(name) = parse_alias_name(line) {
            let value = parse_alias_value(line).unwrap_or_default();
            out.aliases.push(AliasDef {
                name,
                value,
                doc: if comment_buf.is_empty() {
                    None
                } else {
                    Some(comment_buf.join("\n"))
                },
                source_file: path.to_string_lossy().to_string(),
            });
            comment_buf.clear();
            continue;
        }

        if let Some(name) = parse_function_name(line) {
            let mut source_lines: Vec<String> = vec![line.to_string()];
            let mut depth = brace_delta(line);
            while depth > 0 {
                let Some(next) = lines.next() else {
                    break;
                };
                source_lines.push(next.to_string());
                depth += brace_delta(next);
            }

            out.functions.push(FunctionDef {
                name,
                source: source_lines.join("\n"),
                doc: if comment_buf.is_empty() {
                    None
                } else {
                    Some(comment_buf.join("\n"))
                },
                source_file: path.to_string_lossy().to_string(),
            });
            comment_buf.clear();
            continue;
        }

        comment_buf.clear();
    }

    Ok(())
}

fn is_comment_line(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

fn parse_alias_name(line: &str) -> Option<String> {
    let s = line.trim_start();
    if !s.starts_with("alias ") {
        return None;
    }
    let rest = s.strip_prefix("alias ")?;
    let rest = rest.strip_prefix("-g ").unwrap_or(rest);
    let eq = rest.find('=')?;
    let name = rest[..eq].trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn parse_alias_value(line: &str) -> Option<String> {
    let s = line.trim_start();
    let eq = s.find('=')?;
    let raw = s[eq + 1..].trim();
    Some(strip_surrounding_quotes(raw).to_string())
}

fn strip_surrounding_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"'))
    {
        return &s[1..bytes.len() - 1];
    }
    s
}

fn parse_function_name(line: &str) -> Option<String> {
    let s = line.trim_start();
    if s.starts_with("function ") {
        let rest = s.strip_prefix("function ")?.trim_start();
        let name = rest
            .split(|c: char| c.is_whitespace() || c == '(' || c == '{')
            .next()
            .unwrap_or("");
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    if let Some((name, rest)) = s.split_once("()") {
        let name = name.trim();
        if !name.is_empty() && rest.trim_start().starts_with('{') {
            return Some(name.to_string());
        }
    }

    None
}

fn brace_delta(line: &str) -> i32 {
    let mut delta = 0;
    for ch in line.chars() {
        match ch {
            '{' => delta += 1,
            '}' => delta -= 1,
            _ => {}
        }
    }
    delta
}
