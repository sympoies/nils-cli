use crate::defs::block_preview::{self, Block};
use crate::defs::cache;
use crate::defs::index::{AliasDef, DefIndex, FunctionDef};
use crate::util;

pub fn run_env(args: &[String]) -> i32 {
    let query = util::join_args(args);
    let blocks = build_env_blocks();
    run_blocks(&blocks, &query)
}

pub fn run_alias(args: &[String]) -> i32 {
    let query = util::join_args(args);
    if delims_missing() {
        return run_blocks(&[], &query);
    }
    let index = match cache::load_or_build() {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };
    let blocks = build_alias_blocks(&index);
    run_blocks(&blocks, &query)
}

pub fn run_function(args: &[String]) -> i32 {
    let query = util::join_args(args);
    if delims_missing() {
        return run_blocks(&[], &query);
    }
    let index = match cache::load_or_build() {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };
    let blocks = build_function_blocks(&index);
    run_blocks(&blocks, &query)
}

pub fn run_def(args: &[String]) -> i32 {
    let query = util::join_args(args);
    if delims_missing() {
        return run_blocks(&[], &query);
    }
    let index = match cache::load_or_build() {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    let mut blocks = build_env_blocks();
    blocks.extend(build_alias_blocks(&index));
    blocks.extend(build_function_blocks(&index));
    run_blocks(&blocks, &query)
}

fn run_blocks(blocks: &[Block], query: &str) -> i32 {
    let (code, out) = match block_preview::run(blocks, query) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    if code != 0 {
        return code;
    }

    let Some(out) = out else {
        return 0;
    };
    print!("{out}");
    0
}

fn delims_missing() -> bool {
    std::env::var("FZF_DEF_DELIM")
        .unwrap_or_default()
        .is_empty()
        || std::env::var("FZF_DEF_DELIM_END")
            .unwrap_or_default()
            .is_empty()
}

fn build_env_blocks() -> Vec<Block> {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    vars.sort_by(|a, b| a.0.cmp(&b.0));

    vars.into_iter()
        .map(|(name, value)| Block {
            header: format!("🌱 {name}"),
            body: value
                .lines()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect()
}

fn build_alias_blocks(index: &DefIndex) -> Vec<Block> {
    index
        .aliases
        .iter()
        .map(|a| Block {
            header: format!("🔗 {}", a.name),
            body: build_alias_body(a),
        })
        .collect()
}

fn build_function_blocks(index: &DefIndex) -> Vec<Block> {
    index
        .functions
        .iter()
        .map(|f| Block {
            header: format!("🔧 {}", f.name),
            body: build_function_body(f),
        })
        .collect()
}

fn build_alias_body(def: &AliasDef) -> String {
    let mut out = String::new();
    if let Some(doc) = def.doc.as_deref() {
        out.push_str(&docblock_with_separators(doc));
        out.push('\n');
    }
    if !def.value.is_empty() {
        out.push_str(&def.value);
        out.push('\n');
    }
    out.push_str(&format!("(from {})\n", def.source_file));
    out
}

fn build_function_body(def: &FunctionDef) -> String {
    let mut out = String::new();
    if let Some(doc) = def.doc.as_deref() {
        out.push_str(&docblock_with_separators(doc));
        out.push('\n');
    }
    out.push_str(&def.source);
    out.push('\n');
    out.push_str(&format!("(from {})\n", def.source_file));
    out
}

fn docblock_with_separators(doc: &str) -> String {
    let lines: Vec<&str> = doc.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let first = lines[0];
    let prefix = first
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect::<String>();

    let max_len = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let pad = util::env_or_default("FZF_DEF_DOC_SEPARATOR_PAD", "2")
        .parse::<usize>()
        .unwrap_or(2);

    let prefix_len = prefix.chars().count();
    let dash_count = max_len
        .saturating_add(pad)
        .saturating_sub(prefix_len)
        .saturating_sub(2);
    let dashes = "-".repeat(dash_count);

    let mut out = String::new();
    out.push_str(&format!("{prefix}# {dashes}\n"));
    out.push_str(doc);
    if !doc.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("{prefix}# {dashes}\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: tests mutate process env only in scoped guard usage.
            unsafe { std::env::set_var(key, value) };
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => {
                    // SAFETY: tests restore process env only in scoped guard usage.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: tests restore process env only in scoped guard usage.
                    unsafe { std::env::remove_var(self.key) };
                }
            }
        }
    }

    #[test]
    fn docblock_with_separators_respects_indent_and_pad() {
        let _guard = EnvGuard::set("FZF_DEF_DOC_SEPARATOR_PAD", "2");
        let doc = "  # Alpha\n  # Beta";
        let out = docblock_with_separators(doc);
        let lines: Vec<&str> = out.lines().collect();

        assert_eq!(
            lines,
            vec!["  # -------", "  # Alpha", "  # Beta", "  # -------"]
        );
    }

    #[test]
    fn docblock_with_separators_empty_returns_empty() {
        let out = docblock_with_separators("");
        assert_eq!(out, "");
    }

    #[test]
    fn build_alias_body_includes_doc_value_and_footer() {
        let _guard = EnvGuard::set("FZF_DEF_DOC_SEPARATOR_PAD", "2");
        let def = AliasDef {
            name: "ll".to_string(),
            value: "ls -la".to_string(),
            doc: Some("  # Alpha\n  # Beta".to_string()),
            source_file: "/tmp/zshrc".to_string(),
        };

        let body = build_alias_body(&def);
        let lines: Vec<&str> = body.lines().collect();

        assert_eq!(lines[0], "  # -------");
        assert_eq!(lines[3], "  # -------");
        assert_eq!(lines[4], "");
        assert_eq!(lines[5], "ls -la");
        assert_eq!(lines[6], "(from /tmp/zshrc)");
    }

    #[test]
    fn build_function_body_without_doc_renders_footer() {
        let def = FunctionDef {
            name: "hello".to_string(),
            source: "hello() { echo hi }".to_string(),
            doc: None,
            source_file: "/tmp/functions.zsh".to_string(),
        };

        let body = build_function_body(&def);
        let lines: Vec<&str> = body.lines().collect();

        assert_eq!(
            lines,
            vec!["hello() { echo hi }", "(from /tmp/functions.zsh)"]
        );
    }
}
