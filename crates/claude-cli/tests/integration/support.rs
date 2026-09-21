//! Shared fixtures for the `claude-cli` integration modules: the contained
//! `CmdOptions` baseline, the refresh-cooldown guard, cache and fake-binary
//! writers, and the wait helpers the background-refresh tests depend on.

use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::http::{LoopbackServer, TestServer};
use nils_test_support::{bin, git as test_git};
use pretty_assertions::assert_eq;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn claude_cli_bin() -> PathBuf {
    bin::resolve("claude-cli")
}

pub(crate) fn run(args: &[&str], options: &CmdOptions) -> CmdOutput {
    let bin = claude_cli_bin();
    cmd::run_with(&bin, args, options)
}

pub(crate) fn assert_exit(output: &CmdOutput, code: i32) {
    assert_eq!(output.code, code, "stderr: {}", output.stderr_text());
}

pub(crate) fn stdout(output: &CmdOutput) -> String {
    output.stdout_text()
}

pub(crate) fn stderr(output: &CmdOutput) -> String {
    output.stderr_text()
}

/// An endpoint nothing listens on, so a request that escapes a test's control
/// fails immediately instead of reaching a real host.
///
/// `base_options` clears the `CLAUDE_PROMPT` prefix to isolate ambient config,
/// which also clears `CLAUDE_PROMPT_SEGMENT_ENDPOINT` — and the production
/// default is the real usage endpoint. This is **defence in depth, not a fix for
/// an observed reach**: the same prefix removal also clears the token variables
/// and `base_options` disables the keychain, so `refresh_blocking` returns at its
/// token guard before it would fetch anything. What it buys is that the first
/// token-bearing test added under this default cannot reach a real host by
/// omission. A test that wants a server sets its own endpoint afterwards and
/// wins, because removals are applied before values.
pub(crate) const UNROUTABLE_ENDPOINT: &str = "http://127.0.0.1:9/usage";

/// Bound the fast-fail so it stays fast on a host that drops rather than refuses
/// loopback traffic. `gemini-cli` pins its timeouts alongside its unroutable
/// endpoint for the same reason.
pub(crate) const FAST_FAIL_MAX_TIME_SECONDS: &str = "1";

/// Keeps a plain `prompt-segment` run from launching a detached refresh child.
///
/// A run whose cache is expired calls `enqueue_background_refresh`, which spawns
/// `prompt-segment --refresh` and returns without waiting. That child outlives the
/// test, and its first act is `create_dir_all` on the cache directory, so it can
/// recreate the fixture `TempDir` teardown has already removed — class 3 in
/// `docs/specs/test-temp-directory-policy.md`.
///
/// A test whose subject is rendering rather than refreshing opts out through the
/// production cooldown instead of racing the child or neutering it: a recent
/// `usage.refresh.at` plus a long `CLAUDE_PROMPT_SEGMENT_REFRESH_MIN_SECONDS`
/// makes `enqueue_background_refresh` return before it spawns anything. That is
/// the approach the policy prefers, and unlike a no-op executable it is
/// falsifiable — see [`RefreshCooldown::assert_held`].
pub(crate) struct RefreshCooldown {
    marker: PathBuf,
    stamp: String,
}

impl RefreshCooldown {
    /// Hold the cooldown for the `usage.json` cache in `cache_dir`.
    pub(crate) fn hold(cache_dir: &Path) -> Self {
        std::fs::create_dir_all(cache_dir).expect("cache dir");
        let marker = cache_dir.join("usage.refresh.at");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs().saturating_sub(5).max(1))
            .expect("epoch")
            .to_string();
        std::fs::write(&marker, &stamp).expect("write refresh marker");

        Self { marker, stamp }
    }

    /// The environment that makes the held marker actually gate the spawn.
    pub(crate) fn env(&self) -> (&'static str, &'static str) {
        ("CLAUDE_PROMPT_SEGMENT_REFRESH_MIN_SECONDS", "3600")
    }

    /// Fails when the run spawned a refresh child after all.
    ///
    /// `enqueue_background_refresh` rewrites the marker immediately before
    /// spawning, so an unchanged marker is proof that it returned on the cooldown
    /// and that no background writer can outlive this test.
    pub(crate) fn assert_held(&self) {
        assert_eq!(
            std::fs::read_to_string(&self.marker).expect("read refresh marker"),
            self.stamp,
            "the run spawned a detached refresh child instead of honouring the cooldown"
        );
    }
}

pub(crate) fn base_options(cache_dir: &Path) -> CmdOptions {
    let missing_claude = path_str(&cache_dir.join("missing-claude"));
    CmdOptions::default()
        .with_env_remove_prefix("CLAUDE_CLI_")
        .with_env_remove_prefix("CLAUDE_PROMPT")
        .with_env_remove_many(&[
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ])
        .with_env_remove("NO_COLOR")
        .with_env_remove("TZ")
        .with_env("NO_PROXY", "127.0.0.1,localhost")
        .with_env("no_proxy", "127.0.0.1,localhost")
        .with_env(
            "CLAUDE_CONFIG_DIR",
            &path_str(&cache_dir.join("claude-config")),
        )
        .with_env("CLAUDE_PROMPT_SEGMENT_CACHE_DIR", &path_str(cache_dir))
        .with_env("CLAUDE_PROMPT_SEGMENT_KEYCHAIN_DISABLED", "1")
        .with_env("CLAUDE_PROMPT_SEGMENT_ENDPOINT", UNROUTABLE_ENDPOINT)
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_MAX_TIME_SECONDS",
            FAST_FAIL_MAX_TIME_SECONDS,
        )
        .with_env("CLAUDE_CLI_BIN", &missing_claude)
        .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_BIN", &missing_claude)
}

pub(crate) trait ClaudeFixtureOptions {
    fn with_fake_claude(self, bin_dir: &Path) -> Self;
}

impl ClaudeFixtureOptions for CmdOptions {
    fn with_fake_claude(self, bin_dir: &Path) -> Self {
        let claude = path_str(&bin_dir.join("claude"));
        self.with_path_prepend(bin_dir)
            .with_env("CLAUDE_CLI_BIN", &claude)
            .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_BIN", &claude)
    }
}

pub(crate) fn path_str(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(crate) fn write_cache(cache_dir: &Path, body: &str) -> PathBuf {
    std::fs::create_dir_all(cache_dir).expect("cache dir");
    let path = cache_dir.join("usage.json");
    std::fs::write(&path, body).expect("write cache");
    path
}

/// `<stem>.refresh.lock`, matching `prompt_segment::refresh::sibling_path`.
pub(crate) fn refresh_lock_path(cache_file: &Path) -> PathBuf {
    let stem = cache_file
        .file_stem()
        .expect("cache file stem")
        .to_string_lossy()
        .to_string();
    cache_file.with_file_name(format!("{stem}.refresh.lock"))
}

/// Whether no process currently holds the refresh lock.
///
/// `flock` locks belong to the open file description, so a fresh `open` here
/// contends with the detached child's descriptor even though both live on this
/// host.
pub(crate) fn refresh_lock_is_free(lock_file: &Path) -> bool {
    let Ok(file) = OpenOptions::new().read(true).write(true).open(lock_file) else {
        // No lock file means no refresh ever took it; nothing to wait for.
        return true;
    };
    // SAFETY: `flock` observes the valid descriptor owned by `file`.
    let acquired = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if acquired {
        // SAFETY: same descriptor, still owned by `file` here.
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    }
    acquired
}

/// Waits for the detached refresh child to drop `<stem>.refresh.lock`.
///
/// `refresh_blocking` holds that lock across *every* write it makes, so a free
/// lock means the child has no filesystem work left in the fixture. Callers must
/// first observe something the child only does while holding the lock (its HTTP
/// request, or the refreshed cache body); otherwise a free lock just means the
/// child has not started yet.
pub(crate) fn wait_for_refresh_lock_release(cache_file: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let lock_file = refresh_lock_path(cache_file);
    while !refresh_lock_is_free(&lock_file) {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(25));
    }
    true
}

/// Waits until the background refresh wrote `expected` *and* fully settled.
///
/// Returning as soon as the cache body lands is not enough: `refresh_blocking`
/// still has to write the `<stem>.refresh.at` marker, and `write_atomic`
/// re-creates the parent directory before writing. That marker write therefore
/// resurrects the fixture directory *after* `TempDir` removed it, leaking one
/// directory under `$TMPDIR` per run while the test still passes.
pub(crate) fn wait_for_background_refresh_settled(
    cache_file: &Path,
    expected: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if std::fs::read_to_string(cache_file).ok().as_deref() == Some(expected) {
            break;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(25));
    }
    wait_for_refresh_lock_release(
        cache_file,
        deadline.saturating_duration_since(Instant::now()),
    )
}

pub(crate) fn make_old(path: &Path) {
    set_modified(path, SystemTime::now() - Duration::from_secs(120));
}

pub(crate) fn set_modified(path: &Path, time: SystemTime) {
    let file = OpenOptions::new().write(true).open(path).expect("open");
    file.set_modified(time).expect("set modified");
}

pub(crate) fn wait_for_requests(
    server: &LoopbackServer,
    expected: usize,
) -> Vec<nils_test_support::http::RecordedRequest> {
    let deadline = Instant::now() + Duration::from_secs(4);
    let quiet_period = Duration::from_millis(300);
    let mut quiet_since = None;
    let mut requests = Vec::new();
    while Instant::now() < deadline {
        let new_requests = server.take_requests();
        if !new_requests.is_empty() {
            requests.extend(new_requests);
            quiet_since = None;
        }
        if requests.len() >= expected {
            let since = quiet_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= quiet_period {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    requests
}

pub(crate) fn wait_for_test_requests(
    server: &TestServer,
    expected: usize,
) -> Vec<nils_test_support::http::RecordedRequest> {
    let deadline = Instant::now() + Duration::from_secs(4);
    let quiet_period = Duration::from_millis(300);
    let mut quiet_since = None;
    let mut requests = Vec::new();
    while Instant::now() < deadline {
        let new_requests = server.take_requests();
        if !new_requests.is_empty() {
            requests.extend(new_requests);
            quiet_since = None;
        }
        if requests.len() >= expected {
            let since = quiet_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= quiet_period {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    requests
}

pub(crate) fn usage_json(five_utilization: f64, weekly_utilization: f64) -> String {
    usage_json_with_resets(
        five_utilization,
        weekly_utilization,
        "2026-01-01T00:00:00+00:00",
        "2026-01-03T12:30:00+00:00",
    )
}

pub(crate) fn usage_json_with_resets(
    five_utilization: f64,
    weekly_utilization: f64,
    five_resets_at: &str,
    weekly_resets_at: &str,
) -> String {
    format!(
        r#"{{
          "usage": {{
            "five_hour": {{"utilization": {five_utilization}, "resets_at": {five_resets_at:?}}},
            "seven_day": {{"utilization": {weekly_utilization}, "resets_at": {weekly_resets_at:?}}}
          }}
        }}"#
    )
}

#[cfg(unix)]
pub(crate) fn write_fake_claude(dir: &Path, body: &str) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("fake bin dir");
    let path = bin_dir.join("claude");
    std::fs::write(&path, body).expect("write fake claude");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("chmod fake claude");
    bin_dir
}

pub(crate) fn init_git_repo(repo: &Path) {
    std::fs::create_dir_all(repo).expect("repo");
    test_git::git(repo, &["init"]);
    test_git::git(repo, &["config", "user.name", "Test User"]);
    test_git::git(repo, &["config", "user.email", "test@example.com"]);
    test_git::git(repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("base.txt"), "base\n").expect("base");
    test_git::git(repo, &["add", "base.txt"]);
    test_git::git(repo, &["commit", "-m", "chore: base"]);
}

#[cfg(unix)]
pub(crate) fn write_agent_commit_success_tools(dir: &Path) -> PathBuf {
    let bin_dir = write_fake_claude(
        dir,
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt --model --effort'
  exit 0
fi
cat >/dev/null
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"structured_output":{"type":"test","scope":"agent","subject":"commit staged changes","body_bullets":[]}}'
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "staged-context" ]; then
  printf '%s\n' 'STAGED BUNDLE'
  exit 0
fi
repo=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '--repo' ]; then repo="$arg"; fi
  previous="$arg"
done
"$REAL_GIT" -C "$repo" commit -m 'test(agent): commit staged changes' >/dev/null
if [ -n "${SEMANTIC_TEST_RETARGET_URL:-}" ]; then
  "$REAL_GIT" -C "$repo" remote set-url origin "$SEMANTIC_TEST_RETARGET_URL"
fi
"#,
    );
    bin_dir
}
