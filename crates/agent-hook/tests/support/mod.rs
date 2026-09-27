#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use sha2::{Digest, Sha256};

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub state_home: PathBuf,
    pub session_state: PathBuf,
    pub config: PathBuf,
    pub policy: PathBuf,
}

impl Fixture {
    pub fn new(policy: &str) -> Self {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = fs::canonicalize(temp.path()).expect("canonical tempdir");
        let home = root.join("home");
        let config_home = root.join("config");
        let state_home = root.join("state");
        let session_state = root.join("session-state");
        let config_dir = config_home.join("agent-hook");
        let policy_dir = root.join("data/agent-hook/policies/current");
        fs::create_dir_all(&config_dir).expect("config dir");
        fs::create_dir_all(&policy_dir).expect("policy dir");
        fs::create_dir_all(&state_home).expect("state dir");
        fs::create_dir_all(&session_state).expect("session state dir");
        fs::create_dir_all(&home).expect("home");
        let policy_path = policy_dir.join("policy.toml");
        fs::write(&policy_path, policy).expect("policy");
        Self::set_private(&policy_path);
        let digest = sha256(policy.as_bytes());
        let config_path = config_dir.join("config.toml");
        fs::write(
            &config_path,
            format!(
                "schema_version = \"agent-hook.config.v1\"\n\n[policy]\npath = {}\ndigest = \"{}\"\n",
                toml_string(&policy_path),
                digest,
            ),
        )
        .expect("config");
        Self::set_private(&config_path);
        Self {
            _temp: temp,
            root,
            home,
            config_home,
            state_home,
            session_state,
            config: config_path,
            policy: policy_path,
        }
    }

    pub fn run(&self, args: &[&str], stdin: Option<&str>) -> CmdOutput {
        self.run_with_env(args, stdin, &[])
    }

    pub fn run_in_cwd(&self, cwd: &Path, args: &[&str], stdin: Option<&str>) -> CmdOutput {
        self.run_with_options(cwd, args, stdin, &[], &[])
    }

    pub fn run_with_env(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        envs: &[(&str, &str)],
    ) -> CmdOutput {
        self.run_with_env_and_removals(args, stdin, envs, &[])
    }

    pub fn run_with_env_and_removals(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        envs: &[(&str, &str)],
        removals: &[&str],
    ) -> CmdOutput {
        self.run_with_options(&self.root, args, stdin, envs, removals)
    }

    fn run_with_options(
        &self,
        cwd: &Path,
        args: &[&str],
        stdin: Option<&str>,
        envs: &[(&str, &str)],
        removals: &[&str],
    ) -> CmdOutput {
        // Helper resolution, coordination identity, and the state root are all
        // env overrides that outrank what a test can set through `PATH` or the
        // fixture layout, so every one of them has to be dropped unless this
        // call asked for it. Removals are applied before values, so the
        // `AGENT_SESSION_STATE_DIR` default below and any explicit `envs` entry
        // still reach the child. See `sympoies/nils-cli#1420`.
        let options = CmdOptions::new()
            .with_cwd(cwd)
            .without_ambient_managed_session_env()
            .with_env_remove("CODEX_HOME")
            .with_env("HOME", self.home.to_str().expect("home UTF-8"))
            .with_env(
                "XDG_CONFIG_HOME",
                self.config_home.to_str().expect("config UTF-8"),
            )
            .with_env(
                "XDG_STATE_HOME",
                self.state_home.to_str().expect("state UTF-8"),
            );
        let state_overridden = envs
            .iter()
            .any(|(name, _)| *name == "AGENT_SESSION_STATE_DIR");
        let options = if removals.contains(&"AGENT_SESSION_STATE_DIR") || state_overridden {
            options
        } else {
            options.with_env(
                "AGENT_SESSION_STATE_DIR",
                self.session_state.to_str().expect("session state UTF-8"),
            )
        };
        let options = options.with_envs(envs);
        let options = removals
            .iter()
            .fold(options, |options, name| options.with_env_remove(name));
        let options = if let Some(stdin) = stdin {
            options.with_stdin_str(stdin)
        } else {
            options.with_stdin_bytes(&[])
        };
        run_resolved("agent-hook", args, &options)
    }

    pub fn set_private(path: &Path) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("private mode");
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    format!(
        "sha256:{}",
        hash.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

pub fn target_binding_digest(path: &Path) -> String {
    let (effective, start) = effective_target_and_start(path);
    let output = Command::new("git")
        .arg("-C")
        .arg(&start)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("git lookup");
    let binding_root = if output.status.success() {
        PathBuf::from(
            std::str::from_utf8(&output.stdout)
                .expect("git root UTF-8")
                .trim(),
        )
    } else {
        start
    };
    let canonical = fs::canonicalize(&binding_root).expect("canonical binding root");
    let metadata = fs::metadata(&canonical).expect("binding metadata");
    let mut material = b"agent-hook.target-binding.v2\0".to_vec();
    material.extend_from_slice(effective.as_os_str().as_encoded_bytes());
    material.push(0);
    material.extend_from_slice(canonical.as_os_str().as_encoded_bytes());
    material.extend_from_slice(&metadata.dev().to_le_bytes());
    material.extend_from_slice(&metadata.ino().to_le_bytes());
    sha256(&material)
}

fn effective_target_and_start(path: &Path) -> (PathBuf, PathBuf) {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .expect("missing target component")
                        .to_os_string(),
                );
                ancestor = ancestor.parent().expect("target ancestor");
            }
            Err(error) => panic!("target metadata: {error}"),
        }
    }
    let existing_ancestor = fs::canonicalize(ancestor).expect("effective target ancestor");
    let mut effective = existing_ancestor.clone();
    for component in suffix.iter().rev() {
        effective.push(component);
    }
    let start = if !suffix.is_empty() || existing_ancestor.is_dir() {
        existing_ancestor
    } else {
        existing_ancestor
            .parent()
            .expect("effective target parent")
            .to_path_buf()
    };
    (effective, start)
}

pub fn toml_string(path: &Path) -> String {
    toml::Value::String(path.to_string_lossy().into_owned()).to_string()
}

pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64
}

/// A real long-lived client ancestor, with request/receipt files confined to
/// the test fixture. Keep the pipe open so each nils invocation is its child.
#[cfg(target_os = "linux")]
pub struct OwnedFinishLineClient {
    child: std::process::Child,
    request: PathBuf,
    response: PathBuf,
    code: PathBuf,
    reaped: bool,
}

#[cfg(target_os = "linux")]
impl OwnedFinishLineClient {
    pub fn open(fixture: &Fixture, session: &str, token: &str) -> (Self, String) {
        use std::os::unix::process::CommandExt;
        let private_dir = fixture.root.join(format!("owner-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&private_dir).expect("owner private directory");
        fs::set_permissions(&private_dir, fs::Permissions::from_mode(0o700))
            .expect("private owner mode");
        let request = private_dir.join("request.json");
        let response = request.with_extension("response");
        let code = request.with_extension("code");
        let errors = request.with_extension("stderr");
        let child = Command::new("/bin/bash")
            .args([
                "-c",
                r#"
while IFS= read -r action; do
  "$1" finish-line "$action" --format json < "$2" > "$3"
  printf '%s' "$?" > "$4"
done
"#,
                "owned-finish-line-client",
            ])
            .arg(nils_test_support::bin::resolve("agent-hook"))
            .arg(&request)
            .arg(&response)
            .arg(&code)
            .current_dir(&fixture.root)
            .env("HOME", &fixture.home)
            .env("XDG_CONFIG_HOME", &fixture.config_home)
            .env("XDG_STATE_HOME", &fixture.state_home)
            .env("AGENT_SESSION_STATE_DIR", &fixture.session_state)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(fs::File::create(errors).expect("owner stderr"))
            .process_group(0)
            .spawn()
            .expect("real client owner");
        let mut owner = Self {
            child,
            request,
            response,
            code,
            reaped: false,
        };
        let (status, opened) = owner.call(
            "open",
            &serde_json::json!({
                "schema_version": "agent-hook.finish-line.open.v1", "product": "dsh",
                "session_id": session, "turn_id": "turn-open", "cwd": fixture.root,
                "attempt_token": token, "owner_pid": owner.child.id(),
            }),
        );
        assert_eq!(status, 0, "owned open={opened}");
        let capability = opened["data"]["runner_capability"]
            .as_str()
            .expect("owner capability")
            .to_string();
        (owner, capability)
    }

    pub fn start(&mut self, action: &str, request: &serde_json::Value) {
        use std::io::Write;
        let _ = fs::remove_file(&self.response);
        let _ = fs::remove_file(&self.code);
        fs::write(&self.request, request.to_string()).expect("private owner request");
        Self::set_request_private(&self.request);
        self.child
            .stdin
            .as_mut()
            .expect("live owner pipe")
            .write_all(format!("{action}\n").as_bytes())
            .expect("send owner action");
    }

    fn set_request_private(path: &Path) {
        Fixture::set_private(path);
    }

    pub fn call(&mut self, action: &str, request: &serde_json::Value) -> (i32, serde_json::Value) {
        self.start(action, request);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(code) = fs::read_to_string(&self.code)
                && let Ok(code) = code.parse()
                && let Ok(bytes) = fs::read(&self.response)
                && let Ok(response) = serde_json::from_slice(&bytes)
            {
                return (code, response);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "owned invocation did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub fn result(&self) -> Option<(i32, serde_json::Value)> {
        let code = fs::read_to_string(&self.code).ok()?.parse().ok()?;
        let response = serde_json::from_slice(&fs::read(&self.response).ok()?).ok()?;
        Some((code, response))
    }

    pub fn kill_owner_only(&self) {
        assert_eq!(
            unsafe { libc::kill(self.child.id() as i32, libc::SIGKILL) },
            0
        );
        // Keep the dead owner unreaped until Drop, reserving its process-group
        // identity while its surviving supervisor races recovery cleanup.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let status = fs::read_to_string(format!("/proc/{}/status", self.child.id()))
                .expect("unreaped owner status");
            if status
                .lines()
                .any(|line| line.starts_with("State:") && line.contains("Z (zombie)"))
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "owner did not die");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    pub fn kill_and_wait(&mut self) {
        if !self.reaped {
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.wait();
            self.reaped = true;
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for OwnedFinishLineClient {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}
