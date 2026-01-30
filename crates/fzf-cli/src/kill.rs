use crate::confirm;
use anyhow::{Context, Result};
use std::process::{Command, Stdio};

pub struct KillFlags {
    pub kill_now: bool,
    pub force_kill: bool,
    pub rest: Vec<String>,
}

pub fn parse_kill_flags(args: &[String]) -> KillFlags {
    let mut kill_now = false;
    let mut force_kill = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-k" | "--kill" => {
                kill_now = true;
                i += 1;
            }
            "-9" | "--force" => {
                force_kill = true;
                i += 1;
            }
            _ => break,
        }
    }
    KillFlags {
        kill_now,
        force_kill,
        rest: args[i..].to_vec(),
    }
}

pub fn kill_flow(pids: &[String], kill_now: bool, force_kill: bool) -> Result<i32> {
    let pids: Vec<String> = pids
        .iter()
        .filter_map(|p| {
            let trimmed = p.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect();

    if pids.is_empty() {
        return Ok(0);
    }

    let pid_list = pids.join(" ");

    if kill_now {
        if force_kill {
            println!("☠️  Killing PID(s) with SIGKILL: {pid_list}");
            run_kill(&pids, true)?;
        } else {
            println!("☠️  Killing PID(s) with SIGTERM: {pid_list}");
            run_kill(&pids, false)?;
        }
        return Ok(0);
    }

    if !confirm::confirm(&format!("Kill PID(s): {pid_list}? [y/N] "))? {
        return Ok(1);
    }

    let force = confirm::read_line("Force SIGKILL (-9)? [y/N] ")?;
    if force == "y" || force == "Y" {
        println!("☠️  Killing PID(s) with SIGKILL: {pid_list}");
        run_kill(&pids, true)?;
    } else {
        println!("☠️  Killing PID(s) with SIGTERM: {pid_list}");
        run_kill(&pids, false)?;
    }

    Ok(0)
}

fn run_kill(pids: &[String], force: bool) -> Result<()> {
    let mut cmd = Command::new("kill");
    if force {
        cmd.arg("-9");
    }
    for pid in pids {
        cmd.arg(pid);
    }
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("spawn kill")?;
    if !status.success() {
        anyhow::bail!("kill failed");
    }
    Ok(())
}
