//! Node.js launcher for tests, matching `scripts/ci/lib/node-scope.sh`.
//!
//! When a systemd user scope can start, Node runs inside it with `MemoryMax=6G`
//! and no swap. Inside a gate that exports `NILS_CLI_GATE_SLICE`, the scope is
//! placed under that slice and is required: if it cannot start, the launch
//! fails closed and Node never runs. Outside a gate, a host where no systemd
//! user manager can start a scope (for example macOS and most hosted CI
//! runners) runs `node` unchanged.

use std::process::{Command, Stdio};

const SCOPE_ARGS: [&str; 7] = [
    "--user",
    "--scope",
    "--quiet",
    "-p",
    "MemoryMax=6G",
    "-p",
    "MemorySwapMax=0",
];

/// The program and leading arguments for one Node launch, or the fail-closed
/// refusal when a gate requires a scope that cannot start.
pub fn launch_plan(
    gate_slice: Option<&str>,
    scope_available: bool,
) -> Result<(String, Vec<String>), String> {
    if !scope_available {
        if let Some(slice) = gate_slice {
            return Err(format!(
                "node scope unavailable inside gate slice {slice}; refusing to run node"
            ));
        }
        return Ok(("node".to_string(), Vec::new()));
    }
    let mut args: Vec<String> = SCOPE_ARGS.iter().map(|arg| arg.to_string()).collect();
    if let Some(slice) = gate_slice {
        args.push(format!("--slice={slice}"));
    }
    args.push("--".to_string());
    args.push("node".to_string());
    Ok(("systemd-run".to_string(), args))
}

fn scope_available() -> bool {
    Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "--quiet",
            "-p",
            "MemoryMax=64M",
            "-p",
            "MemorySwapMax=0",
            "--",
            "true",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A `Command` that runs Node under the scope policy above; append the script
/// and its arguments as usual. Panics inside a gate when no scope can start,
/// so the test fails instead of running Node outside the 6 GiB boundary.
pub fn command() -> Command {
    let slice = std::env::var("NILS_CLI_GATE_SLICE")
        .ok()
        .filter(|slice| !slice.is_empty());
    match launch_plan(slice.as_deref(), scope_available()) {
        Ok((program, args)) => {
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        Err(message) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::launch_plan;
    use pretty_assertions::assert_eq;

    fn scoped(slice: Option<&str>) -> Vec<String> {
        let mut args: Vec<String> = [
            "--user",
            "--scope",
            "--quiet",
            "-p",
            "MemoryMax=6G",
            "-p",
            "MemorySwapMax=0",
        ]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
        if let Some(slice) = slice {
            args.push(format!("--slice={slice}"));
        }
        args.extend(["--".to_string(), "node".to_string()]);
        args
    }

    #[test]
    fn available_scope_wraps_node_without_a_gate() {
        assert_eq!(
            launch_plan(None, true),
            Ok(("systemd-run".to_string(), scoped(None)))
        );
    }

    #[test]
    fn available_scope_is_placed_under_the_gate_slice() {
        assert_eq!(
            launch_plan(Some("nilsgatefixture.slice"), true),
            Ok((
                "systemd-run".to_string(),
                scoped(Some("nilsgatefixture.slice"))
            ))
        );
    }

    #[test]
    fn outside_a_gate_an_unavailable_scope_runs_node_unchanged() {
        assert_eq!(
            launch_plan(None, false),
            Ok(("node".to_string(), Vec::new()))
        );
    }

    #[test]
    fn inside_a_gate_an_unavailable_scope_fails_closed() {
        let refusal = launch_plan(Some("nilsgatefixture.slice"), false).unwrap_err();
        assert!(
            refusal.contains("node scope unavailable inside gate slice"),
            "{refusal}"
        );
    }
}
