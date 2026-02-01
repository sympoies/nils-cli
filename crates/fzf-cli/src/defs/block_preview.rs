use crate::{fzf, util};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

pub struct Block {
    pub header: String,
    pub body: String,
}

pub fn run(blocks: &[Block], default_query: &str) -> Result<(i32, Option<String>)> {
    let delim = std::env::var("FZF_DEF_DELIM").unwrap_or_default();
    let enddelim = std::env::var("FZF_DEF_DELIM_END").unwrap_or_default();
    if delim.is_empty() || enddelim.is_empty() {
        println!("❌ Error: FZF_DEF_DELIM or FZF_DEF_DELIM_END is not set.");
        println!("💡 Please export FZF_DEF_DELIM and FZF_DEF_DELIM_END before running.");
        return Ok((1, None));
    }

    let mut header_to_block: HashMap<String, String> = HashMap::new();
    let mut tmp = tempfile::NamedTempFile::new().context("mktemp")?;
    for b in blocks {
        let _ = writeln!(tmp, "{delim}");
        let _ = writeln!(tmp, "{}", b.header);
        if !b.body.is_empty() {
            let _ = writeln!(tmp, "{}", b.body);
        }
        let _ = writeln!(tmp, "{enddelim}\n");

        let mut rendered = String::new();
        rendered.push_str(&b.header);
        rendered.push('\n');
        rendered.push('\n');
        rendered.push_str(&b.body);
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        header_to_block.insert(b.header.clone(), rendered);
    }
    let _ = tmp.flush();

    let mut preview = tempfile::NamedTempFile::new().context("mktemp preview script")?;
    preview
        .write_all(
            br#"#!/usr/bin/env -S awk -f
BEGIN {
  target      = ENVIRON["FZF_PREVIEW_TARGET"]
  start_delim = ENVIRON["FZF_DEF_DELIM"]
  end_delim   = ENVIRON["FZF_DEF_DELIM_END"]
  printing    = 0
}
{
  if ($0 == start_delim) {
    getline header
    if (header == target) {
      print header
      print ""
      printing = 1
      next
    }
  }
  if (printing && $0 == end_delim) exit
  if (printing) print
}
"#,
        )
        .context("write preview script")?;
    let _ = preview.flush();

    let _ = std::fs::set_permissions(preview.path(), std::fs::Permissions::from_mode(0o755));

    let input = blocks
        .iter()
        .map(|b| b.header.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let args_vec: Vec<String> = vec![
        "--ansi".to_string(),
        "--reverse".to_string(),
        "--height=50%".to_string(),
        "--prompt".to_string(),
        "» Select > ".to_string(),
        "--query".to_string(),
        default_query.to_string(),
        "--preview-window=right:70%:wrap".to_string(),
        "--preview".to_string(),
        format!(
            "FZF_PREVIEW_TARGET={{}} {} {}",
            preview.path().to_string_lossy(),
            tmp.path().to_string_lossy()
        ),
    ];

    let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();
    let (code, selected) = fzf::run_lines(
        &format!("{input}\n"),
        &args_ref,
        &[("FZF_DEF_DELIM", &delim), ("FZF_DEF_DELIM_END", &enddelim)],
    )?;

    if code != 0 {
        return Ok((0, None));
    }

    let Some(sel) = selected.first().cloned() else {
        return Ok((0, None));
    };

    let out = match header_to_block.get(&sel) {
        Some(v) => v.clone(),
        None => return Ok((0, None)),
    };

    set_clipboard_best_effort(&out);
    Ok((0, Some(out)))
}

fn set_clipboard_best_effort(text: &str) {
    if util::cmd_exists("pbcopy") {
        let _ = pipe_to_command("pbcopy", &[], text);
        return;
    }
    if util::cmd_exists("wl-copy") {
        let _ = pipe_to_command("wl-copy", &[], text);
        return;
    }
    if util::cmd_exists("xclip") {
        let _ = pipe_to_command("xclip", &["-selection", "clipboard"], text);
    }
}

fn pipe_to_command(cmd: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {cmd}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn write_exe(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        fs::write(&path, content).expect("write stub");
        let mut perms = fs::metadata(&path).expect("meta").permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        fs::set_permissions(&path, perms).expect("chmod");
    }

    fn fzf_stub_script() -> &'static str {
        r#"#!/bin/bash
set -euo pipefail

dir="${FZF_STUB_OUT_DIR:?FZF_STUB_OUT_DIR is required}"
counter="$dir/.counter"
n=1
if [[ -f "$counter" ]]; then
  n=$(( $(/bin/cat "$counter") + 1 ))
fi
echo "$n" > "$counter"

out="$dir/$n.out"
code_file="$dir/$n.code"
if [[ -f "$out" ]]; then
  /bin/cat "$out"
fi

if [[ -f "$code_file" ]]; then
  exit "$(/bin/cat "$code_file")"
fi
exit 0
"#
    }

    #[test]
    fn run_requires_delimiters() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("FZF_DEF_DELIM", "");
        let _guard_end = EnvGuard::set("FZF_DEF_DELIM_END", "");

        let (code, out) = run(&[], "").expect("run");
        assert_eq!(code, 1);
        assert!(out.is_none());
    }

    #[test]
    fn run_renders_and_copies_selected_block() {
        let _lock = ENV_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let stub = temp.path().join("stub");
        fs::create_dir_all(&stub).unwrap();

        let out_dir = temp.path().join("fzf-out");
        fs::create_dir_all(&out_dir).unwrap();
        fs::write(out_dir.join("1.out"), "Header A\n").unwrap();

        let clipboard = temp.path().join("clipboard.txt");
        write_exe(&stub, "fzf", fzf_stub_script());
        write_exe(
            &stub,
            "pbcopy",
            r#"#!/bin/bash
set -euo pipefail
cat > "${PBCOPY_OUT:?}"
"#,
        );

        let path_s = format!("{}:{}", stub.display(), std::env::var("PATH").unwrap());
        let _guard_path = EnvGuard::set("PATH", &path_s);
        let _guard_out = EnvGuard::set("FZF_STUB_OUT_DIR", out_dir.to_string_lossy().as_ref());
        let _guard_delim = EnvGuard::set("FZF_DEF_DELIM", "---");
        let _guard_end = EnvGuard::set("FZF_DEF_DELIM_END", "+++");
        let _guard_clip = EnvGuard::set("PBCOPY_OUT", clipboard.to_string_lossy().as_ref());

        let blocks = vec![Block {
            header: "Header A".to_string(),
            body: "line1\nline2".to_string(),
        }];
        let (code, out) = run(&blocks, "").expect("run");
        assert_eq!(code, 0);
        let out = out.expect("output");
        assert!(out.contains("Header A"));
        assert!(out.contains("line1"));
        let clipboard_out = fs::read_to_string(&clipboard).unwrap();
        assert_eq!(clipboard_out, out);
    }
}
