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
