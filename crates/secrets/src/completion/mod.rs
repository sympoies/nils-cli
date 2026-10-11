use clap::CommandFactory;
use clap_complete::CompleteEnv;
use clap_complete::engine::CompletionCandidate;
use clap_complete::env::{Bash, EnvCompleter, Zsh};
use std::io;

use crate::store;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn run(shell: crate::cli::CompletionShell) -> i32 {
    match shell {
        crate::cli::CompletionShell::Bash => emit_registration(&Bash),
        crate::cli::CompletionShell::Zsh => emit_registration(&Zsh),
    }
}

/// Emit a `clap_complete` `CompleteEnv` dynamic-completion registration stub for
/// the given shell.
///
/// secrets is a `completion_engine=dynamic` CLI (see the completion coverage
/// matrix): the `name` argument's candidates are the live store entry names,
/// enumerated at TAB time by the binary itself, so the exported script is a thin
/// registration that calls back into `secrets` rather than a static
/// `generate()` script. This remains a single completion path per the completion
/// development standard.
fn emit_registration<C: EnvCompleter>(completer: &C) -> i32 {
    match completer.write_registration(
        "COMPLETE",
        "secrets",
        "secrets",
        "secrets",
        &mut io::stdout(),
    ) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("secrets: error: failed to emit completion registration: {err}");
            1
        }
    }
}

/// Intercept `COMPLETE=<shell> secrets ...` completion requests before the
/// normal parse.
///
/// On a completion request `CompleteEnv::complete()` prints the registration
/// stub (or the runtime candidates) and exits the process itself; when
/// `COMPLETE` is unset it returns and the normal application path proceeds
/// unchanged, so this is a no-op for ordinary invocations.
pub(crate) fn complete_env() {
    CompleteEnv::with_factory(crate::cli::Cli::command).complete();
}

/// Live completion candidates for a `name` argument (`pull`/`edit`/`which`): the
/// store entry names sourced at TAB time from the real SOPS store via
/// [`store::list_entries`]. Fails soft to an empty vec on any error — an unset
/// `SECRETS_REPO` with no resolvable `HOME`, or an unreadable store — so a
/// missing store never panics the completer. Only entry *names* are read, never
/// secret values, and no network access is performed.
pub(crate) fn name_candidates() -> Vec<CompletionCandidate> {
    let secrets_repo = std::env::var("SECRETS_REPO").ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cwd = std::env::current_dir().unwrap_or_default();
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|path| path.join(".config")));
    let config_path = config_home
        .unwrap_or_else(|| PathBuf::from("."))
        .join("secrets/stores.toml");
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|path| path.join(".local/share")));
    let remote = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    let Ok(selection) = store::select_store(
        secrets_repo.as_deref(),
        &config_path,
        &cwd,
        remote.as_deref(),
        data_home.as_deref(),
    ) else {
        return Vec::new();
    };

    store::list_entries(&selection.root)
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}
