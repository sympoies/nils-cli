use anyhow::{Context, Result};
use std::io::{self, Write};

pub fn confirm(prompt: &str) -> Result<bool> {
    if prompt.is_empty() {
        return Ok(false);
    }

    print!("{prompt}");
    io::stdout().flush().context("flush stdout")?;

    let mut line = String::new();
    io::stdin().read_line(&mut line).context("read stdin")?;
    let answer = line.trim_end_matches(['\n', '\r']);

    if answer == "y" || answer == "Y" {
        return Ok(true);
    }

    println!("🚫 Aborted.");
    Ok(false)
}

pub fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().context("flush stdout")?;
    let mut line = String::new();
    io::stdin().read_line(&mut line).context("read stdin")?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}
