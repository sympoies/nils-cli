use crate::backend::process::{map_failure, ProcessRequest, ProcessRunner};
use crate::error::CliError;

const FRONTMOST_APP_SCRIPT: &str = r#"tell application "System Events" to get name of first application process whose frontmost is true"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationTarget {
    App(String),
    BundleId(String),
}

pub fn activate(
    runner: &dyn ProcessRunner,
    target: &ActivationTarget,
    timeout_ms: u64,
) -> Result<(), CliError> {
    let script = match target {
        ActivationTarget::App(app) => {
            format!(
                r#"tell application "{}" to activate"#,
                escape_applescript(app)
            )
        }
        ActivationTarget::BundleId(bundle_id) => {
            format!(
                r#"tell application id "{}" to activate"#,
                escape_applescript(bundle_id)
            )
        }
    };

    run_osascript(runner, "window.activate", script, timeout_ms).map(|_| ())
}

pub fn type_text(
    runner: &dyn ProcessRunner,
    text: &str,
    delay_ms: Option<u64>,
    enter: bool,
    timeout_ms: u64,
) -> Result<(), CliError> {
    let escaped = escape_applescript(text);
    let mut lines = vec![
        "tell application \"System Events\"".to_string(),
        format!("  keystroke \"{escaped}\""),
    ];

    if let Some(delay) = delay_ms {
        lines.push(format!("  delay {}", (delay as f64) / 1000.0));
    }
    if enter {
        lines.push("  key code 36".to_string());
    }

    lines.push("end tell".to_string());
    run_osascript(runner, "input.type", lines.join("\n"), timeout_ms).map(|_| ())
}

pub fn send_hotkey(
    runner: &dyn ProcessRunner,
    mods: &[Modifier],
    key: &str,
    timeout_ms: u64,
) -> Result<(), CliError> {
    if key.trim().is_empty() {
        return Err(CliError::usage("--key cannot be empty"));
    }

    let modifiers = if mods.is_empty() {
        String::new()
    } else {
        let joined = mods
            .iter()
            .map(|modifier| modifier.applescript_token())
            .collect::<Vec<_>>()
            .join(", ");
        format!(" using {{{joined}}}")
    };

    let script = format!(
        "tell application \"System Events\"\n  keystroke \"{}\"{}\nend tell",
        escape_applescript(key),
        modifiers
    );

    run_osascript(runner, "input.hotkey", script, timeout_ms).map(|_| ())
}

pub fn frontmost_app_name(runner: &dyn ProcessRunner, timeout_ms: u64) -> Result<String, CliError> {
    run_osascript(
        runner,
        "wait.app-active",
        FRONTMOST_APP_SCRIPT.to_string(),
        timeout_ms,
    )
    .map(|out| out.trim().to_string())
}

pub fn parse_modifiers(raw: &str) -> Result<Vec<Modifier>, CliError> {
    let mut mods = Vec::new();
    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let modifier = Modifier::parse(token).ok_or_else(|| {
            CliError::usage(format!(
                "invalid modifier `{token}`; expected cmd,ctrl,alt,shift,fn"
            ))
        })?;
        if !mods.contains(&modifier) {
            mods.push(modifier);
        }
    }

    if mods.is_empty() {
        return Err(CliError::usage(
            "--mods cannot be empty; expected cmd,ctrl,alt,shift,fn",
        ));
    }

    Ok(mods)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
    Fn,
}

impl Modifier {
    fn parse(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "cmd" | "command" => Some(Self::Cmd),
            "ctrl" | "control" => Some(Self::Ctrl),
            "alt" | "option" => Some(Self::Alt),
            "shift" => Some(Self::Shift),
            "fn" | "function" => Some(Self::Fn),
            _ => None,
        }
    }

    pub fn canonical(self) -> &'static str {
        match self {
            Self::Cmd => "cmd",
            Self::Ctrl => "ctrl",
            Self::Alt => "alt",
            Self::Shift => "shift",
            Self::Fn => "fn",
        }
    }

    fn applescript_token(self) -> &'static str {
        match self {
            Self::Cmd => "command down",
            Self::Ctrl => "control down",
            Self::Alt => "option down",
            Self::Shift => "shift down",
            Self::Fn => "fn down",
        }
    }
}

fn run_osascript(
    runner: &dyn ProcessRunner,
    operation: &str,
    script: String,
    timeout_ms: u64,
) -> Result<String, CliError> {
    let request = ProcessRequest::new(
        "osascript",
        vec!["-e".to_string(), script],
        timeout_ms.max(1),
    );
    runner
        .run(&request)
        .map(|output| output.stdout)
        .map_err(|failure| map_failure(operation, failure))
}

fn escape_applescript(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{escape_applescript, parse_modifiers, Modifier};

    #[test]
    fn escapes_applescript_string_literals() {
        assert_eq!(escape_applescript("a\\\"b"), "a\\\\\\\"b".to_string());
    }

    #[test]
    fn parses_modifiers_deduped_and_canonicalized() {
        let mods = parse_modifiers("cmd,shift,command").expect("modifiers should parse");
        assert_eq!(mods, vec![Modifier::Cmd, Modifier::Shift]);
        let canonical = mods
            .iter()
            .map(|m| m.canonical().to_string())
            .collect::<Vec<_>>();
        assert_eq!(canonical, vec!["cmd".to_string(), "shift".to_string()]);
    }

    #[test]
    fn rejects_unknown_modifier() {
        let err = parse_modifiers("cmd,nope").expect_err("unknown modifiers should fail");
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("invalid modifier"));
    }
}
