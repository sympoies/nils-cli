//! CLI-wide operator rules. A document is admitted as a whole, never per-key.
use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use clap::Args;
use nils_common::cli_contract::OutputFormat;
use serde::Serialize;
use serde_json::{Value, json};

use super::Command;

const SCHEMA: &str = "agent-session.operator-rules.v1";
const MAX_BYTES: u64 = 64 * 1024;
static ACTIVE: OnceLock<Mutex<State>> = OnceLock::new();

#[derive(Debug, Args)]
pub struct RulesArgs {
    /// Validate the shared operator rules file and exit with an error if invalid.
    #[arg(long)]
    pub check: bool,
    /// Show effective rules, selected file, and source.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Rules {
    coordinator_requires_root: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            coordinator_requires_root: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Invalid {
    key: String,
    reason: &'static str,
}

fn invalid(key: &str, reason: &'static str) -> Invalid {
    // Unknown keys are input, so never print arbitrary contents or control bytes.
    let key = if !key.is_empty()
        && key.len() <= 64
        && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || matches!(key, "$file" | "$document")
    {
        key
    } else {
        "<non-identifier-key>"
    };
    Invalid {
        key: key.into(),
        reason,
    }
}

fn load(path: Option<&Path>) -> Result<(Rules, bool), Invalid> {
    let Some(path) = path else {
        return Ok((Rules::default(), false));
    };
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Rules::default(), false));
        }
        Err(_) => return Err(invalid("$file", "cannot read file")),
    };
    if !file
        .metadata()
        .map_err(|_| invalid("$file", "cannot inspect file"))?
        .is_file()
    {
        return Err(invalid("$file", "expected a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("$file", "cannot read file"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("$file", "document exceeds 64 KiB"));
    }
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid("$document", "expected valid JSON"))?;
    let root = document
        .as_object()
        .ok_or_else(|| invalid("$document", "expected a JSON object"))?;
    if root.get("schema_version").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(invalid(
            "schema_version",
            "expected agent-session.operator-rules.v1",
        ));
    }
    for key in root.keys() {
        if !matches!(key.as_str(), "schema_version" | "coordinator_requires_root") {
            return Err(invalid(key, "unknown operator rule"));
        }
    }
    let coordinator_requires_root = match root.get("coordinator_requires_root") {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid("coordinator_requires_root", "expected a boolean"))?,
    };
    Ok((
        Rules {
            coordinator_requires_root,
        },
        true,
    ))
}

fn selected_path() -> Option<PathBuf> {
    let nonempty = |key| {
        env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    nonempty("AGENT_SESSION_RULES_FILE").or_else(|| {
        nonempty("XDG_CONFIG_HOME")
            .or_else(|| nils_common::fs::home_dir().map(|home| home.join(".config")))
            .map(|base| base.join("agent-session/rules.json"))
    })
}

struct State {
    path: Option<PathBuf>,
    rules: Rules,
    from_file: bool,
    reload: bool,
    last_error: Option<Invalid>,
}

impl State {
    fn new(reload: bool) -> Self {
        Self {
            path: selected_path(),
            rules: Rules::default(),
            from_file: false,
            reload,
            last_error: None,
        }
    }

    fn refresh(&mut self) -> Result<(), Invalid> {
        // Only a completely valid document replaces the active rules.
        let (rules, from_file) = load(self.path.as_deref())?;
        self.rules = rules;
        self.from_file = from_file;
        self.last_error = None;
        Ok(())
    }

    fn refresh_or_warn(&mut self) {
        if let Err(error) = self.refresh() {
            if self.last_error.as_ref() != Some(&error) {
                let _ = writeln!(
                    std::io::stderr(),
                    "warning: operator rules file {:?}, key {}: {}; keeping {}",
                    self.path.as_deref().unwrap_or(Path::new("<unset>")),
                    error.key,
                    error.reason,
                    if self.from_file {
                        "last good rules"
                    } else {
                        "built-in defaults"
                    }
                );
            }
            self.last_error = Some(error);
        }
    }

    fn view(&self) -> Value {
        json!({
            "rules_schema_version": SCHEMA,
            "file": self.path.as_ref().map(|path| path.to_string_lossy()),
            "source": if self.from_file { "file" } else { "built_in" },
            "retained_last_good": self.from_file && self.last_error.is_some(),
            "rules": self.rules,
        })
    }
}

/// Run before dispatch, including hook commands and serve. Strict checking is
/// shared; serve deployment inputs keep their own separate loader.
pub(crate) fn prepare(command: &Command) -> Option<i32> {
    let mut state = State::new(matches!(command, Command::Serve(_)));
    let checking = match command {
        Command::Rules(args) => Some((args.check, args.format)),
        Command::Serve(args) if args.check => Some((true, args.format)),
        _ => None,
    };
    if let Some((true, format)) = checking {
        if let Err(error) = state.refresh() {
            return Some(crate::render_error(
                "rules",
                format,
                crate::CliError::usage(
                    "operator-rules-invalid",
                    format!(
                        "operator rules file {:?}, key {}: {}",
                        state.path.as_deref().unwrap_or(Path::new("<unset>")),
                        error.key,
                        error.reason
                    ),
                    Some(
                        json!({"file":state.path.as_ref().map(|path| path.to_string_lossy()),"key":error.key}),
                    ),
                ),
            ));
        }
    } else {
        state.refresh_or_warn();
    }
    let result = match command {
        Command::Rules(args) => Some(render(args.format, state.view())),
        Command::Serve(args) if args.check && args.config.is_none() => {
            Some(render(args.format, state.view()))
        }
        _ => None,
    };
    *ACTIVE
        .get_or_init(|| Mutex::new(State::new(false)))
        .lock()
        .unwrap() = state;
    result
}

fn render(format: OutputFormat, view: Value) -> i32 {
    crate::render_single_success("rules", format, &view, |view| {
        format!(
            "operator rules: {}\nfile: {}\ncoordinator_requires_root: {}\nretained_last_good: {}\n",
            view["source"].as_str().unwrap_or_default(),
            view["file"],
            view["rules"]["coordinator_requires_root"],
            view["retained_last_good"],
        )
    })
}

pub(crate) fn require_coordinator_root() -> bool {
    let Some(active) = ACTIVE.get() else {
        return true;
    };
    let mut state = active.lock().unwrap();
    if state.reload {
        state.refresh_or_warn();
    }
    state.rules.coordinator_requires_root
}

/// Inspect the running daemon, including retained rules after a bad reload.
pub(crate) fn routes() -> axum::Router {
    axum::Router::new().route("/operator-rules", axum::routing::get(serve_view))
}

async fn serve_view() -> axum::Json<Value> {
    let active = ACTIVE.get_or_init(|| Mutex::new(State::new(true)));
    let mut state = active.lock().unwrap();
    state.refresh_or_warn();
    axum::Json(json!({"schema_version":"cli.agent-session.rules.v1","ok":true,"data":state.view()}))
}
