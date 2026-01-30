use anyhow::{Context, Result};
use std::process::{Command, Stdio};

pub fn run_lines(input: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<(i32, Vec<String>)> {
    let mut cmd = Command::new("fzf");
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().context("spawn fzf")?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(input.as_bytes())
            .context("write fzf stdin")?;
    }

    let output = child.wait_with_output().context("wait fzf")?;
    let code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let lines = stdout.lines().map(|s| s.to_string()).collect();
    Ok((code, lines))
}

pub fn run_print_query(
    input: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<(i32, Option<String>, Option<String>)> {
    let (code, lines) = run_lines(input, args, envs)?;
    let query = lines.first().cloned();
    let selected = lines.get(1).cloned();
    Ok((code, query, selected))
}

pub fn run_expect(
    input: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<(i32, Option<String>, Vec<String>)> {
    let (code, lines) = run_lines(input, args, envs)?;
    let key = lines.first().cloned();
    let rest = lines.into_iter().skip(1).collect::<Vec<_>>();
    Ok((code, key, rest))
}
