use anyhow::Result;
use std::io::{self, Write};

pub fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let trimmed = input.trim();
    if trimmed == "y" || trimmed == "Y" {
        return Ok(true);
    }

    println!("🚫 Aborted");
    Ok(false)
}
