//! CLI boundary tests: synthetic stores and subprocesses, no provider operations.
use nils_test_support::{StubBinDir, bin, write_exe};
use pretty_assertions::assert_eq;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

const CANARY: &str = "SYNTHETIC-SETTER-CANARY";

struct Fixture {
    root: TempDir,
    stubs: StubBinDir,
    store: PathBuf,
    home: nils_test_support::hermetic::HermeticHome,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let stubs = StubBinDir::new();
        let store = root.path().join("store");
        fs::create_dir_all(store.join("repos")).unwrap();
        fs::write(store.join(".git"), "gitdir: synthetic\n").unwrap();
        write_exe(
            stubs.path(),
            "git",
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$OPS_LOG"
case "$*" in
  'remote get-url origin') echo 'https://github.com/example/service.git';;
  'symbolic-ref --quiet --short HEAD')
    test "${DETACHED:-}" != yes || exit 1
    echo "${BRANCH:-feat/store-update}";;
  'symbolic-ref --quiet --short refs/remotes/origin/HEAD')
    test "${NO_DEFAULT:-}" != yes || exit 1
    echo origin/main;;
  'rev-parse --show-toplevel') echo "$SECRETS_REPO";;
  'status --porcelain -z --untracked-files=all') printf '%s' "${DIRTY:-}";;
  *) exit 99;;
esac
"#,
        );
        write_exe(
            stubs.path(),
            "git-cli",
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$OPS_LOG"
test "$*" = 'worktree list --format json' || exit 99
python3 -c 'import json,os; print(json.dumps({"ok":True,"data":{"entries":[{"path":os.environ["SECRETS_REPO"],"managed":os.environ.get("UNMANAGED")!="yes","branch":os.environ.get("BRANCH","feat/store-update"),"bare":False,"detached":False,"prunable":False}]}}))'
"#,
        );
        for name in ["semantic-commit", "forge-cli"] {
            write_exe(
                stubs.path(),
                name,
                "#!/bin/sh\nprintf '%s\\n' provider-unexpected >> \"$OPS_LOG\"\nexit 99\n",
            );
        }
        write_exe(stubs.path(), "sops", include_str!("support/setter-sops.py"));
        Self {
            root,
            stubs,
            store,
            home: nils_test_support::hermetic::HermeticHome::new(),
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(bin::resolve("secrets"));
        nils_test_support::hermetic::apply(&mut cmd, &self.home, &[]);
        cmd.current_dir(self.root.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.stubs.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("SECRETS_REPO", &self.store)
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("OPS_LOG", self.root.path().join("ops"))
            .env("SOPS_LOG", self.root.path().join("sops-ops"))
            .env("SYNTHETIC_CANARY", CANARY)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    fn run(&self, input: &[u8], extra: &[(&str, &str)]) -> Output {
        let mut cmd = self.command();
        cmd.args([
            "--format",
            "json",
            "set",
            "service",
            "TOKEN",
            "--value-stdin",
        ]);
        for (key, value) in extra {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().unwrap();
        let _ = child.stdin.take().unwrap().write_all(input);
        let output = child.wait_with_output().unwrap();
        self.assert_no_leak(&output);
        output
    }

    fn assert_no_leak(&self, output: &Output) {
        for bytes in [&output.stdout, &output.stderr] {
            assert!(
                !String::from_utf8_lossy(bytes).contains(CANARY),
                "secret leaked"
            );
        }
        for log in ["ops", "sops-ops"] {
            let contents = fs::read_to_string(self.root.path().join(log)).unwrap_or_default();
            assert!(!contents.contains(CANARY), "secret in child argv");
        }
    }

    fn target(&self) -> PathBuf {
        self.store.join("repos/service.enc.env")
    }

    fn assert_no_temp(&self) {
        let names: Vec<_> = fs::read_dir(self.store.join("repos"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(names.is_empty(), "owned temporary output remains");
    }
}

fn encode(document: &str) -> String {
    document
        .lines()
        .map(|line| {
            if line.starts_with('#') || line.is_empty() {
                return format!("{line}\n");
            }
            let (key, value) = line.split_once('=').unwrap();
            let hex: String = value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("{key}=ENC[AES256_GCM,data:{hex}]\n")
        })
        .collect::<String>()
        + "sops_mac=ENC[AES256_GCM,data:synthetic]\n"
}

fn decode(ciphertext: &str) -> String {
    ciphertext
        .lines()
        .filter(|line| !line.starts_with("sops_"))
        .map(|line| {
            if line.starts_with('#') || line.is_empty() {
                return format!("{line}\n");
            }
            let (key, value) = line.split_once("=ENC[AES256_GCM,data:").unwrap();
            let hex = value.strip_suffix(']').unwrap();
            let bytes: Vec<_> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            format!("{key}={}\n", String::from_utf8(bytes).unwrap())
        })
        .collect()
}

#[test]
fn set_hands_off_pending_without_any_git_delivery() {
    let f = Fixture::new();
    let out = f.run(CANARY.as_bytes(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let data = &json["data"];
    assert_eq!(data["encrypted_write_completed"], true);
    assert_eq!(data["delivery_verified"], false);
    assert_eq!(data["delivery_status"], "pending");
    assert_eq!(data["committed"], false);
    assert_eq!(data["pr_created"], false);
    assert!(data["next_actions"].to_string().contains("semantic-commit"));
    assert!(data["next_actions"].to_string().contains("forge-cli"));
    assert_eq!(
        decode(&fs::read_to_string(f.target()).unwrap()),
        format!("TOKEN=\"{CANARY}\"\n")
    );
    let ops = fs::read_to_string(f.root.path().join("ops")).unwrap();
    assert!(!ops.lines().any(|line| line.starts_with("add ")
        || line.starts_with("commit")
        || line.starts_with("push")
        || line.starts_with("provider")));
    f.assert_no_temp();
}

#[test]
fn set_updates_exact_key_and_preserves_other_keys_and_comments() {
    let f = Fixture::new();
    let original = "# retained\nTOKEN_SUFFIX=untouched\nTOKEN=old\nOTHER='literal value'\n";
    fs::write(f.target(), encode(original)).unwrap();
    let input = format!("{CANARY} \"quote\" 'single' \\ $HOME\nnext\n");
    let out = f.run(input.as_bytes(), &[]);
    assert!(out.status.success());
    assert_eq!(
        decode(&fs::read_to_string(f.target()).unwrap()),
        format!(
            "# retained\nTOKEN_SUFFIX=untouched\nTOKEN=\"{CANARY} \\\"quote\\\" 'single' \\\\ \\$HOME\\nnext\\n\"\nOTHER='literal value'\n"
        )
    );
    let sops_ops = fs::read_to_string(f.root.path().join("sops-ops")).unwrap();
    assert!(
        sops_ops
            .lines()
            .all(|line| line.contains("--filename-override repos/service.enc.env"))
    );
}

#[test]
fn set_accepts_empty_and_preserves_one_final_linefeed() {
    for (input, expected) in [("", "TOKEN=\"\"\n"), ("value\n", "TOKEN=\"value\\n\"\n")] {
        let f = Fixture::new();
        assert!(f.run(input.as_bytes(), &[]).status.success());
        assert_eq!(decode(&fs::read_to_string(f.target()).unwrap()), expected);
    }
}

#[test]
fn set_refuses_ungoverned_or_dirty_store_before_sops_or_writing() {
    for env in [
        [("BRANCH", "main")],
        [("UNMANAGED", "yes")],
        [("DIRTY", " M unrelated")],
        [("NO_DEFAULT", "yes")],
        [("DETACHED", "yes")],
    ] {
        let f = Fixture::new();
        let out = f.run(CANARY.as_bytes(), &env);
        assert!(!out.status.success());
        assert!(!f.target().exists());
        assert!(!f.root.path().join("sops-ops").exists());
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap()
                .contains("worktree")
        );
    }
}

#[test]
fn set_refuses_primary_checkout_before_reading_or_writing() {
    let f = Fixture::new();
    fs::remove_file(f.store.join(".git")).unwrap();
    fs::create_dir(f.store.join(".git")).unwrap();
    assert!(!f.run(CANARY.as_bytes(), &[]).status.success());
    assert!(!f.target().exists());
    assert!(!f.root.path().join("sops-ops").exists());
}

#[test]
fn set_preinstall_failures_preserve_ciphertext_and_clean_output() {
    for mode in ["encrypt-fail", "validate-fail", "plaintext", "decrypt-fail"] {
        let f = Fixture::new();
        let previous = encode("TOKEN=old\nOTHER=keep\n");
        fs::write(f.target(), &previous).unwrap();
        let out = f.run(CANARY.as_bytes(), &[("SOPS_MODE", mode)]);
        assert!(!out.status.success(), "{mode}");
        assert_eq!(fs::read_to_string(f.target()).unwrap(), previous);
        f.assert_no_temp();
    }
}

#[test]
fn set_refuses_invalid_input_and_ambiguous_existing_document() {
    for input in [b"invalid\0value".as_slice(), b"carriage\rreturn", b"\xff"] {
        let f = Fixture::new();
        assert!(!f.run(input, &[]).status.success());
        assert!(!f.target().exists());
        f.assert_no_temp();
    }
    for original in ["TOKEN=one\nTOKEN=two\n", "not-an-assignment\n"] {
        let f = Fixture::new();
        let previous = if original.starts_with("TOKEN") {
            encode(original)
        } else {
            "invalid synthetic ciphertext".to_string()
        };
        fs::write(f.target(), &previous).unwrap();
        assert!(!f.run(CANARY.as_bytes(), &[]).status.success());
        assert_eq!(fs::read_to_string(f.target()).unwrap(), previous);
    }
}

#[test]
fn set_uses_final_entry_rules_for_new_and_existing_entries() {
    for existing in [false, true] {
        let f = Fixture::new();
        if existing {
            fs::write(f.target(), encode("TOKEN=old\n")).unwrap();
        }
        let out = f.run(CANARY.as_bytes(), &[]);
        assert!(out.status.success());
        let cipher = fs::read_to_string(f.target()).unwrap();
        assert!(cipher.contains("sops_age__list_0__map_recipient=synthetic-final-recipient"));
    }
}

#[cfg(unix)]
#[test]
fn set_termination_before_install_preserves_previous_ciphertext() {
    use std::thread;
    use std::time::{Duration, Instant};
    for mode in ["interrupt", "interrupt-validation", "stalled-input"] {
        let f = Fixture::new();
        let previous = encode("TOKEN=old\nOTHER=keep\n");
        fs::write(f.target(), &previous).unwrap();
        let ready = f.root.path().join("ready");
        let mut cmd = f.command();
        cmd.args(["set", "service", "TOKEN", "--value-stdin"])
            .env("SOPS_MODE", mode)
            .env("READY_FILE", &ready);
        let mut child = cmd.spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        if mode != "stalled-input" {
            input.write_all(CANARY.as_bytes()).unwrap();
            drop(input);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        if mode == "stalled-input" {
            // Governance invokes status before the cancellable stdin reader starts.
            while !fs::read_to_string(f.root.path().join("ops"))
                .unwrap_or_default()
                .contains("status --porcelain")
            {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            thread::sleep(Duration::from_millis(100));
        } else {
            while !ready.exists() {
                assert!(Instant::now() < deadline, "SOPS did not reach {mode}");
                thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(
            Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        while child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "interruption did not cancel {mode}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let out = child.wait_with_output().unwrap();
        f.assert_no_leak(&out);
        assert!(!out.status.success());
        assert_eq!(fs::read_to_string(f.target()).unwrap(), previous);
        f.assert_no_temp();
    }
}

#[cfg(unix)]
#[test]
fn set_refuses_symlink_entries_and_parent_paths() {
    use std::os::unix::fs::symlink;
    for parent in [false, true] {
        let f = Fixture::new();
        let outside = f.root.path().join("outside");
        if parent {
            fs::create_dir(&outside).unwrap();
            fs::remove_dir(f.store.join("repos")).unwrap();
            symlink(&outside, f.store.join("repos")).unwrap();
        } else {
            fs::write(&outside, "synthetic unchanged").unwrap();
            symlink(&outside, f.target()).unwrap();
        }
        assert!(!f.run(CANARY.as_bytes(), &[]).status.success());
        assert!(!f.root.path().join("sops-ops").exists());
    }
}

#[test]
fn set_requires_stdin_flag_and_refuses_unsafe_entry_and_reserved_key() {
    let f = Fixture::new();
    for args in [
        vec!["set", "service", "TOKEN"],
        vec!["set", "../outside", "TOKEN", "--value-stdin"],
        vec!["set", "service", "sops_mac", "--value-stdin"],
        vec!["set", "service", "INVALID-KEY", "--value-stdin"],
    ] {
        let out = f.command().args(args).output().unwrap();
        f.assert_no_leak(&out);
        assert!(!out.status.success());
    }
    assert!(!f.target().exists());
    assert!(!f.root.path().join("sops-ops").exists());
}

#[test]
fn set_new_entry_failures_leave_no_target() {
    for mode in ["encrypt-fail", "validate-fail", "plaintext"] {
        let f = Fixture::new();
        assert!(
            !f.run(CANARY.as_bytes(), &[("SOPS_MODE", mode)])
                .status
                .success()
        );
        assert!(!f.target().exists());
        f.assert_no_temp();
    }
}

#[test]
fn set_pending_write_survives_failed_delivery_and_refuses_replay() {
    let f = Fixture::new();
    assert!(f.run(CANARY.as_bytes(), &[]).status.success());
    let pending = fs::read(f.target()).unwrap();
    // The owner follows the explicit handoff; signing/provider tools may fail.
    let failed = Command::new(f.stubs.path().join("semantic-commit"))
        .env("OPS_LOG", f.root.path().join("ops"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let replay = f.run(b"replacement", &[("DIRTY", "?? repos/service.enc.env")]);
    assert!(!replay.status.success());
    assert_eq!(fs::read(f.target()).unwrap(), pending);
}

#[cfg(unix)]
#[test]
fn set_terminal_stdin_is_refused_without_prompting() {
    let f = Fixture::new();
    let template = f.command();
    let mut cmd = Command::new("python3");
    cmd.env_clear();
    cmd.arg("-c").arg(r#"import os,pty,subprocess,sys
master,slave=pty.openpty()
p=subprocess.run([sys.argv[1], 'set', 'service', 'TOKEN', '--value-stdin'], stdin=slave, capture_output=True, timeout=5)
os.close(slave)
os.close(master)
sys.stdout.buffer.write(p.stdout)
sys.stderr.buffer.write(p.stderr)
sys.exit(p.returncode)
"#).arg(bin::resolve("secrets")).current_dir(f.root.path());
    for (key, value) in template.get_envs() {
        if let Some(value) = value {
            cmd.env(key, value);
        }
    }
    let out = cmd.output().unwrap();
    f.assert_no_leak(&out);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("terminal stdin refused"));
    assert!(!f.target().exists());
}

#[test]
fn set_store_rules_cannot_be_replaced_by_recipient_environment() {
    let f = Fixture::new();
    let out = f.run(
        CANARY.as_bytes(),
        &[
            ("SOPS_AGE_RECIPIENTS", "synthetic-environment-override"),
            ("SOPS_CONFIG", "synthetic-other-config"),
        ],
    );
    assert!(out.status.success());
    let cipher = fs::read_to_string(f.target()).unwrap();
    assert!(cipher.contains("sops_age__list_0__map_recipient=synthetic-final-recipient"));
    let ops = fs::read_to_string(f.root.path().join("sops-ops")).unwrap();
    assert!(
        ops.lines()
            .find(|line| line.starts_with("-e "))
            .unwrap()
            .contains("--config")
    );
}

#[test]
fn set_large_utf8_value_crosses_pipe_buffers_without_loss() {
    let f = Fixture::new();
    let input = format!("{}\t{CANARY}", "synthetic-λ".repeat(16_384));
    assert!(f.run(input.as_bytes(), &[]).status.success());
    assert_eq!(
        decode(&fs::read_to_string(f.target()).unwrap()),
        format!("TOKEN=\"{}\"\n", input.replace('\t', "\\t"))
    );
}

#[test]
fn set_value_limit_refuses_oversized_input_without_writing() {
    let f = Fixture::new();
    assert!(!f.run(&vec![b'x'; 1024 * 1024 + 1], &[]).status.success());
    assert!(!f.target().exists());
    f.assert_no_temp();
}

#[test]
fn set_dynamic_completion_offers_entries_and_stdin_flag() {
    let f = Fixture::new();
    fs::write(f.target(), encode("TOKEN=old\n")).unwrap();
    for (index, args, expected) in [
        ("2", vec!["--", "secrets", "set", ""], "repos/service"),
        (
            "4",
            vec!["--", "secrets", "set", "service", "TOKEN", "--"],
            "--value-stdin",
        ),
    ] {
        let out = f
            .command()
            .env("COMPLETE", "zsh")
            .env("_CLAP_COMPLETE_INDEX", index)
            .env("_CLAP_IFS", "\n")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line.split(':').next() == Some(expected))
        );
    }
}

#[test]
fn set_resolves_existing_stack_and_appends_missing_key() {
    let f = Fixture::new();
    fs::create_dir(f.store.join("stacks")).unwrap();
    let target = f.store.join("stacks/service.enc.env");
    fs::write(&target, encode("# retained\nOTHER=keep\n")).unwrap();
    let out = f.run(CANARY.as_bytes(), &[]);
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["data"]["entry"], "stacks/service.enc.env");
    assert_eq!(
        decode(&fs::read_to_string(target).unwrap()),
        format!("# retained\nOTHER=keep\nTOKEN=\"{CANARY}\"\n")
    );
    assert!(!f.target().exists());
}
