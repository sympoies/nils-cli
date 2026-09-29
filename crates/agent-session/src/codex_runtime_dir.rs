//! Platform default for the Codex app-server private runtime directory, and
//! the visible record of an automatic runtime downgrade.
//!
//! `XDG_RUNTIME_DIR` stays authoritative whenever it is an absolute path. A
//! launchd job on macOS, or a system service, has no such variable; serve then
//! derives a private per-user root instead of silently falling back to the raw
//! tmux runtime. The root prefers the daemon's own state directory, which is
//! persistent and outside system temp-file cleanup, and falls back to a short
//! `/tmp/agent-session-<uid>` root when the state directory is too long to host
//! a Unix socket. Either root is created mode `0700` and must still pass the
//! owner, symlink, and mode validation applied to `XDG_RUNTIME_DIR`.
//!
//! When automatic selection still cannot use the app-server runtime, the reason
//! is recorded on the session runtime and projected as `startup.runtime_fallback`.

use std::env;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::{CliError, SessionRecord};

/// Runtime-extra key holding the allowlisted automatic-downgrade reason.
pub(crate) const FALLBACK_KEY: &str = "codex_runtime_fallback";
const GENERIC_FALLBACK_REASON: &str = "codex-app-server-runtime-unavailable";
const FALLBACK_REASONS: &[&str] = &[
    "codex-unavailable",
    "codex-version-unrecognized",
    "codex-version-too-old",
    "codex-app-server-transport-unavailable",
    "codex-app-server-runtime-dir-unavailable",
    "codex-app-server-runtime-dir-unsafe",
    "codex-app-server-socket-path-too-long",
];

/// Bytes a runtime root must leave for `/agent-session/cx-<16 hex>.sock`.
const SOCKET_SUFFIX_BYTES: usize = "/agent-session/cx-0123456789abcdef.sock".len();
const STATE_DIR_RUNTIME_ROOT: &str = "run";

/// Resolve the private runtime root, creating the platform default if needed.
///
/// The caller validates the returned root before using it.
pub(crate) fn runtime_root(state_dir: &Path) -> Result<PathBuf, CliError> {
    if let Some(root) = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Ok(root);
    }
    let root = platform_default_root(state_dir);
    match DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|err| {
            CliError::runtime(
                "codex-app-server-runtime-dir-unavailable",
                format!("failed to secure the default Codex runtime directory: {err}"),
                None,
            )
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            return Err(CliError::runtime(
                "codex-app-server-runtime-dir-unavailable",
                format!("failed to create the default Codex runtime directory: {err}"),
                None,
            ));
        }
    }
    Ok(root)
}

/// The per-user runtime root used when `XDG_RUNTIME_DIR` is absent.
fn platform_default_root(state_dir: &Path) -> PathBuf {
    let persistent = state_dir.join(STATE_DIR_RUNTIME_ROOT);
    if state_dir.is_absolute()
        && persistent.as_os_str().len() + SOCKET_SUFFIX_BYTES
            <= crate::codex_app_server::UNIX_SOCKET_PATH_BUDGET
    {
        return persistent;
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    PathBuf::from(format!("/tmp/agent-session-{uid}"))
}

/// Reduce an error code to the allowlisted downgrade reason vocabulary.
pub(crate) fn fallback_reason(code: &str) -> &'static str {
    FALLBACK_REASONS
        .iter()
        .copied()
        .find(|reason| *reason == code)
        .unwrap_or(GENERIC_FALLBACK_REASON)
}

/// Record why automatic selection kept the raw tmux runtime.
pub(crate) fn record_fallback(record: &mut SessionRecord, code: &str) {
    if let Some(runtime) = record.runtime.as_mut() {
        runtime
            .extra
            .insert(FALLBACK_KEY.to_string(), json!(fallback_reason(code)));
    }
}

pub(crate) fn clear_fallback(record: &mut SessionRecord) {
    if let Some(runtime) = record.runtime.as_mut() {
        runtime.extra.remove(FALLBACK_KEY);
    }
}

/// The allowlisted downgrade reason for the current runtime, if any.
pub(crate) fn fallback_for_view(record: &SessionRecord) -> Option<&'static str> {
    let runtime = record.runtime.as_ref()?;
    if runtime.kind == crate::codex_app_server::RUNTIME_KIND {
        return None;
    }
    runtime
        .extra
        .get(FALLBACK_KEY)
        .map(|value| fallback_reason(value.as_str().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_app_server::{self, RUNTIME_KIND};
    use crate::{CliContext, RuntimeInfo, SessionRecord};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    const CAPABLE_CODEX: &str = "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  Supported values: stdio://, unix://PATH'; exit 0; fi\nexit 1\n";

    struct Fixture {
        _tmp: tempfile::TempDir,
        context: CliContext,
        agent: PathBuf,
        record: SessionRecord,
    }

    fn fixture(codex_script: &str) -> Fixture {
        // A short /tmp prefix keeps the derived socket inside the Unix budget.
        let tmp = tempfile::Builder::new()
            .prefix("cx-")
            .tempdir_in("/tmp")
            .unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let agent = tmp.path().join("codex");
        fs::write(&agent, codex_script).unwrap();
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
        let record = tmux_record("runtime-dir");
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        Fixture {
            _tmp: tmp,
            context,
            agent,
            record,
        }
    }

    fn tmux_record(id: &str) -> SessionRecord {
        SessionRecord {
            schema_version: crate::SESSION_DOCUMENT_VERSION.to_string(),
            id: id.to_string(),
            agent: "codex".to_string(),
            mode: "interactive".to_string(),
            coordination_mode: crate::cli::CoordinationMode::Advisory,
            title: None,
            title_state: None,
            title_revision: 0,
            cwd: "/repo".to_string(),
            tmux_session: format!("hs-{id}"),
            prompt_file: None,
            log_file: None,
            created_at: "2030-01-01T00:00:00Z".to_string(),
            updated_at: "2030-01-01T00:00:00Z".to_string(),
            provider_resume: None,
            runtime: Some(RuntimeInfo {
                kind: "tmux".to_string(),
                tmux_session: format!("hs-{id}"),
                generation: 1,
                started_at: "2030-01-01T00:00:00Z".to_string(),
                launch_id: format!("launch-{id}"),
                extra: BTreeMap::new(),
            }),
            public_metadata: None,
            agent_args: Vec::new(),
            agent_bin: None,
            extra: BTreeMap::new(),
            resume_sidecar_extra: BTreeMap::new(),
        }
    }

    #[test]
    fn auto_selection_without_xdg_runtime_dir_uses_the_platform_default() {
        let lock = GlobalStateLock::new();
        let _runtime_dir = EnvGuard::remove(&lock, "XDG_RUNTIME_DIR");
        let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "auto");
        let mut fx = fixture(CAPABLE_CODEX);

        codex_app_server::configure_runtime(&fx.context, &fx.agent, &mut fx.record, true).unwrap();

        assert_eq!(fx.record.runtime.as_ref().unwrap().kind, RUNTIME_KIND);
        let socket = PathBuf::from(codex_app_server::socket_path(&fx.record).unwrap());
        let socket_dir = socket.parent().unwrap();
        assert_eq!(socket_dir, fx.context.state_dir.join("run/agent-session"));
        for dir in [socket_dir, socket_dir.parent().unwrap()] {
            let mode = fs::metadata(dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", Path::display(dir));
        }
        assert_eq!(
            socket.as_os_str().len(),
            fx.context.state_dir.join("run").as_os_str().len() + SOCKET_SUFFIX_BYTES
        );
        assert_eq!(fallback_for_view(&fx.record), None);
    }

    #[test]
    fn a_successful_selection_clears_an_earlier_fallback_marker() {
        let lock = GlobalStateLock::new();
        let _runtime_dir = EnvGuard::remove(&lock, "XDG_RUNTIME_DIR");
        let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "auto");
        let mut fx = fixture(CAPABLE_CODEX);
        record_fallback(&mut fx.record, "codex-app-server-runtime-dir-unavailable");

        codex_app_server::configure_runtime(&fx.context, &fx.agent, &mut fx.record, true).unwrap();

        assert_eq!(fx.record.runtime.as_ref().unwrap().kind, RUNTIME_KIND);
        assert_eq!(fallback_for_view(&fx.record), None);
    }

    #[test]
    fn platform_default_prefers_the_state_dir_and_falls_back_to_a_short_tmp_root() {
        let short = Path::new("/Users/operator/.local/state/agent-session");
        assert_eq!(platform_default_root(short), short.join("run"));

        let long = PathBuf::from(format!(
            "/Users/{}/.local/state/agent-session",
            "x".repeat(40)
        ));
        let fallback = platform_default_root(&long);
        let uid = unsafe { libc::geteuid() };
        assert_eq!(fallback, PathBuf::from(format!("/tmp/agent-session-{uid}")));
        assert!(
            fallback.as_os_str().len() + SOCKET_SUFFIX_BYTES
                <= codex_app_server::UNIX_SOCKET_PATH_BUDGET
        );

        // A relative state directory never anchors a private socket.
        let fallback = platform_default_root(Path::new("state"));
        assert_eq!(fallback, PathBuf::from(format!("/tmp/agent-session-{uid}")));
    }

    #[test]
    fn a_forced_platform_default_failure_is_visible_on_session_and_readiness() {
        let lock = GlobalStateLock::new();
        let _runtime_dir = EnvGuard::remove(&lock, "XDG_RUNTIME_DIR");
        let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "auto");
        let mut fx = fixture(CAPABLE_CODEX);
        // Another principal could have widened the derived directory.
        let root = fx.context.state_dir.join("run");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        crate::store_startup_projection(
            &mut fx.record,
            &crate::starting_projection("2030-01-01T00:00:00Z", "record"),
        );

        codex_app_server::configure_runtime(&fx.context, &fx.agent, &mut fx.record, true).unwrap();

        assert_eq!(fx.record.runtime.as_ref().unwrap().kind, "tmux");
        assert_eq!(
            fallback_for_view(&fx.record),
            Some("codex-app-server-runtime-dir-unsafe")
        );
        let persisted = crate::load_session_record(&fx.context, &fx.record.id).unwrap();
        let startup = crate::startup_projection_for_view(&persisted).unwrap();
        assert_eq!(
            serde_json::to_value(&startup).unwrap()["runtime_fallback"],
            "codex-app-server-runtime-dir-unsafe"
        );

        let readiness =
            codex_app_server::account_binding_readiness(&fx.agent, &fx.context.state_dir);
        assert_eq!(
            serde_json::to_value(readiness).unwrap(),
            serde_json::json!({
                "schema_version": "agent-session.codex-account-readiness.v1",
                "supported": false,
                "provider_version": "0.145.0",
                "reason_code": "codex-app-server-runtime-dir-unsafe",
            })
        );
    }

    #[test]
    fn an_unavailable_capability_is_visible_on_the_session() {
        let lock = GlobalStateLock::new();
        let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "auto");
        let mut fx = fixture("#!/bin/sh\nprintf '%s\\n' 'codex-cli 0.100.0'\n");

        codex_app_server::configure_runtime(&fx.context, &fx.agent, &mut fx.record, true).unwrap();

        assert_eq!(fx.record.runtime.as_ref().unwrap().kind, "tmux");
        assert_eq!(fallback_for_view(&fx.record), Some("codex-version-too-old"));
    }

    #[test]
    fn an_explicit_raw_preference_is_not_a_downgrade() {
        let lock = GlobalStateLock::new();
        let _runtime_dir = EnvGuard::remove(&lock, "XDG_RUNTIME_DIR");
        let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "raw");
        let mut fx = fixture(CAPABLE_CODEX);

        codex_app_server::configure_runtime(&fx.context, &fx.agent, &mut fx.record, true).unwrap();

        assert_eq!(fx.record.runtime.as_ref().unwrap().kind, "tmux");
        assert_eq!(fallback_for_view(&fx.record), None);
    }

    #[test]
    fn unknown_fallback_codes_are_reduced_to_a_generic_allowlisted_code() {
        assert_eq!(
            fallback_reason("codex-app-server-socket-path-too-long"),
            "codex-app-server-socket-path-too-long"
        );
        assert_eq!(
            fallback_reason("/private/path leaked"),
            "codex-app-server-runtime-unavailable"
        );
        let mut record = tmux_record("tampered");
        record
            .runtime
            .as_mut()
            .unwrap()
            .extra
            .insert(FALLBACK_KEY.to_string(), serde_json::json!("/private/path"));
        assert_eq!(
            fallback_for_view(&record),
            Some("codex-app-server-runtime-unavailable")
        );
    }
}
