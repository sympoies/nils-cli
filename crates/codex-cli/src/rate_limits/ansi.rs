use std::io::{self, IsTerminal};

const CURRENT_PROFILE_FG: &str = "\x1b[38;2;199;146;234m";

pub fn should_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    io::stdout().is_terminal()
}

pub fn format_name_cell(
    raw: &str,
    width: usize,
    is_current: bool,
    color_enabled: Option<bool>,
) -> String {
    let padded = format!("{:<width$}", raw, width = width);

    let enabled = color_enabled.unwrap_or_else(should_color);
    if !enabled || !is_current {
        return padded;
    }

    format!("{CURRENT_PROFILE_FG}{padded}{}", reset())
}

pub fn format_percent_cell(raw: &str, width: usize, color_enabled: Option<bool>) -> String {
    let mut trimmed = raw.to_string();
    let raw_len = trimmed.chars().count();
    if raw_len > width {
        trimmed = trimmed.chars().take(width).collect();
    }
    let padded = format!("{:>width$}", trimmed, width = width);

    let enabled = color_enabled.unwrap_or_else(should_color);
    if !enabled {
        return padded;
    }

    let percent = match extract_percent(raw) {
        Some(value) => value,
        None => return padded,
    };
    let color = match fg_for_percent(percent) {
        Some(value) => value,
        None => return padded,
    };

    format!("{color}{padded}{}", reset())
}

pub fn format_percent_token(raw: &str, color_enabled: Option<bool>) -> String {
    let width = raw.chars().count();
    if width == 0 {
        return String::new();
    }
    format_percent_cell(raw, width, color_enabled)
}

fn extract_percent(raw: &str) -> Option<i32> {
    let mut part = raw.rsplit(':').next()?.to_string();
    part = part
        .trim()
        .trim_end_matches('%')
        .replace(char::is_whitespace, "");
    part.parse::<i32>().ok()
}

fn fg_for_percent(percent: i32) -> Option<String> {
    if percent <= 0 {
        Some(fg_truecolor(99, 119, 119))
    } else if percent >= 80 {
        Some(fg_truecolor(127, 219, 202))
    } else if percent >= 60 {
        Some(fg_truecolor(173, 219, 103))
    } else if percent >= 40 {
        Some(fg_truecolor(236, 196, 141))
    } else if percent >= 20 {
        Some(fg_truecolor(247, 140, 108))
    } else {
        Some(fg_truecolor(240, 113, 120))
    }
}

fn fg_truecolor(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn reset() -> &'static str {
    "\x1b[0m"
}
