use crate::env as shared_env;
use std::io::{self, IsTerminal};

const CURRENT_PROFILE_FG: &str = "\x1b[38;2;199;146;234m";

pub fn should_color() -> bool {
    if shared_env::no_color_enabled() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};

    #[test]
    fn should_color_respects_no_color_env() {
        let lock = GlobalStateLock::new();
        let _no_color = EnvGuard::set(&lock, "NO_COLOR", "1");
        assert!(!should_color());
    }

    #[test]
    fn format_percent_cell_trims_and_keeps_non_percent_values_plain() {
        assert_eq!(format_percent_cell("5h:94%", 8, Some(false)), "  5h:94%");
        assert_eq!(format_percent_cell("too_long", 3, Some(false)), "too");
        assert_eq!(format_percent_cell("oops", 4, Some(true)), "oops");
    }

    #[test]
    fn format_percent_cell_applies_color_bands() {
        for raw in ["x:0%", "x:80%", "x:60%", "x:40%", "x:20%", "x:19%"] {
            let rendered = format_percent_cell(raw, raw.chars().count(), Some(true));
            assert!(rendered.starts_with("\x1b["));
            assert!(rendered.ends_with("\x1b[0m"));
            assert!(rendered.contains(raw));
        }
    }

    #[test]
    fn format_percent_token_handles_empty_input() {
        assert_eq!(format_percent_token("", Some(true)), "");
    }

    #[test]
    fn format_name_cell_colors_only_current_profile() {
        assert_eq!(
            format_name_cell("work", 15, true, Some(false)),
            "work           "
        );
        assert_eq!(
            format_name_cell("work", 15, false, Some(true)),
            "work           "
        );

        let rendered = format_name_cell("work", 15, true, Some(true));
        assert!(rendered.starts_with("\x1b[38;2;199;146;234m"));
        assert!(rendered.ends_with("\x1b[0m"));
        assert!(rendered.contains("work"));
    }
}
