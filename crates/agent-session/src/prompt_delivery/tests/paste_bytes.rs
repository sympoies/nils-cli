use super::*;
use nils_test_support::tempdir::ScopedTempDir;
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;

struct Receiver {
    tmux: PathBuf,
    dir: ScopedTempDir,
}

impl Receiver {
    fn start(count: usize) -> Option<Self> {
        let Some(tmux) = binary_on_path("tmux") else {
            eprintln!("skipping paste-byte fixture: tmux is unavailable");
            return None;
        };
        let (dir, socket) = nils_test_support::short_socket_path("tmux.sock");
        let wrapper = dir.path().join("tmux");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexec {} -f /dev/null -S {} \"$@\"\n",
                shell_words::quote(&tmux.to_string_lossy()),
                shell_words::quote(&socket.to_string_lossy())
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let receiver = Self { tmux: wrapper, dir };
        let script = format!(
            "stty raw -echo && printf '\\033[?2004hready' && dd bs=1 count={count} of={} 2>/dev/null; sleep 30",
            shell_words::quote(&receiver.bytes_path().to_string_lossy())
        );
        let status = nils_test_support::cmd::retry_executable_file_busy(|| {
            ProcessCommand::new(&receiver.tmux)
                .args([
                    "new-session",
                    "-d",
                    "-s",
                    "receiver",
                    "--",
                    "sh",
                    "-c",
                    &script,
                ])
                .status()
        })
        .expect("start private tmux server");
        assert!(status.success(), "start receiver pane");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !capture_visible_pane(&receiver.tmux, "receiver:0.0")
            .is_some_and(|pane| pane.contains("ready"))
        {
            assert!(Instant::now() < deadline, "receiver pane was never ready");
            thread::sleep(Duration::from_millis(20));
        }
        Some(receiver)
    }

    fn bytes_path(&self) -> PathBuf {
        self.dir.path().join("received")
    }

    fn paste(&self, prompt: &str) -> Vec<u8> {
        let path = self.dir.path().join("prompt.txt");
        fs::write(&path, prompt).unwrap();
        let record = serde_json::from_value(json!({
            "schema_version": SESSION_DOCUMENT_VERSION,
            "id": "paste-byte-fixture", "agent": "claude", "mode": "interactive",
            "cwd": self.dir.path(), "tmux_session": "receiver", "prompt_file": path,
            "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z"
        }))
        .unwrap();

        crate::paste_prompt(&self.tmux, &record).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(bytes) = fs::read(self.bytes_path())
                && bytes.len() == prompt.len() + 12
            {
                return bytes;
            }
            assert!(
                Instant::now() < deadline,
                "receiver did not record the paste"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let _ = ProcessCommand::new(&self.tmux).arg("kill-server").status();
    }
}

#[test]
fn image_path_and_text_keep_separate_lf_delimited_rows() {
    let prompt = "\"sample image.png\"\n> Describe the image.";
    let expected = format!("\u{1b}[200~{prompt}\u{1b}[201~");
    let Some(receiver) = Receiver::start(expected.len()) else {
        return;
    };
    fs::write(receiver.dir.path().join("sample image.png"), b"fixture").unwrap();
    assert_eq!(receiver.paste(prompt), expected.as_bytes());
}

#[test]
fn text_only_multiline_keeps_cr_separators() {
    let expected = b"\x1b[200~First line\r> Second line\x1b[201~";
    let Some(receiver) = Receiver::start(expected.len()) else {
        return;
    };
    assert_eq!(receiver.paste("First line\n> Second line"), expected);
}
