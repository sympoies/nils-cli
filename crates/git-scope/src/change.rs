#[derive(Debug, Clone)]
pub struct ChangeEntry {
    pub kind: String,
    pub src: String,
    pub dest: Option<String>,
}

impl ChangeEntry {
    pub fn is_rename_or_copy(&self) -> bool {
        is_rename_or_copy(&self.kind)
    }

    pub fn display_path(&self) -> String {
        if self.is_rename_or_copy() {
            match self.dest.as_ref() {
                Some(dest) => format!("{} -> {}", self.src, dest),
                None => self.src.clone(),
            }
        } else {
            self.src.clone()
        }
    }

    pub fn file_path(&self) -> String {
        if self.is_rename_or_copy() {
            self.dest.clone().unwrap_or_else(|| self.src.clone())
        } else {
            self.src.clone()
        }
    }
}

pub fn parse_name_status_lines(lines: &[String]) -> Vec<ChangeEntry> {
    lines
        .iter()
        .filter_map(|line| parse_name_status_line(line.as_str()))
        .collect()
}

pub fn parse_name_status_output(output: &str) -> Vec<ChangeEntry> {
    output.lines().filter_map(parse_name_status_line).collect()
}

pub fn canonical_path(raw: &str) -> String {
    if raw.contains("=>") {
        if raw.contains('{') && raw.contains('}') {
            let (prefix, after_open) = raw.split_once('{').unwrap_or((raw, ""));
            let (inside, suffix) = after_open.split_once('}').unwrap_or((after_open, ""));

            let mut new_part = inside.split("=>").last().unwrap_or(inside).trim();
            if new_part.starts_with(' ') {
                new_part = new_part.trim_start();
            }

            format!("{prefix}{new_part}{suffix}")
        } else {
            let mut new_part = raw.split("=>").last().unwrap_or(raw).trim();
            if new_part.starts_with(' ') {
                new_part = new_part.trim_start();
            }
            new_part.to_string()
        }
    } else {
        raw.to_string()
    }
}

fn parse_name_status_line(line: &str) -> Option<ChangeEntry> {
    let mut parts = line.split('\t');
    let kind = parts.next()?;
    let src = parts.next()?;
    let dest = parts.next();
    Some(ChangeEntry {
        kind: kind.to_string(),
        src: src.to_string(),
        dest: dest.map(|value| value.to_string()),
    })
}

fn is_rename_or_copy(kind: &str) -> bool {
    kind.starts_with('R') || kind.starts_with('C')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rename_change_entry() {
        let entries = parse_name_status_output("R100\told.txt\tnew.txt\n");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert!(entry.is_rename_or_copy());
        assert_eq!(entry.display_path(), "old.txt -> new.txt");
        assert_eq!(entry.file_path(), "new.txt");
    }

    #[test]
    fn canonical_path_brace_syntax() {
        let raw = "src/{old => new}/file.txt";
        assert_eq!(canonical_path(raw), "src/new/file.txt");
    }

    #[test]
    fn parse_modify_change_entry() {
        let entries = parse_name_status_output("M\tfile.txt\n");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert!(!entry.is_rename_or_copy());
        assert_eq!(entry.display_path(), "file.txt");
        assert_eq!(entry.file_path(), "file.txt");
    }
}
